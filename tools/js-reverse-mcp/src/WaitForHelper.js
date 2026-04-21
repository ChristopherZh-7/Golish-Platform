/**
 * @license
 * Copyright 2025 Google LLC
 * SPDX-License-Identifier: Apache-2.0
 */
import { logger } from './logger.js';
export class WaitForHelper {
    #abortController = new AbortController();
    #page;
    #cdpSession;
    #stableDomTimeout;
    #stableDomFor;
    #expectNavigationIn;
    #navigationTimeout;
    constructor(page, cdpSession, cpuTimeoutMultiplier, networkTimeoutMultiplier) {
        this.#stableDomTimeout = 3000 * cpuTimeoutMultiplier;
        this.#stableDomFor = 100 * cpuTimeoutMultiplier;
        this.#expectNavigationIn = 100 * cpuTimeoutMultiplier;
        this.#navigationTimeout = 3000 * networkTimeoutMultiplier;
        this.#page = page;
        this.#cdpSession = cdpSession;
    }
    static async create(page, sessionProvider, cpuTimeoutMultiplier, networkTimeoutMultiplier) {
        const session = await sessionProvider.getSession(page);
        return new WaitForHelper(page, session, cpuTimeoutMultiplier, networkTimeoutMultiplier);
    }
    /**
     * A wrapper that executes a action and waits for
     * a potential navigation, after which it waits
     * for the DOM to be stable before returning.
     */
    async waitForStableDom() {
        const stableDomObserver = await this.#page.evaluateHandle(timeout => {
            let timeoutId;
            function callback() {
                clearTimeout(timeoutId);
                timeoutId = setTimeout(() => {
                    domObserver.resolver.resolve();
                    domObserver.observer.disconnect();
                }, timeout);
            }
            const domObserver = {
                resolver: Promise.withResolvers(),
                observer: new MutationObserver(callback),
            };
            // It's possible that the DOM is not gonna change so we
            // need to start the timeout initially.
            callback();
            domObserver.observer.observe(document.body, {
                childList: true,
                subtree: true,
                attributes: true,
            });
            return domObserver;
        }, this.#stableDomFor);
        this.#abortController.signal.addEventListener('abort', async () => {
            try {
                await stableDomObserver.evaluate(observer => {
                    observer.observer.disconnect();
                    observer.resolver.resolve();
                });
                await stableDomObserver.dispose();
            }
            catch {
                // Ignored cleanup errors
            }
        });
        return Promise.race([
            stableDomObserver.evaluate(async (observer) => {
                return await observer.resolver.promise;
            }),
            this.timeout(this.#stableDomTimeout).then(() => {
                throw new Error('Timeout');
            }),
        ]);
    }
    async waitForNavigationStarted() {
        const navigationStartedPromise = new Promise(resolve => {
            const listener = (event) => {
                if ([
                    'historySameDocument',
                    'historyDifferentDocument',
                    'sameDocument',
                ].includes(event.navigationType)) {
                    resolve(false);
                    return;
                }
                resolve(true);
            };
            this.#cdpSession.on('Page.frameStartedNavigating', listener);
            this.#abortController.signal.addEventListener('abort', () => {
                resolve(false);
                this.#cdpSession.off('Page.frameStartedNavigating', listener);
            });
        });
        return await Promise.race([
            navigationStartedPromise,
            this.timeout(this.#expectNavigationIn).then(() => false),
        ]);
    }
    timeout(time) {
        return new Promise(res => {
            const id = setTimeout(res, time);
            this.#abortController.signal.addEventListener('abort', () => {
                res();
                clearTimeout(id);
            });
        });
    }
    async waitForEventsAfterAction(action) {
        // Overall timeout to prevent infinite hanging (15 seconds max)
        const overallTimeout = new Promise((_, reject) => {
            const id = setTimeout(() => {
                reject(new Error('Overall navigation timeout'));
            }, 15000);
            this.#abortController.signal.addEventListener('abort', () => {
                clearTimeout(id);
            });
        });
        const doAction = async () => {
            const navigationFinished = this.waitForNavigationStarted()
                .then(navigationStarted => {
                if (navigationStarted) {
                    return this.#page.waitForLoadState('domcontentloaded', {
                        timeout: this.#navigationTimeout,
                    });
                }
                return;
            })
                .catch(error => logger(error));
            try {
                await action();
            }
            catch (error) {
                // Clear up pending promises
                this.#abortController.abort();
                throw error;
            }
            try {
                await navigationFinished;
                // Wait for stable dom after navigation so we execute in
                // the correct context
                await this.waitForStableDom();
            }
            catch (error) {
                logger(error);
            }
            finally {
                this.#abortController.abort();
            }
        };
        try {
            await Promise.race([doAction(), overallTimeout]);
        }
        catch (error) {
            logger(error);
            this.#abortController.abort();
        }
    }
}
