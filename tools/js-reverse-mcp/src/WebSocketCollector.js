/**
 * @license
 * Copyright 2025 Google LLC
 * SPDX-License-Identifier: Apache-2.0
 */
const stableIdSymbol = Symbol('wsStableIdSymbol');
function createIdGenerator() {
    let i = 1;
    return () => {
        if (i === Number.MAX_SAFE_INTEGER) {
            i = 0;
        }
        return i++;
    };
}
/**
 * Collector for WebSocket connections and messages.
 * Listens to CDP Network events for WebSocket activity.
 */
export class WebSocketCollector {
    #context;
    #sessionProvider;
    /**
     * Storage: Page -> Array of navigations -> Array of WebSocket connections.
     * Newer navigations come first.
     */
    #storage = new WeakMap();
    /**
     * Quick lookup: Page -> requestId -> WebSocketData
     */
    #connectionMap = new WeakMap();
    /**
     * ID generator per page for stable IDs.
     */
    #idGenerators = new WeakMap();
    /**
     * CDP cleanup per page.
     */
    #cdpCleanup = new WeakMap();
    #maxNavigationSaved = 3;
    constructor(context, sessionProvider) {
        this.#context = context;
        this.#sessionProvider = sessionProvider;
    }
    async init() {
        const pages = this.#context.pages();
        for (const page of pages) {
            await this.addPage(page);
        }
        this.#context.on('page', this.#onPageCreated);
    }
    dispose() {
        this.#context.off('page', this.#onPageCreated);
    }
    #onPageCreated = async (page) => {
        await this.addPage(page);
        page.on('close', () => {
            this.#cleanupPage(page);
        });
    };
    async addPage(page) {
        if (this.#storage.has(page)) {
            return;
        }
        const idGenerator = createIdGenerator();
        this.#idGenerators.set(page, idGenerator);
        const storedLists = [[]];
        this.#storage.set(page, storedLists);
        this.#connectionMap.set(page, new Map());
        await this.#setupCdpListeners(page);
    }
    async #setupCdpListeners(page) {
        try {
            const client = await this.#sessionProvider.getSession(page);
            const connectionMap = this.#connectionMap.get(page);
            const idGenerator = this.#idGenerators.get(page);
            const onCreated = (event) => {
                const wsData = {
                    connection: {
                        requestId: event.requestId,
                        url: event.url,
                        initiator: event.initiator,
                        status: 'connecting',
                        createdAt: Date.now(),
                    },
                    frames: [],
                };
                wsData[stableIdSymbol] = idGenerator();
                connectionMap.set(event.requestId, wsData);
                const navigations = this.#storage.get(page);
                if (navigations) {
                    navigations[0].push(wsData);
                }
                // Mark as open once created (CDP doesn't have a separate open event for ws)
                wsData.connection.status = 'open';
            };
            const onFrameSent = (event) => {
                const wsData = connectionMap.get(event.requestId);
                if (!wsData) {
                    return;
                }
                wsData.frames.push({
                    requestId: event.requestId,
                    direction: 'sent',
                    timestamp: event.timestamp * 1000, // Convert to ms
                    opcode: event.response.opcode,
                    payloadData: event.response.payloadData,
                });
            };
            const onFrameReceived = (event) => {
                const wsData = connectionMap.get(event.requestId);
                if (!wsData) {
                    return;
                }
                wsData.frames.push({
                    requestId: event.requestId,
                    direction: 'received',
                    timestamp: event.timestamp * 1000, // Convert to ms
                    opcode: event.response.opcode,
                    payloadData: event.response.payloadData,
                });
            };
            const onClosed = (event) => {
                const wsData = connectionMap.get(event.requestId);
                if (!wsData) {
                    return;
                }
                wsData.connection.status = 'closed';
                wsData.connection.closedAt = event.timestamp * 1000;
            };
            const onFrameNavigated = () => {
                this.#splitAfterNavigation(page);
            };
            client.on('Network.webSocketCreated', onCreated);
            client.on('Network.webSocketFrameSent', onFrameSent);
            client.on('Network.webSocketFrameReceived', onFrameReceived);
            client.on('Network.webSocketClosed', onClosed);
            page.on('framenavigated', frame => {
                if (frame === page.mainFrame()) {
                    onFrameNavigated();
                }
            });
            this.#cdpCleanup.set(page, () => {
                client.off('Network.webSocketCreated', onCreated);
                client.off('Network.webSocketFrameSent', onFrameSent);
                client.off('Network.webSocketFrameReceived', onFrameReceived);
                client.off('Network.webSocketClosed', onClosed);
            });
        }
        catch {
            // Page might already be closed
        }
    }
    #splitAfterNavigation(page) {
        const navigations = this.#storage.get(page);
        if (!navigations) {
            return;
        }
        // Add a new navigation
        navigations.unshift([]);
        navigations.splice(this.#maxNavigationSaved);
        // Reset connection map for new navigation
        this.#connectionMap.set(page, new Map());
    }
    #cleanupPage(page) {
        const cleanup = this.#cdpCleanup.get(page);
        if (cleanup) {
            try {
                cleanup();
            }
            catch {
                // Page might already be closed
            }
        }
        this.#cdpCleanup.delete(page);
        this.#storage.delete(page);
        this.#connectionMap.delete(page);
        this.#idGenerators.delete(page);
    }
    /**
     * Get all WebSocket connections for a page.
     */
    getData(page, includePreservedData) {
        const navigations = this.#storage.get(page);
        if (!navigations) {
            return [];
        }
        if (!includePreservedData) {
            return navigations[0] ?? [];
        }
        const data = [];
        for (let index = this.#maxNavigationSaved; index >= 0; index--) {
            if (navigations[index]) {
                data.push(...navigations[index]);
            }
        }
        return data;
    }
    /**
     * Get stable ID for a WebSocket connection.
     */
    getIdForResource(resource) {
        return resource[stableIdSymbol] ?? -1;
    }
    /**
     * Get WebSocket connection by stable ID.
     */
    getById(page, stableId) {
        const navigations = this.#storage.get(page);
        if (!navigations) {
            throw new Error('No WebSocket connections found for selected page');
        }
        for (const navigation of navigations) {
            const item = navigation.find(ws => ws[stableIdSymbol] === stableId);
            if (item) {
                return item;
            }
        }
        throw new Error('WebSocket connection not found for selected page');
    }
    /**
     * Find a WebSocket connection matching the filter.
     */
    find(page, filter) {
        const navigations = this.#storage.get(page);
        if (!navigations) {
            return undefined;
        }
        for (const navigation of navigations) {
            const item = navigation.find(filter);
            if (item) {
                return item;
            }
        }
        return undefined;
    }
}
