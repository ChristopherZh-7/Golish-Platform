/**
 * @license
 * Copyright 2025 Google LLC
 * SPDX-License-Identifier: Apache-2.0
 */
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { CdpSessionProvider } from './CdpSessionProvider.js';
import { DebuggerContext } from './DebuggerContext.js';
import { extractUrlLikeFromDevToolsTitle, urlsEqual } from './DevtoolsUtils.js';
import { NetworkCollector, ConsoleCollector } from './PageCollector.js';
import { selectPage } from './tools/pages.js';
import { CLOSE_PAGE_ERROR } from './tools/ToolDefinition.js';
import { WaitForHelper } from './WaitForHelper.js';
import { WebSocketCollector } from './WebSocketCollector.js';
const DEFAULT_TIMEOUT = 5_000;
const NAVIGATION_TIMEOUT = 10_000;
function getNetworkMultiplierFromString(condition) {
    switch (condition) {
        case 'Fast 4G':
            return 1;
        case 'Slow 4G':
            return 2.5;
        case 'Fast 3G':
            return 5;
        case 'Slow 3G':
            return 10;
    }
    return 1;
}
function getExtensionFromMimeType(mimeType) {
    switch (mimeType) {
        case 'image/png':
            return 'png';
        case 'image/jpeg':
            return 'jpeg';
        case 'image/webp':
            return 'webp';
    }
    throw new Error(`No mapping for Mime type ${mimeType}.`);
}
export class McpContext {
    browserContext;
    sessionProvider;
    logger;
    // The most recent page state.
    #pages = [];
    #pageToDevToolsPage = new Map();
    #selectedPage;
    #networkCollector;
    #consoleCollector;
    #webSocketCollector;
    #isRunningTrace = false;
    #networkConditionsMap = new WeakMap();
    #cpuThrottlingRateMap = new WeakMap();
    #dialog;
    #debuggerContext = new DebuggerContext();
    #selectedFrame;
    #traceResults = [];
    #trafficSummaryCache = new Map();
    #injectedScriptsByPage = new WeakMap();
    #navigationTimeout = NAVIGATION_TIMEOUT;
    #options;
    constructor(browserContext, logger, options) {
        this.browserContext = browserContext;
        this.sessionProvider = new CdpSessionProvider(browserContext);
        this.logger = logger;
        this.#options = options;
        this.#networkCollector = new NetworkCollector(this.browserContext, this.sessionProvider, undefined);
        this.#consoleCollector = new ConsoleCollector(this.browserContext, this.sessionProvider, collect => {
            return {
                console: event => {
                    collect(event);
                },
                pageerror: event => {
                    if (event instanceof Error) {
                        collect(event);
                    }
                    else {
                        const error = new Error(`${event}`);
                        error.stack = undefined;
                        collect(error);
                    }
                },
                issue: event => {
                    collect(event);
                },
            };
        });
        this.#webSocketCollector = new WebSocketCollector(this.browserContext, this.sessionProvider);
    }
    // Whether CDP-heavy collectors have been initialized.
    // Deferred to avoid polluting the browser with CDP domains during navigation,
    // which anti-bot systems can detect (Debugger.enable, Network events, etc.).
    #collectorsInitialized = false;
    async #init() {
        await this.createPagesSnapshot();
        // NOTE: addInitScript is already called in browser.ts (launch/connect).
        // Do NOT call it again here — double injection causes scripts to run twice
        // per page load, which can create detectable discrepancies.
        // Initialize Playwright-level listeners early so that page load requests
        // and console messages are captured immediately. These only register
        // Node.js event listeners on Playwright objects — no extra CDP domains
        // are activated, so anti-bot systems cannot detect them.
        await this.#networkCollector.init();
        await this.#consoleCollector.init();
        // NOTE: CDP-heavy collectors (initiator collection, Audits.enable,
        // WebSocket CDP events, Debugger.enable) are NOT initialized here.
        // They are lazily initialized on first tool use that needs them,
        // via ensureCollectorsInitialized(). This prevents CDP domain activation
        // from leaking automation signals during page navigation.
    }
    /**
     * Lazily initialize CDP-dependent collectors (network, console, websocket, debugger).
     * Called before any tool that needs collected data.
     * This defers CDP domain activation so that page navigations happen in a
     * "clean" state without Debugger/Network/Runtime domains enabled.
     */
    async ensureCollectorsInitialized() {
        if (this.#collectorsInitialized)
            return;
        this.#collectorsInitialized = true;
        this.logger('Initializing CDP collectors (deferred)');
        // Activate CDP-dependent features for network/console collectors
        // that already have Playwright listeners running.
        await this.#networkCollector.initCdp();
        await this.#consoleCollector.initCdp();
        // WebSocket collector is fully CDP-based, initialize it entirely here.
        await this.#webSocketCollector.init();
        await this.#initDebugger();
    }
    async #initDebugger(frame) {
        const page = this.getSelectedPage();
        if (!page) {
            return;
        }
        try {
            let client;
            if (frame && frame !== page.mainFrame()) {
                client = await this.sessionProvider.getSession(frame);
            }
            else {
                client = await this.sessionProvider.getSession(page);
            }
            await this.#debuggerContext.enable(client);
        }
        catch (error) {
            this.logger('Failed to initialize debugger context', error);
        }
    }
    dispose() {
        this.#networkCollector.dispose();
        this.#consoleCollector.dispose();
        this.#webSocketCollector.dispose();
        void this.#debuggerContext.disable();
    }
    /**
     * Get the debugger context for script/breakpoint management.
     */
    get debuggerContext() {
        return this.#debuggerContext;
    }
    /**
     * Reinitialize the debugger for the current page.
     * Clears stale script IDs, re-enables the debugger to receive fresh
     * scriptParsed events, and restores any previously set breakpoints.
     * Called after selecting a new page or after in-page navigation
     * (goto/reload/back/forward).
     */
    async reinitDebugger() {
        if (!this.#collectorsInitialized)
            return;
        // Save breakpoint definitions before disable wipes them
        const savedBreakpoints = this.#debuggerContext.getBreakpoints();
        await this.#debuggerContext.disable();
        await this.#initDebugger();
        // Restore breakpoints after re-enabling the debugger
        if (savedBreakpoints.length > 0) {
            await this.#debuggerContext.restoreBreakpoints(savedBreakpoints);
        }
    }
    /**
     * Reinitialize the debugger for a specific frame's CDP session.
     * This enables script collection from cross-origin iframes (OOPIFs).
     */
    async reinitDebuggerForFrame(frame) {
        if (!this.#collectorsInitialized)
            return;
        await this.#debuggerContext.disable();
        await this.#initDebugger(frame);
    }
    static async from(browserContext, logger, opts) {
        const context = new McpContext(browserContext, logger, opts);
        await context.#init();
        return context;
    }
    resolveCdpRequestId(cdpRequestId) {
        const selectedPage = this.getSelectedPage();
        if (!cdpRequestId) {
            this.logger('no network request');
            return;
        }
        const request = this.#networkCollector.find(selectedPage, request => {
            return this.#networkCollector.getCdpRequestId(request) === cdpRequestId;
        });
        if (!request) {
            this.logger('no network request for ' + cdpRequestId);
            return;
        }
        return this.#networkCollector.getIdForResource(request);
    }
    getNetworkRequests(includePreservedRequests) {
        const page = this.getSelectedPage();
        return this.#networkCollector.getData(page, includePreservedRequests);
    }
    getConsoleData(includePreservedMessages) {
        const page = this.getSelectedPage();
        return this.#consoleCollector.getData(page, includePreservedMessages);
    }
    getConsoleMessageStableId(message) {
        return this.#consoleCollector.getIdForResource(message);
    }
    getConsoleMessageById(id) {
        return this.#consoleCollector.getById(this.getSelectedPage(), id);
    }
    async newPage() {
        const page = await this.browserContext.newPage();
        await this.createPagesSnapshot();
        this.selectPage(page);
        // Always add to network/console collectors — their Playwright listeners
        // are active from startup. addPage() internally handles CDP setup if
        // initCdp() has already been called.
        this.#networkCollector.addPage(page);
        this.#consoleCollector.addPage(page);
        // WebSocket collector is fully CDP-based, only add if initialized.
        if (this.#collectorsInitialized) {
            await this.#webSocketCollector.addPage(page);
        }
        return page;
    }
    async closePage(pageIdx) {
        if (this.#pages.length === 1) {
            throw new Error(CLOSE_PAGE_ERROR);
        }
        const page = this.getPageByIdx(pageIdx);
        await page.close({ runBeforeUnload: false });
    }
    getNetworkRequestById(reqid) {
        return this.#networkCollector.getById(this.getSelectedPage(), reqid);
    }
    setNetworkConditions(conditions) {
        const page = this.getSelectedPage();
        if (conditions === null) {
            this.#networkConditionsMap.delete(page);
        }
        else {
            this.#networkConditionsMap.set(page, conditions);
        }
        this.#updateSelectedPageTimeouts();
    }
    getNetworkConditions() {
        const page = this.getSelectedPage();
        return this.#networkConditionsMap.get(page) ?? null;
    }
    setCpuThrottlingRate(rate) {
        const page = this.getSelectedPage();
        this.#cpuThrottlingRateMap.set(page, rate);
        this.#updateSelectedPageTimeouts();
    }
    getCpuThrottlingRate() {
        const page = this.getSelectedPage();
        return this.#cpuThrottlingRateMap.get(page) ?? 1;
    }
    setIsRunningPerformanceTrace(x) {
        this.#isRunningTrace = x;
    }
    isRunningPerformanceTrace() {
        return this.#isRunningTrace;
    }
    getDialog() {
        return this.#dialog;
    }
    clearDialog() {
        this.#dialog = undefined;
    }
    getSelectedPage() {
        const page = this.#selectedPage;
        if (!page) {
            throw new Error('No page selected');
        }
        if (page.isClosed()) {
            throw new Error(`The selected page has been closed. Call ${selectPage.name} to see open pages.`);
        }
        return page;
    }
    getPageByIdx(idx) {
        const pages = this.#pages;
        const page = pages[idx];
        if (!page) {
            throw new Error('No page found');
        }
        return page;
    }
    #dialogHandler = (dialog) => {
        this.#dialog = dialog;
    };
    isPageSelected(page) {
        return this.#selectedPage === page;
    }
    selectPage(newPage) {
        const oldPage = this.#selectedPage;
        if (oldPage) {
            oldPage.off('dialog', this.#dialogHandler);
        }
        this.#selectedPage = newPage;
        this.#selectedFrame = undefined;
        newPage.on('dialog', this.#dialogHandler);
        this.#updateSelectedPageTimeouts();
        // Reinitialize debugger for the new page
        void this.reinitDebugger();
    }
    getSelectedFrame() {
        return this.#selectedFrame ?? this.getSelectedPage().mainFrame();
    }
    selectFrame(frame) {
        this.#selectedFrame = frame;
        // Reinitialize debugger for the frame's CDP session
        // so that scripts from cross-origin iframes (OOPIFs) are visible
        void this.reinitDebuggerForFrame(frame);
    }
    resetSelectedFrame() {
        this.#selectedFrame = undefined;
        // Reinitialize debugger for the main page's CDP session
        void this.reinitDebugger();
    }
    #updateSelectedPageTimeouts() {
        const page = this.getSelectedPage();
        // For waiters 5sec timeout should be sufficient.
        // Increased in case we throttle the CPU
        const cpuMultiplier = this.getCpuThrottlingRate();
        page.setDefaultTimeout(DEFAULT_TIMEOUT * cpuMultiplier);
        // 10sec should be enough for the load event to be emitted during
        // navigations.
        // Increased in case we throttle the network requests
        const networkMultiplier = getNetworkMultiplierFromString(this.getNetworkConditions());
        this.#navigationTimeout = NAVIGATION_TIMEOUT * networkMultiplier;
        page.setDefaultNavigationTimeout(this.#navigationTimeout);
    }
    getNavigationTimeout() {
        return this.#navigationTimeout;
    }
    /**
     * Creates a snapshot of the pages.
     */
    async createPagesSnapshot() {
        const allPages = this.browserContext.pages();
        this.#pages = allPages.filter(page => {
            // If we allow debugging DevTools windows, return all pages.
            // If we are in regular mode, the user should only see non-DevTools page.
            return (this.#options.experimentalDevToolsDebugging ||
                !page.url().startsWith('devtools://'));
        });
        if (!this.#selectedPage || this.#pages.indexOf(this.#selectedPage) === -1) {
            this.selectPage(this.#pages[0]);
        }
        // Skip DevTools window detection when collectors aren't initialized.
        // detectOpenDevToolsWindows() creates CDP sessions which leak automation
        // signals to anti-bot systems during navigation.
        if (this.#collectorsInitialized) {
            await this.detectOpenDevToolsWindows();
        }
        return this.#pages;
    }
    async detectOpenDevToolsWindows() {
        this.logger('Detecting open DevTools windows');
        const pages = this.browserContext.pages();
        this.#pageToDevToolsPage = new Map();
        for (const devToolsPage of pages) {
            if (devToolsPage.url().startsWith('devtools://')) {
                try {
                    this.logger('Calling getTargetInfo for ' + devToolsPage.url());
                    const session = await this.sessionProvider.getSession(devToolsPage);
                    const data = await session.send('Target.getTargetInfo');
                    const devtoolsPageTitle = data.targetInfo.title;
                    const urlLike = extractUrlLikeFromDevToolsTitle(devtoolsPageTitle);
                    if (!urlLike) {
                        continue;
                    }
                    // TODO: lookup without a loop.
                    for (const page of this.#pages) {
                        if (urlsEqual(page.url(), urlLike)) {
                            this.#pageToDevToolsPage.set(page, devToolsPage);
                        }
                    }
                }
                catch (error) {
                    this.logger('Issue occurred while trying to find DevTools', error);
                }
            }
        }
    }
    getPages() {
        return this.#pages;
    }
    getDevToolsPage(page) {
        return this.#pageToDevToolsPage.get(page);
    }
    async getDevToolsData() {
        try {
            this.logger('Getting DevTools UI data');
            const selectedPage = this.getSelectedPage();
            const devtoolsPage = this.getDevToolsPage(selectedPage);
            if (!devtoolsPage) {
                this.logger('No DevTools page detected');
                return {};
            }
            const { cdpRequestId, cdpBackendNodeId } = await devtoolsPage.evaluate(async () => {
                // @ts-expect-error no types
                const UI = await import('/bundled/ui/legacy/legacy.js');
                // @ts-expect-error no types
                const SDK = await import('/bundled/core/sdk/sdk.js');
                const request = UI.Context.Context.instance().flavor(SDK.NetworkRequest.NetworkRequest);
                const node = UI.Context.Context.instance().flavor(SDK.DOMModel.DOMNode);
                return {
                    cdpRequestId: request?.requestId(),
                    cdpBackendNodeId: node?.backendNodeId(),
                };
            });
            return { cdpBackendNodeId, cdpRequestId };
        }
        catch (err) {
            this.logger('error getting devtools data', err);
        }
        return {};
    }
    async saveTemporaryFile(data, mimeType) {
        try {
            const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'chrome-devtools-mcp-'));
            const filename = path.join(dir, `screenshot.${getExtensionFromMimeType(mimeType)}`);
            await fs.writeFile(filename, data);
            return { filename };
        }
        catch (err) {
            this.logger(err);
            throw new Error('Could not save a screenshot to a file', { cause: err });
        }
    }
    async saveFile(data, filename) {
        try {
            const filePath = path.resolve(filename);
            await fs.writeFile(filePath, data);
            return { filename };
        }
        catch (err) {
            this.logger(err);
            throw new Error('Could not save a screenshot to a file', { cause: err });
        }
    }
    storeTraceRecording(result) {
        this.#traceResults.push(result);
    }
    recordedTraces() {
        return this.#traceResults;
    }
    getWaitForHelper(page, cpuMultiplier, networkMultiplier) {
        return WaitForHelper.create(page, this.sessionProvider, cpuMultiplier, networkMultiplier);
    }
    async waitForEventsAfterAction(action) {
        const page = this.getSelectedPage();
        const cpuMultiplier = this.getCpuThrottlingRate();
        const networkMultiplier = getNetworkMultiplierFromString(this.getNetworkConditions());
        const waitForHelper = await this.getWaitForHelper(page, cpuMultiplier, networkMultiplier);
        return waitForHelper.waitForEventsAfterAction(action);
    }
    getNetworkRequestStableId(request) {
        return this.#networkCollector.getIdForResource(request);
    }
    /**
     * Get the initiator (call stack) for a network request.
     */
    getRequestInitiator(request) {
        const page = this.getSelectedPage();
        return this.#networkCollector.getInitiator(page, request);
    }
    /**
     * Get the initiator by request ID.
     */
    getRequestInitiatorById(requestId) {
        const page = this.getSelectedPage();
        const request = this.#networkCollector.getById(page, requestId);
        return this.#networkCollector.getInitiator(page, request);
    }
    /**
     * Get all WebSocket connections for the selected page.
     */
    getWebSocketConnections(includePreservedData) {
        const page = this.getSelectedPage();
        return this.#webSocketCollector.getData(page, includePreservedData);
    }
    /**
     * Get a WebSocket connection by stable ID.
     */
    getWebSocketById(wsid) {
        const page = this.getSelectedPage();
        return this.#webSocketCollector.getById(page, wsid);
    }
    /**
     * Get stable ID for a WebSocket connection.
     */
    getWebSocketStableId(ws) {
        return this.#webSocketCollector.getIdForResource(ws);
    }
    /**
     * Cache traffic summary for a WebSocket connection.
     */
    cacheTrafficSummary(wsid, summary) {
        this.#trafficSummaryCache.set(wsid, summary);
    }
    /**
     * Get cached traffic summary for a WebSocket connection.
     */
    getCachedTrafficSummary(wsid) {
        return this.#trafficSummaryCache.get(wsid);
    }
    trackInjectedScript(identifier, source) {
        const page = this.getSelectedPage();
        let map = this.#injectedScriptsByPage.get(page);
        if (!map) {
            map = new Map();
            this.#injectedScriptsByPage.set(page, map);
        }
        map.set(identifier, source);
    }
    untrackInjectedScript(identifier) {
        const page = this.getSelectedPage();
        const map = this.#injectedScriptsByPage.get(page);
        if (!map)
            return false;
        return map.delete(identifier);
    }
    getInjectedScriptIds() {
        const page = this.getSelectedPage();
        const map = this.#injectedScriptsByPage.get(page);
        if (!map)
            return [];
        return [...map.keys()];
    }
    async waitForTextOnPage({ text, timeout, }) {
        const page = this.getSelectedPage();
        const frames = page.frames();
        // Use Promise.race with Playwright's getByText across all frames
        const locators = frames.flatMap(frame => [
            frame.getByRole('link', { name: text }),
            frame.getByRole('button', { name: text }),
            frame.getByText(text),
        ]);
        const waitPromises = locators.map(locator => locator.waitFor({ timeout: timeout ?? 5000 }).catch(() => null));
        await Promise.race(waitPromises);
        return undefined;
    }
    /**
     * We need to ignore favicon request as they make our test flaky
     */
    async setUpNetworkCollectorForTesting() {
        this.#networkCollector = new NetworkCollector(this.browserContext, this.sessionProvider, collect => {
            return {
                request: req => {
                    if (req.url().includes('favicon.ico')) {
                        return;
                    }
                    collect(req);
                },
            };
        });
        await this.#networkCollector.init();
    }
}
