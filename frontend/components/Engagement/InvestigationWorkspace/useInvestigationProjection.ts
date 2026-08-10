import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  InvestigationCampaignDetailResponse,
  InvestigationCampaignPageResponse,
  InvestigationHypothesisDetailView,
  InvestigationHypothesisListView,
  InvestigationSummaryView,
  InvestigationTimelinePageResponse,
} from "@/lib/api/investigation";
import type { InvestigationProjectionEnvelope } from "@/lib/generated/InvestigationProjectionEnvelope";

const PROJECTION_SCHEMA_VERSION = 1;
const PAGE_SIZE = 100;
const STALE_CODE = "INVESTIGATION_PROJECTION_STALE";

export interface ProjectionStamp {
  projectionSchemaVersion: 1;
  changeSeq: number;
  authorityEpochSetHash: string;
  observedAsOf: string;
}

export interface ProjectionResource<T> {
  data?: T;
  stamp?: ProjectionStamp;
  status: "idle" | "loading" | "ready" | "error" | "stale";
  errorCode?: string;
  errorMessage?: string;
  nextCursor?: string | null;
}

export interface InvestigationWorkspaceApi {
  getSummary: (request: {
    sessionId: string;
    operationId: string;
  }) => Promise<InvestigationSummaryView>;
  listHypotheses: (request: Record<string, unknown>) => Promise<InvestigationHypothesisListView>;
  getHypothesis: (request: {
    sessionId: string;
    operationId: string;
    revisionId: string;
  }) => Promise<InvestigationHypothesisDetailView>;
  listCampaigns: (request: Record<string, unknown>) => Promise<InvestigationCampaignPageResponse>;
  getCampaign: (request: {
    sessionId: string;
    operationId: string;
    campaignId: string;
  }) => Promise<InvestigationCampaignDetailResponse>;
  listTimeline: (request: Record<string, unknown>) => Promise<InvestigationTimelinePageResponse>;
}

function legacyUnavailable(): Promise<never> {
  return Promise.reject(
    new Error("Legacy Investigation workspace cannot call the unified exact-stage read API.")
  );
}

const defaultApi: InvestigationWorkspaceApi = {
  getSummary: legacyUnavailable,
  listHypotheses: legacyUnavailable,
  getHypothesis: legacyUnavailable,
  listCampaigns: legacyUnavailable,
  getCampaign: legacyUnavailable,
  listTimeline: legacyUnavailable,
};

interface ProjectionErrorShape {
  code?: unknown;
  message?: unknown;
}

function projectionError(error: unknown): { code: string; message: string } {
  const shaped = error as ProjectionErrorShape;
  return {
    code: typeof shaped?.code === "string" ? shaped.code : "INVESTIGATION_READ_FAILED",
    message:
      typeof shaped?.message === "string"
        ? shaped.message
        : error instanceof Error
          ? error.message
          : String(error),
  };
}

export function projectionStamp(envelope: InvestigationProjectionEnvelope): ProjectionStamp {
  if (envelope.projectionSchemaVersion !== PROJECTION_SCHEMA_VERSION) {
    throw new Error(
      `INVESTIGATION_PROJECTION_SCHEMA_UNSUPPORTED:${envelope.projectionSchemaVersion}`
    );
  }
  return {
    projectionSchemaVersion: PROJECTION_SCHEMA_VERSION,
    changeSeq: envelope.changeSeq,
    authorityEpochSetHash: envelope.temporalSnapshot.authorityEpochSetHash,
    observedAsOf: envelope.readAt,
  };
}

/**
 * Accept only server-orderable projection snapshots. Browser time never
 * expires, refreshes or advances authority.
 */
export function acceptsProjection(
  current: ProjectionStamp | undefined,
  incoming: ProjectionStamp
): boolean {
  if (!current) return true;
  if (incoming.projectionSchemaVersion !== current.projectionSchemaVersion) return false;
  if (incoming.changeSeq !== current.changeSeq) return incoming.changeSeq > current.changeSeq;
  if (incoming.observedAsOf !== current.observedAsOf) {
    return incoming.observedAsOf > current.observedAsOf;
  }
  return incoming.authorityEpochSetHash === current.authorityEpochSetHash;
}

function staleResource<T>(
  current: ProjectionResource<T>,
  code: string,
  errorMessage: string
): ProjectionResource<T> {
  return {
    ...current,
    status: current.data === undefined ? "error" : "stale",
    errorCode: code,
    errorMessage,
  };
}

function useSummaryResource({
  api,
  sessionId,
  operationId,
  refreshSeq,
}: {
  api: InvestigationWorkspaceApi;
  sessionId: string;
  operationId: string;
  refreshSeq: number;
}) {
  const [resource, setResource] = useState<ProjectionResource<InvestigationSummaryView>>({
    status: "idle",
  });
  const generation = useRef(0);

  const reload = useCallback(async () => {
    const requestGeneration = ++generation.current;
    setResource((current) => ({ ...current, status: "loading", errorCode: undefined }));
    try {
      const data = await api.getSummary({ sessionId, operationId });
      const stamp = projectionStamp(data.envelope);
      if (generation.current !== requestGeneration) return;
      setResource((current) =>
        acceptsProjection(current.stamp, stamp)
          ? { data, stamp, status: "ready", nextCursor: data.envelope.nextCursor }
          : current
      );
    } catch (error) {
      if (generation.current !== requestGeneration) return;
      const failure = projectionError(error);
      setResource((current) => staleResource(current, failure.code, failure.message));
    }
  }, [api, operationId, sessionId]);

  useEffect(() => {
    void refreshSeq;
    void reload();
    return () => {
      generation.current += 1;
    };
  }, [refreshSeq, reload]);

  return { resource, reload };
}

interface PagedProjectionConfig<TPage, TItem> {
  refreshSeq: number;
  requestPage: (cursor: string | null, expectedChangeSeq: number | null) => Promise<TPage>;
  envelope: (page: TPage) => InvestigationProjectionEnvelope;
  items: (page: TPage) => TItem[];
  replaceItems: (page: TPage, items: TItem[]) => TPage;
  itemKey: (item: TItem) => string;
}

function usePagedProjection<TPage, TItem>(config: PagedProjectionConfig<TPage, TItem>) {
  const [resource, setResource] = useState<ProjectionResource<TPage>>({ status: "idle" });
  const resourceRef = useRef(resource);
  resourceRef.current = resource;
  const generation = useRef(0);
  const inFlightCursor = useRef<string | null | undefined>(undefined);

  const load = useCallback(
    async (append: boolean, restartedAfterStale = false) => {
      const snapshot = resourceRef.current;
      const cursor = append ? (snapshot.nextCursor ?? null) : null;
      if (append && !cursor) return;
      if (inFlightCursor.current === cursor) return;
      inFlightCursor.current = cursor;
      const requestGeneration = ++generation.current;
      setResource((current) => {
        const next = {
          ...current,
          status: current.data === undefined ? ("loading" as const) : current.status,
          errorCode: undefined,
        };
        resourceRef.current = next;
        return next;
      });
      try {
        const page = await config.requestPage(
          cursor,
          append ? (snapshot.stamp?.changeSeq ?? null) : null
        );
        const envelope = config.envelope(page);
        const stamp = projectionStamp(envelope);
        if (generation.current !== requestGeneration) return;
        setResource((current) => {
          if (!acceptsProjection(current.stamp, stamp)) return current;
          let next: ProjectionResource<TPage>;
          if (!append || !current.data || current.stamp?.changeSeq !== stamp.changeSeq) {
            next = {
              data: page,
              stamp,
              status: "ready",
              nextCursor: envelope.nextCursor,
            };
          } else {
            const seen = new Set(config.items(current.data).map(config.itemKey));
            const appended = config.items(page).filter((item) => !seen.has(config.itemKey(item)));
            next = {
              data: config.replaceItems(page, [...config.items(current.data), ...appended]),
              stamp,
              status: "ready",
              nextCursor: envelope.nextCursor,
            };
          }
          resourceRef.current = next;
          return next;
        });
      } catch (error) {
        if (generation.current !== requestGeneration) return;
        const failure = projectionError(error);
        setResource((current) => {
          const next = staleResource(current, failure.code, failure.message);
          resourceRef.current = next;
          return next;
        });
        if (failure.code === STALE_CODE && !restartedAfterStale) {
          inFlightCursor.current = undefined;
          await load(false, true);
        }
      } finally {
        if (generation.current === requestGeneration) inFlightCursor.current = undefined;
      }
    },
    [config]
  );

  useEffect(() => {
    void config.refreshSeq;
    void load(false);
    return () => {
      generation.current += 1;
      inFlightCursor.current = undefined;
    };
  }, [config.refreshSeq, load]);

  return {
    resource,
    reload: useCallback(() => load(false), [load]),
    loadMore: useCallback(() => load(true), [load]),
  };
}

function useLazyDetail<T>({
  identity,
  load,
}: {
  identity: string | null;
  load: (identity: string) => Promise<T>;
}) {
  const [resource, setResource] = useState<ProjectionResource<T>>({ status: "idle" });
  const generation = useRef(0);

  const reload = useCallback(async () => {
    if (!identity) {
      setResource({ status: "idle" });
      return;
    }
    const requestGeneration = ++generation.current;
    setResource((current) => ({ ...current, status: "loading" }));
    try {
      const data = await load(identity);
      if (generation.current !== requestGeneration) return;
      const envelope = (data as { envelope: InvestigationProjectionEnvelope }).envelope;
      const stamp = projectionStamp(envelope);
      setResource((current) =>
        acceptsProjection(current.stamp, stamp) ? { data, stamp, status: "ready" } : current
      );
    } catch (error) {
      if (generation.current !== requestGeneration) return;
      const failure = projectionError(error);
      setResource((current) => staleResource(current, failure.code, failure.message));
    }
  }, [identity, load]);

  useEffect(() => {
    void reload();
    return () => {
      generation.current += 1;
    };
  }, [reload]);

  return { resource, reload };
}

export function useInvestigationProjection({
  sessionId,
  operationId,
  refreshSeq,
  selectedHypothesisId,
  selectedCampaignId,
  api = defaultApi,
}: {
  sessionId: string;
  operationId: string;
  refreshSeq: number;
  selectedHypothesisId: string | null;
  selectedCampaignId: string | null;
  api?: InvestigationWorkspaceApi;
}) {
  const summary = useSummaryResource({ api, sessionId, operationId, refreshSeq });

  const hypothesisConfig = useMemo<
    PagedProjectionConfig<
      InvestigationHypothesisListView,
      InvestigationHypothesisListView["hypotheses"][number]
    >
  >(
    () => ({
      refreshSeq,
      requestPage: (cursor, expectedChangeSeq) =>
        api.listHypotheses({
          sessionId,
          operationId,
          organizationIds: [],
          epistemicStates: [],
          readinessStates: [],
          capabilityStates: [],
          sourceKinds: [],
          cursor,
          expectedChangeSeq,
          pageSize: PAGE_SIZE,
        }),
      envelope: (page) => page.envelope,
      items: (page) => page.hypotheses,
      replaceItems: (page, hypotheses) => ({ ...page, hypotheses }),
      itemKey: (item) => item.revisionId,
    }),
    [api, operationId, refreshSeq, sessionId]
  );
  const hypotheses = usePagedProjection(hypothesisConfig);

  const campaignConfig = useMemo<
    PagedProjectionConfig<
      InvestigationCampaignPageResponse,
      InvestigationCampaignPageResponse["campaigns"][number]
    >
  >(
    () => ({
      refreshSeq,
      requestPage: (cursor, expectedChangeSeq) =>
        api.listCampaigns({
          sessionId,
          operationId,
          waveIds: [],
          campaignStates: [],
          cursor,
          expectedChangeSeq,
          pageSize: PAGE_SIZE,
        }),
      envelope: (page) => page.envelope,
      items: (page) => page.campaigns,
      replaceItems: (page, campaigns) => ({ ...page, campaigns }),
      itemKey: (item) => item.campaignId,
    }),
    [api, operationId, refreshSeq]
  );
  const campaigns = usePagedProjection(campaignConfig);

  const timelineConfig = useMemo<
    PagedProjectionConfig<
      InvestigationTimelinePageResponse,
      InvestigationTimelinePageResponse["events"][number]
    >
  >(
    () => ({
      refreshSeq,
      requestPage: (cursor, expectedChangeSeq) =>
        api.listTimeline({
          sessionId,
          operationId,
          eventKinds: [],
          cursor,
          expectedChangeSeq,
          pageSize: PAGE_SIZE,
        }),
      envelope: (page) => page.envelope,
      items: (page) => page.events,
      replaceItems: (page, events) => ({ ...page, events }),
      itemKey: (item) => item.eventId,
    }),
    [api, operationId, refreshSeq]
  );
  const timeline = usePagedProjection(timelineConfig);

  const loadHypothesis = useCallback(
    (revisionId: string) => api.getHypothesis({ sessionId, operationId, revisionId }),
    [api, operationId, sessionId]
  );
  const hypothesisDetail = useLazyDetail<InvestigationHypothesisDetailView>({
    identity: selectedHypothesisId,
    load: loadHypothesis,
  });

  const loadCampaign = useCallback(
    (campaignId: string) =>
      api.getCampaign({
        sessionId,
        operationId,
        campaignId,
      }),
    [api, operationId, sessionId]
  );
  const campaignDetail = useLazyDetail<InvestigationCampaignDetailResponse>({
    identity: selectedCampaignId,
    load: loadCampaign,
  });

  const refreshAll = useCallback(() => {
    void summary.reload();
    void hypotheses.reload();
    void campaigns.reload();
    void timeline.reload();
    if (selectedHypothesisId) void hypothesisDetail.reload();
    if (selectedCampaignId) void campaignDetail.reload();
  }, [
    campaignDetail,
    campaigns,
    hypothesisDetail,
    hypotheses,
    selectedCampaignId,
    selectedHypothesisId,
    summary,
    timeline,
  ]);

  return {
    summary: summary.resource,
    hypotheses: hypotheses.resource,
    campaigns: campaigns.resource,
    timeline: timeline.resource,
    hypothesisDetail: hypothesisDetail.resource,
    campaignDetail: campaignDetail.resource,
    loadMoreHypotheses: hypotheses.loadMore,
    loadMoreCampaigns: campaigns.loadMore,
    loadMoreTimeline: timeline.loadMore,
    refreshAll,
  };
}
