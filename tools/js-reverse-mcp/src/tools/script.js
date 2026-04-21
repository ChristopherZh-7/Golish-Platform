/**
 * @license
 * Copyright 2025 Google LLC
 * SPDX-License-Identifier: Apache-2.0
 */
import { zod } from '../third_party/index.js';
import { ToolCategory } from './categories.js';
import { defineTool } from './ToolDefinition.js';
// Default script evaluation timeout in milliseconds (30 seconds)
const DEFAULT_SCRIPT_TIMEOUT = 30000;
function withTimeout(promise, ms, message) {
    return Promise.race([
        promise,
        new Promise((_, reject) => {
            setTimeout(() => reject(new Error(message)), ms);
        }),
    ]);
}
export const evaluateScript = defineTool({
    name: 'evaluate_script',
    description: `Evaluate a JavaScript function inside the currently selected page. Returns the response as JSON
so returned values have to JSON-serializable. When execution is paused at a breakpoint, automatically evaluates in the paused call frame context.`,
    annotations: {
        category: ToolCategory.DEBUGGING,
        readOnlyHint: false,
    },
    schema: {
        function: zod.string().describe(`A JavaScript function declaration to be executed by the tool in the currently selected page.
Example without arguments: \`() => {
  return document.title
}\` or \`async () => {
  return await fetch("example.com")
}\`.
Example with arguments: \`(el) => {
  return el.innerText;
}\`
`),
        mainWorld: zod
            .boolean()
            .optional()
            .default(false)
            .describe('Execute the function in the page main world instead of the default isolated context. ' +
            'Use this when you need to access page-defined globals (e.g. window.bdms, window.app). ' +
            'The function must be synchronous and return a JSON-serializable value.'),
        frameIndex: zod
            .number()
            .int()
            .optional()
            .describe('When paused at a breakpoint, which call frame to evaluate in (0 = top frame). ' +
            'If omitted, uses the top frame. Use get_paused_info to see available frames.'),
    },
    handler: async (request, response, context) => {
        // When paused at a breakpoint, evaluate in the paused call frame context
        // to avoid a 30s timeout that would confuse the agent.
        const debugger_ = context.debuggerContext;
        if (debugger_.isEnabled() && debugger_.isPaused()) {
            const pausedState = debugger_.getPausedState();
            const frameIdx = request.params.frameIndex ?? 0;
            if (frameIdx < 0 || frameIdx >= pausedState.callFrames.length) {
                throw new Error(`frameIndex ${frameIdx} is out of range (0-${pausedState.callFrames.length - 1})`);
            }
            const callFrameId = pausedState.callFrames[frameIdx]?.callFrameId;
            if (callFrameId) {
                const expression = `JSON.stringify((${request.params.function})())`;
                const result = await debugger_.evaluateOnCallFrame(callFrameId, expression, { returnByValue: true });
                if (result.exceptionDetails) {
                    const errMsg = result.exceptionDetails.exception?.description ||
                        result.exceptionDetails.text;
                    throw new Error(`Script evaluation error: ${errMsg}`);
                }
                const value = result.result.value;
                response.appendResponseLine('Script ran on page (paused context) and returned:');
                response.appendResponseLine('```json');
                response.appendResponseLine(`${value ?? 'undefined'}`);
                response.appendResponseLine('```');
                return;
            }
        }
        if (request.params.mainWorld) {
            // Main world execution via script tag injection + DOM bridge.
            //
            // Why: Patchright (our browser automation library) deliberately runs
            // frame.evaluate() in an isolated ExecutionContext by default. This is
            // its core anti-detection mechanism — it avoids calling CDP
            // Runtime.enable, which all major anti-bot systems (Cloudflare,
            // DataDome, ByteDance bdms, etc.) can detect.
            //
            // The trade-off is that isolated contexts cannot access page-defined
            // globals like window.bdms or window.app. To work around this without
            // breaking stealth, we inject a <script> tag (which always executes in
            // the main world) and pass the result back via a DOM attribute (the DOM
            // is shared between worlds).
            const frame = context.getSelectedFrame();
            const bridgeId = `__mcp_bridge_${Date.now()}_${Math.random().toString(36).slice(2)}`;
            const result = await withTimeout(frame.evaluate(async ({ fn, id }) => {
                const el = document.createElement('div');
                el.id = id;
                el.style.display = 'none';
                document.documentElement.appendChild(el);
                const script = document.createElement('script');
                script.textContent = `
            (function() {
              var el = document.getElementById(${JSON.stringify(id)});
              try {
                var result = (${fn})();
                el.setAttribute('data-result', JSON.stringify(result));
              } catch(e) {
                el.setAttribute('data-error', e.message || String(e));
              }
            })();
          `;
                document.documentElement.appendChild(script);
                script.remove();
                // Read result from the DOM bridge
                const data = el.getAttribute('data-result');
                const error = el.getAttribute('data-error');
                el.remove();
                if (error) {
                    throw new Error(error);
                }
                return data ?? 'undefined';
            }, { fn: request.params.function, id: bridgeId }), DEFAULT_SCRIPT_TIMEOUT, 'Script evaluation timed out');
            response.appendResponseLine('Script ran on page (main world) and returned:');
            response.appendResponseLine('```json');
            response.appendResponseLine(`${result}`);
            response.appendResponseLine('```');
            return;
        }
        let fn;
        try {
            const frame = context.getSelectedFrame();
            fn = await withTimeout(frame.evaluateHandle(`(${request.params.function})`), DEFAULT_SCRIPT_TIMEOUT, 'Script evaluation timed out');
            await context.waitForEventsAfterAction(async () => {
                const result = await withTimeout(frame.evaluate(async (fn) => {
                    // @ts-expect-error no types.
                    return JSON.stringify(await fn());
                }, fn), DEFAULT_SCRIPT_TIMEOUT, 'Script execution timed out');
                response.appendResponseLine('Script ran on page and returned:');
                response.appendResponseLine('```json');
                response.appendResponseLine(`${result}`);
                response.appendResponseLine('```');
            });
        }
        finally {
            if (fn) {
                void fn.dispose();
            }
        }
    },
});
