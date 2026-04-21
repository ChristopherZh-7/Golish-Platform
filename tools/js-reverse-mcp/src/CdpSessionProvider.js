/**
 * @license
 * Copyright 2025 Google LLC
 * SPDX-License-Identifier: Apache-2.0
 */
/**
 * CDP Session cache layer for Playwright/Patchright.
 *
 * In Puppeteer, `page._client()` is synchronous and returns the same session.
 * In Playwright, `page.context().newCDPSession(page)` is async and creates
 * a new session each time. This provider caches sessions per Page/Frame.
 */
export class CdpSessionProvider {
    #pageSessions = new WeakMap();
    #frameSessions = new WeakMap();
    #context;
    constructor(context) {
        this.#context = context;
    }
    async getSession(pageOrFrame) {
        // Check if it's a Page (has context() method that returns BrowserContext)
        if ('context' in pageOrFrame && typeof pageOrFrame.context === 'function') {
            // It could be either Page or Frame - check for mainFrame to distinguish
            if ('mainFrame' in pageOrFrame) {
                return this.#getPageSession(pageOrFrame);
            }
        }
        return this.#getFrameSession(pageOrFrame);
    }
    async #getPageSession(page) {
        const cached = this.#pageSessions.get(page);
        if (cached) {
            return cached;
        }
        const session = await this.#context.newCDPSession(page);
        this.#pageSessions.set(page, session);
        return session;
    }
    async #getFrameSession(frame) {
        const cached = this.#frameSessions.get(frame);
        if (cached) {
            return cached;
        }
        // Playwright's newCDPSession accepts Frame directly for OOPIFs
        const session = await this.#context.newCDPSession(frame);
        this.#frameSessions.set(frame, session);
        return session;
    }
    /**
     * Invalidate cached session for a page or frame.
     * Call this when the page/frame is closed or navigated.
     */
    invalidate(pageOrFrame) {
        if ('mainFrame' in pageOrFrame) {
            const session = this.#pageSessions.get(pageOrFrame);
            if (session) {
                void session.detach().catch(() => { });
                this.#pageSessions.delete(pageOrFrame);
            }
        }
        else {
            const session = this.#frameSessions.get(pageOrFrame);
            if (session) {
                void session.detach().catch(() => { });
                this.#frameSessions.delete(pageOrFrame);
            }
        }
    }
}
