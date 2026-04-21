/**
 * @license
 * Copyright 2025 Google LLC
 * SPDX-License-Identifier: Apache-2.0
 */
import { createIssuesFromProtocolIssue, IssueAggregator, } from '../node_modules/chrome-devtools-frontend/mcp/mcp.js';
import { FakeIssuesManager } from './DevtoolsUtils.js';
import { features } from './features.js';
import { logger } from './logger.js';
function createIdGenerator() {
    let i = 1;
    return () => {
        if (i === Number.MAX_SAFE_INTEGER) {
            i = 0;
        }
        return i++;
    };
}
export const stableIdSymbol = Symbol('stableIdSymbol');
export class PageCollector {
    #context;
    #listenersInitializer;
    #listeners = new WeakMap();
    #maxNavigationSaved = 3;
    #maxItemsPerNavigation = 1000;
    /**
     * This maps a Page to a list of navigations with a sub-list
     * of all collected resources.
     * The newer navigations come first.
     */
    storage = new WeakMap();
    constructor(context, listeners) {
        this.#context = context;
        this.#listenersInitializer = listeners;
    }
    get context() {
        return this.#context;
    }
    async init() {
        const pages = this.#context.pages();
        for (const page of pages) {
            this.addPage(page);
        }
        this.#context.on('page', this.#onPageCreated);
    }
    dispose() {
        this.#context.off('page', this.#onPageCreated);
    }
    #onPageCreated = (page) => {
        this.addPage(page);
        page.on('close', () => {
            this.cleanupPageDestroyed(page);
        });
    };
    addPage(page) {
        this.#initializePage(page);
    }
    #initializePage(page) {
        if (this.storage.has(page)) {
            return;
        }
        const idGenerator = createIdGenerator();
        const storedLists = [[]];
        this.storage.set(page, storedLists);
        const listeners = this.#listenersInitializer(value => {
            const withId = value;
            withId[stableIdSymbol] = idGenerator();
            const navigations = this.storage.get(page) ?? [[]];
            navigations[0].push(withId);
            if (navigations[0].length > this.#maxItemsPerNavigation) {
                navigations[0].shift();
            }
        });
        listeners['framenavigated'] = (frame) => {
            // Only split the storage on main frame navigation
            if (frame !== page.mainFrame()) {
                return;
            }
            this.splitAfterNavigation(page);
        };
        for (const [name, listener] of Object.entries(listeners)) {
            page.on(name, listener);
        }
        this.#listeners.set(page, listeners);
    }
    splitAfterNavigation(page) {
        const navigations = this.storage.get(page);
        if (!navigations) {
            return;
        }
        // Add the latest navigation first
        navigations.unshift([]);
        navigations.splice(this.#maxNavigationSaved);
    }
    cleanupPageDestroyed(page) {
        const listeners = this.#listeners.get(page);
        if (listeners) {
            for (const [name, listener] of Object.entries(listeners)) {
                page.off(name, listener);
            }
        }
        this.storage.delete(page);
    }
    getData(page, includePreservedData) {
        const navigations = this.storage.get(page);
        if (!navigations) {
            return [];
        }
        if (!includePreservedData) {
            return navigations[0];
        }
        const data = [];
        for (let index = this.#maxNavigationSaved; index >= 0; index--) {
            if (navigations[index]) {
                data.push(...navigations[index]);
            }
        }
        return data;
    }
    getIdForResource(resource) {
        return resource[stableIdSymbol] ?? -1;
    }
    getById(page, stableId) {
        const navigations = this.storage.get(page);
        if (!navigations) {
            throw new Error('No requests found for selected page');
        }
        const item = this.find(page, item => item[stableIdSymbol] === stableId);
        if (item) {
            return item;
        }
        throw new Error('Request not found for selected page');
    }
    find(page, filter) {
        const navigations = this.storage.get(page);
        if (!navigations) {
            return;
        }
        for (const navigation of navigations) {
            const item = navigation.find(filter);
            if (item) {
                return item;
            }
        }
        return;
    }
}
export class ConsoleCollector extends PageCollector {
    #subscribedPages = new WeakMap();
    #sessionProvider;
    // Per-page issue collectors that feed into the PageCollector's storage
    #pageIssueCollectors = new WeakMap();
    #cdpReady = false;
    constructor(context, sessionProvider, listeners) {
        // Wrap the original listener initializer to capture per-page collectors
        const wrappedListeners = (collector) => {
            // Call the original to get the base listeners
            const baseListeners = listeners(collector);
            // The 'issue' key in baseListeners calls collector(event)
            // We'll also use this collector reference for PageIssueSubscriber
            return baseListeners;
        };
        super(context, wrappedListeners);
        this.#sessionProvider = sessionProvider;
    }
    addPage(page) {
        super.addPage(page);
        // Only set up CDP issue subscriber if CDP has been initialized
        if (this.#cdpReady) {
            this.#setupIssueSubscriber(page);
        }
    }
    /**
     * Initialize CDP-dependent features (Audits.enable for issue collection).
     * Called lazily to avoid leaking CDP signals during navigation.
     */
    async initCdp() {
        if (this.#cdpReady)
            return;
        this.#cdpReady = true;
        // Set up issue subscribers for all already-tracked pages
        for (const page of this.context.pages()) {
            if (this.storage.has(page)) {
                this.#setupIssueSubscriber(page);
            }
        }
    }
    #setupIssueSubscriber(page) {
        if (!features.issues) {
            return;
        }
        if (!this.#subscribedPages.has(page)) {
            // Create a direct collector that adds issues to this page's storage with stable IDs
            const idGen = createIdGenerator();
            const issueCollector = (issue) => {
                const navigations = this.storage.get(page);
                if (navigations && navigations[0]) {
                    const withId = issue;
                    withId[stableIdSymbol] = idGen();
                    navigations[0].push(withId);
                }
            };
            this.#pageIssueCollectors.set(page, issueCollector);
            const subscriber = new PageIssueSubscriber(page, this.#sessionProvider, issueCollector);
            this.#subscribedPages.set(page, subscriber);
            void subscriber.subscribe();
        }
    }
    cleanupPageDestroyed(page) {
        super.cleanupPageDestroyed(page);
        this.#subscribedPages.get(page)?.unsubscribe();
        this.#subscribedPages.delete(page);
    }
}
class PageIssueSubscriber {
    #issueManager = new FakeIssuesManager();
    #issueAggregator = new IssueAggregator(this.#issueManager);
    #seenKeys = new Set();
    #seenIssues = new Set();
    #page;
    #sessionProvider;
    #session = null;
    #onIssueCallback;
    constructor(page, sessionProvider, onIssue) {
        this.#page = page;
        this.#sessionProvider = sessionProvider;
        this.#onIssueCallback = onIssue;
    }
    #resetIssueAggregator() {
        this.#issueManager = new FakeIssuesManager();
        if (this.#issueAggregator) {
            this.#issueAggregator.removeEventListener("AggregatedIssueUpdated" /* IssueAggregatorEvents.AGGREGATED_ISSUE_UPDATED */, this.#onAggregatedissue);
        }
        this.#issueAggregator = new IssueAggregator(this.#issueManager);
        this.#issueAggregator.addEventListener("AggregatedIssueUpdated" /* IssueAggregatorEvents.AGGREGATED_ISSUE_UPDATED */, this.#onAggregatedissue);
    }
    async subscribe() {
        this.#resetIssueAggregator();
        this.#page.on('framenavigated', this.#onFrameNavigated);
        try {
            this.#session = await this.#sessionProvider.getSession(this.#page);
            this.#session.on('Audits.issueAdded', this.#onIssueAdded);
            await this.#session.send('Audits.enable');
        }
        catch (error) {
            logger('Error subscribing to issues', error);
        }
    }
    unsubscribe() {
        this.#seenKeys.clear();
        this.#seenIssues.clear();
        this.#page.off('framenavigated', this.#onFrameNavigated);
        if (this.#session) {
            this.#session.off('Audits.issueAdded', this.#onIssueAdded);
        }
        if (this.#issueAggregator) {
            this.#issueAggregator.removeEventListener("AggregatedIssueUpdated" /* IssueAggregatorEvents.AGGREGATED_ISSUE_UPDATED */, this.#onAggregatedissue);
        }
        if (this.#session) {
            void this.#session.send('Audits.disable').catch(() => {
                // might fail.
            });
        }
    }
    #onAggregatedissue = (event) => {
        if (this.#seenIssues.has(event.data)) {
            return;
        }
        this.#seenIssues.add(event.data);
        this.#onIssueCallback(event.data);
    };
    // On navigation, we reset issue aggregation.
    #onFrameNavigated = (frame) => {
        // Only split the storage on main frame navigation
        if (frame !== frame.page().mainFrame()) {
            return;
        }
        this.#seenKeys.clear();
        this.#seenIssues.clear();
        this.#resetIssueAggregator();
    };
    #onIssueAdded = (data) => {
        try {
            const inspectorIssue = data.issue;
            // @ts-expect-error Types of protocol from Playwright and CDP are
            // incomparable for InspectorIssueCode, one is union, other is enum.
            const issue = createIssuesFromProtocolIssue(null, inspectorIssue)[0];
            if (!issue) {
                logger('No issue mapping for for the issue: ', inspectorIssue.code);
                return;
            }
            const primaryKey = issue.primaryKey();
            if (this.#seenKeys.has(primaryKey)) {
                return;
            }
            this.#seenKeys.add(primaryKey);
            this.#issueManager.dispatchEventToListeners("IssueAdded" /* IssuesManagerEvents.ISSUE_ADDED */, {
                issue,
                // @ts-expect-error We don't care that issues model is null
                issuesModel: null,
            });
        }
        catch (error) {
            logger('Error creating a new issue', error);
        }
    };
}
const cdpRequestIdSymbol = Symbol('cdpRequestId');
export class NetworkCollector extends PageCollector {
    #initiators = new WeakMap();
    #cdpListeners = new WeakMap();
    #sessionProvider;
    #cdpReady = false;
    constructor(context, sessionProvider, listeners) {
        super(context, listeners ??
            (collect => {
                return {
                    request: req => {
                        collect(req);
                    },
                };
            }));
        this.#sessionProvider = sessionProvider;
    }
    addPage(page) {
        super.addPage(page);
        // Only set up CDP initiator collection if CDP has been initialized
        if (this.#cdpReady) {
            void this.#setupInitiatorCollection(page);
        }
    }
    /**
     * Initialize CDP-dependent features (initiator collection).
     * Called lazily to avoid leaking CDP signals during navigation.
     */
    async initCdp() {
        if (this.#cdpReady)
            return;
        this.#cdpReady = true;
        // Set up CDP initiator collection for all already-tracked pages
        for (const page of this.context.pages()) {
            if (this.storage.has(page)) {
                void this.#setupInitiatorCollection(page);
            }
        }
    }
    async #setupInitiatorCollection(page) {
        if (this.#initiators.has(page)) {
            return;
        }
        const initiatorMap = new Map();
        this.#initiators.set(page, initiatorMap);
        try {
            const client = await this.#sessionProvider.getSession(page);
            // Listen to CDP events for initiator info and request ID mapping
            const onRequestWillBeSent = (event) => {
                if (event.initiator) {
                    initiatorMap.set(event.requestId, event.initiator);
                }
                // Map CDP request ID to Playwright Request via URL+method matching
                // This allows us to correlate Playwright Request objects with CDP request IDs
                const navigations = this.storage.get(page);
                if (navigations) {
                    for (const navigation of navigations) {
                        for (const request of navigation) {
                            const req = request;
                            if (!req[cdpRequestIdSymbol] &&
                                req.url() === event.request.url &&
                                req.method() === event.request.method) {
                                req[cdpRequestIdSymbol] = event.requestId;
                                break;
                            }
                        }
                    }
                }
            };
            client.on('Network.requestWillBeSent', onRequestWillBeSent);
            const cleanup = () => {
                client.off('Network.requestWillBeSent', onRequestWillBeSent);
            };
            this.#cdpListeners.set(page, cleanup);
        }
        catch {
            // Page might already be closed
        }
    }
    cleanupPageDestroyed(page) {
        super.cleanupPageDestroyed(page);
        const cleanup = this.#cdpListeners.get(page);
        if (cleanup) {
            try {
                cleanup();
            }
            catch {
                // Page might already be closed
            }
        }
        this.#cdpListeners.delete(page);
        this.#initiators.delete(page);
    }
    /**
     * Get the CDP request ID for a request.
     */
    getCdpRequestId(request) {
        return request[cdpRequestIdSymbol];
    }
    /**
     * Get the initiator info for a request.
     * @param page The page the request belongs to
     * @param request The HTTP request
     * @returns The initiator info or undefined if not found
     */
    getInitiator(page, request) {
        const initiatorMap = this.#initiators.get(page);
        if (!initiatorMap) {
            return undefined;
        }
        const requestId = this.getCdpRequestId(request);
        if (!requestId) {
            return undefined;
        }
        return initiatorMap.get(requestId);
    }
    /**
     * Get initiator by CDP request ID.
     */
    getInitiatorByRequestId(page, requestId) {
        const initiatorMap = this.#initiators.get(page);
        return initiatorMap?.get(requestId);
    }
    splitAfterNavigation(page) {
        const navigations = this.storage.get(page) ?? [];
        if (!navigations) {
            return;
        }
        const requests = navigations[0];
        const lastRequestIdx = requests.findLastIndex(request => {
            try {
                return request.frame() === page.mainFrame()
                    ? request.isNavigationRequest()
                    : false;
            }
            catch {
                // frame() can throw for service worker requests
                return false;
            }
        });
        // Keep all requests since the last navigation request including that
        // navigation request itself.
        // Keep the reference
        if (lastRequestIdx !== -1) {
            const fromCurrentNavigation = requests.splice(lastRequestIdx);
            navigations.unshift(fromCurrentNavigation);
        }
        else {
            navigations.unshift([]);
        }
        // Clear old initiator data on navigation
        const initiatorMap = this.#initiators.get(page);
        if (initiatorMap) {
            initiatorMap.clear();
        }
    }
}
