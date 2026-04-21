/**
 * @license
 * Copyright 2025 Google LLC
 * SPDX-License-Identifier: Apache-2.0
 */
import os from 'node:os';
import path from 'node:path';
import { logger } from './logger.js';
import { DEFAULT_ARGS, STEALTH_ARGS, HARMFUL_ARGS } from './stealth-args.js';
import { chromium } from './third_party/index.js';
let browserResult;
// Default persistent user data directory for login state, cookies, etc.
const DEFAULT_USER_DATA_DIR = path.join(os.homedir(), '.cache', 'chrome-devtools-mcp', 'chrome-profile');
export async function ensureBrowserConnected(options) {
    if (browserResult) {
        return browserResult;
    }
    let endpoint = options.wsEndpoint;
    // If browserURL is given (e.g. http://localhost:9222), resolve to ws endpoint
    if (!endpoint && options.browserURL) {
        const url = new URL('/json/version', options.browserURL);
        const res = await fetch(url.toString());
        const json = (await res.json());
        endpoint = json.webSocketDebuggerUrl;
    }
    if (!endpoint) {
        throw new Error('Either browserURL or wsEndpoint must be provided');
    }
    logger('Connecting Patchright via CDP to', endpoint);
    const browser = await chromium.connectOverCDP(endpoint, {
        headers: options.wsHeaders,
    });
    logger('Connected Patchright');
    const context = browser.contexts()[0];
    if (!context) {
        throw new Error('No browser context found after connecting');
    }
    browserResult = { browser, context };
    // Clear cached result when browser disconnects so we can reconnect.
    browser.on('disconnected', () => {
        logger('Browser disconnected, clearing cached browser result');
        browserResult = undefined;
    });
    return browserResult;
}
export async function launch(options) {
    const { channel, executablePath, headless, isolated } = options;
    const args = [
        ...DEFAULT_ARGS,
        ...(options.noStealth ? [] : STEALTH_ARGS),
        ...(options.args ?? []),
        '--hide-crash-restore-bubble',
    ];
    if (headless) {
        args.push('--screen-info={3840x2160}');
    }
    if (options.devtools) {
        args.push('--auto-open-devtools-for-tabs');
    }
    if (options.hideCanvas) {
        args.push('--fingerprinting-canvas-image-data-noise');
    }
    if (options.blockWebrtc) {
        args.push('--webrtc-ip-handling-policy=disable_non_proxied_udp', '--force-webrtc-ip-handling-policy');
    }
    if (options.disableWebgl) {
        args.push('--disable-webgl', '--disable-webgl-image-chromium', '--disable-webgl2');
    }
    // Resolve Chrome channel for Patchright
    let patchrightChannel;
    if (!executablePath) {
        if (channel === 'canary') {
            patchrightChannel = 'chrome-canary';
        }
        else if (channel === 'beta') {
            patchrightChannel = 'chrome-beta';
        }
        else if (channel === 'dev') {
            patchrightChannel = 'chrome-dev';
        }
        else {
            patchrightChannel = 'chrome';
        }
    }
    // Use viewport: null to disable Playwright's viewport emulation.
    // This exposes real OS window/screen dimensions (no fake 1920x1080).
    // Note: deviceScaleFactor is incompatible with viewport: null.
    const hasCustomViewport = !!options.viewport;
    const contextOptions = {
        viewport: hasCustomViewport ? options.viewport : null,
        ...(hasCustomViewport ? {
            screen: { width: options.viewport.width, height: options.viewport.height },
            deviceScaleFactor: 2,
        } : {}),
        colorScheme: 'dark',
        isMobile: false,
        hasTouch: false,
        serviceWorkers: 'allow',
        permissions: ['geolocation', 'notifications'],
        ignoreHTTPSErrors: options.acceptInsecureCerts ?? true,
    };
    // --isolated mode: launch() + newContext() for clean isolated context.
    // Creates an incognito-like context with no persisted state.
    if (isolated) {
        const browser = await chromium.launch({
            channel: patchrightChannel,
            executablePath,
            headless,
            args,
            ignoreDefaultArgs: options.noStealth ? undefined : HARMFUL_ARGS,
        });
        const context = await browser.newContext(contextOptions);
        if (context.pages().length === 0) {
            await context.newPage();
        }
        return { browser, context };
    }
    // Default: launchPersistentContext for full state persistence
    // (cookies, IndexedDB, Cache Storage, Service Workers, localStorage).
    const userDataDir = options.userDataDir ?? DEFAULT_USER_DATA_DIR;
    try {
        const context = await chromium.launchPersistentContext(userDataDir, {
            channel: patchrightChannel,
            executablePath,
            headless,
            args,
            ignoreDefaultArgs: options.noStealth ? undefined : HARMFUL_ARGS,
            ...contextOptions,
        });
        return { browser: undefined, context };
    }
    catch (error) {
        if (error.message.includes('The browser is already running')) {
            throw new Error(`The browser is already running for ${userDataDir}. Use --isolated to run a separate browser instance.`, { cause: error });
        }
        throw error;
    }
}
export async function ensureBrowserLaunched(options) {
    if (browserResult) {
        return browserResult;
    }
    browserResult = await launch(options);
    // Clear cached result when browser is manually closed so we can relaunch.
    const { browser, context } = browserResult;
    if (browser) {
        browser.on('disconnected', () => {
            logger('Browser disconnected, clearing cached browser result');
            browserResult = undefined;
        });
    }
    else {
        // Persistent context mode (no browser object) — listen on context.
        context.on('close', () => {
            logger('Browser context closed, clearing cached browser result');
            browserResult = undefined;
        });
    }
    return browserResult;
}
