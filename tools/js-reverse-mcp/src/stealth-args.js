/**
 * @license
 * Copyright 2026 Google LLC
 * SPDX-License-Identifier: Apache-2.0
 */
/**
 * Stealth launch arguments for anti-detection.
 *
 * Based on Scrapling's multi-layer defense system:
 * - Layer 2: Chromium stealth launch arguments
 * - Layer 3: Browser context spoofing options
 */
/**
 * Harmful args that Patchright/Playwright adds by default.
 * These leak automation signals and must be removed via ignoreDefaultArgs.
 */
export const HARMFUL_ARGS = [
    '--enable-automation',
    '--disable-popup-blocking',
    '--disable-component-update',
    '--disable-default-apps',
    '--disable-extensions',
];
/**
 * Basic args used in all modes.
 */
export const DEFAULT_ARGS = [
    '--no-pings',
    '--no-first-run',
    '--disable-infobars',
    '--disable-breakpad',
    '--no-service-autorun',
    '--homepage=about:blank',
    '--password-store=basic',
    '--disable-hang-monitor',
    '--no-default-browser-check',
    '--disable-session-crashed-bubble',
    '--disable-search-engine-choice-screen',
];
/**
 * 60+ anti-detection arguments covering:
 * - Automation feature removal
 * - Headless detection bypass
 * - Device fingerprint spoofing
 * - GPU & rendering normalization
 * - Network characteristics
 * - Behavioral trait hiding
 * - Privacy & security hardening
 */
export const STEALTH_ARGS = [
    // Automation feature removal
    '--disable-blink-features=AutomationControlled',
    '--test-type',
    // Headless detection bypass
    '--start-maximized',
    '--window-position=0,0',
    '--window-size=1920,1080',
    // Localization
    '--lang=en-US',
    '--accept-lang=en-US',
    // Device fingerprint spoofing
    '--blink-settings=primaryHoverType=2,availableHoverTypes=2,primaryPointerType=4,availablePointerTypes=4',
    // GPU & rendering
    '--ignore-gpu-blocklist',
    '--force-color-profile=srgb',
    '--font-render-hinting=none',
    // Network features
    '--enable-features=NetworkService,NetworkServiceInProcess,TrustTokens,TrustTokensAlwaysAllowIssuance',
    '--disable-features=AudioServiceOutOfProcess,TranslateUI,BlinkGenPropertyTrees',
    '--enable-async-dns',
    '--enable-tcp-fast-open',
    '--enable-web-bluetooth',
    // Audio/media
    '--mute-audio',
    // Sync & data
    '--disable-sync',
    '--use-mock-keychain',
    '--disable-translate',
    '--disable-voice-input',
    // UI elements
    '--hide-scrollbars',
    // Behavioral trait hiding
    '--autoplay-policy=user-gesture-required',
    '--disable-ipc-flooding-protection',
    '--disable-renderer-backgrounding',
    '--disable-backgrounding-occluded-windows',
    // Privacy & security
    '--disable-client-side-phishing-detection',
    '--safebrowsing-disable-auto-update',
    '--disable-domain-reliability',
    '--metrics-recording-only',
    '--disable-cookie-encryption',
    // Performance & caching
    '--disable-logging',
    '--disable-dev-shm-usage',
    '--disable-crash-reporter',
    '--disable-partial-raster',
    '--disable-gesture-typing',
    '--disable-checker-imaging',
    '--disable-prompt-on-repost',
    '--aggressive-cache-discard',
    '--disable-threaded-animation',
    '--disable-threaded-scrolling',
    '--enable-simple-cache-backend',
    '--disable-background-networking',
    '--disable-background-timer-throttling',
    '--disable-new-content-rendering-timeout',
    '--disable-image-animation-resync',
    '--disable-offer-upload-credit-cards',
    '--disable-offer-store-unmasked-wallet-cards',
    // Rendering & sync
    '--enable-surface-synchronization',
    '--run-all-compositor-stages-before-draw',
    // Other
    '--cloud-import',
    '--disable-print-preview',
    '--prerender-from-omnibox=disabled',
    '--disable-layer-tree-host-memory-pressure',
    '--disable-component-extensions-with-background-pages',
];
