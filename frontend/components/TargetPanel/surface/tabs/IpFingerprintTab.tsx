import type { Fingerprint } from "@/lib/security-analysis";
import { Section } from "../SurfaceParts";
import type { WebOriginVM } from "../surfaceHierarchy";
import { FingerprintList } from "./FingerprintList";

export function IpFingerprintTab({
  fingerprints,
  webOrigins,
  loading = false,
}: {
  fingerprints: Fingerprint[];
  webOrigins: WebOriginVM[];
  loading?: boolean;
}) {
  const attributedIds = new Set(
    webOrigins.flatMap((origin) => origin.fingerprints.map((fingerprint) => fingerprint.id))
  );
  const attributed = fingerprints.filter((fingerprint) => attributedIds.has(fingerprint.id));
  const unassigned = fingerprints.filter((fingerprint) => !attributedIds.has(fingerprint.id));

  return (
    <div className="space-y-2.5">
      <Section
        title="Fingerprint attribution"
        subtitle={`${fingerprints.length} target-level row(s)`}
      >
        <p className="text-[10px] leading-relaxed text-muted-foreground">
          This aggregate includes fingerprints loaded for the selected IP and its related targets.
          Only evidence with an explicit matching Web Origin is attributed below; legacy evidence
          without an origin stays target-level and unassigned.
        </p>
      </Section>
      <FingerprintList
        title="Web Origin fingerprints"
        fingerprints={attributed}
        loading={loading}
        emptyLabel="No fingerprint carries explicit evidence for a displayed Web Origin."
      />
      <FingerprintList
        title="Target-level / unassigned fingerprints"
        fingerprints={unassigned}
        loading={loading}
        emptyLabel="No target-level fingerprint is waiting for exact Web Origin attribution."
      />
    </div>
  );
}
