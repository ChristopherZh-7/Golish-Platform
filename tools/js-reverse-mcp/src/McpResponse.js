/**
 * @license
 * Copyright 2025 Google LLC
 * SPDX-License-Identifier: Apache-2.0
 */
import { AggregatedIssue } from '../node_modules/chrome-devtools-frontend/mcp/mcp.js';
import { mapIssueToMessageObject } from './DevtoolsUtils.js';
import { formatConsoleEventShort, formatConsoleEventVerbose, } from './formatters/consoleFormatter.js';
import { getFormattedHeaderValue, getFormattedResponseBody, getFormattedRequestBody, getShortDescriptionForRequest, getStatusFromRequest, } from './formatters/networkFormatter.js';
import { formatWebSocketConnectionShort, formatWebSocketConnectionVerbose, } from './formatters/websocketFormatter.js';
import { paginate } from './utils/pagination.js';
export class McpResponse {
    #includePages = false;
    #attachedNetworkRequestId;
    #attachedConsoleMessageId;
    #textResponseLines = [];
    #images = [];
    #networkRequestsOptions;
    #consoleDataOptions;
    #webSocketOptions;
    #attachedWebSocketId;
    setIncludePages(value) {
        this.#includePages = value;
    }
    setIncludeNetworkRequests(value, options) {
        if (!value) {
            this.#networkRequestsOptions = undefined;
            return;
        }
        this.#networkRequestsOptions = {
            include: value,
            pagination: options?.pageSize || options?.pageIdx
                ? {
                    pageSize: options.pageSize,
                    pageIdx: options.pageIdx,
                }
                : undefined,
            resourceTypes: options?.resourceTypes,
            urlFilter: options?.urlFilter,
            includePreservedRequests: options?.includePreservedRequests,
            networkRequestIdInDevToolsUI: options?.networkRequestIdInDevToolsUI,
        };
    }
    setIncludeConsoleData(value, options) {
        if (!value) {
            this.#consoleDataOptions = undefined;
            return;
        }
        this.#consoleDataOptions = {
            include: value,
            pagination: options?.pageSize || options?.pageIdx
                ? {
                    pageSize: options.pageSize,
                    pageIdx: options.pageIdx,
                }
                : undefined,
            types: options?.types,
            includePreservedMessages: options?.includePreservedMessages,
        };
    }
    attachNetworkRequest(reqid) {
        this.#attachedNetworkRequestId = reqid;
    }
    attachConsoleMessage(msgid) {
        this.#attachedConsoleMessageId = msgid;
    }
    setIncludeWebSocketConnections(value, options) {
        if (!value) {
            this.#webSocketOptions = undefined;
            return;
        }
        this.#webSocketOptions = {
            include: value,
            pagination: options?.pageSize || options?.pageIdx
                ? {
                    pageSize: options.pageSize,
                    pageIdx: options.pageIdx,
                }
                : undefined,
            urlFilter: options?.urlFilter,
            includePreservedConnections: options?.includePreservedConnections,
        };
    }
    attachWebSocket(wsid) {
        this.#attachedWebSocketId = wsid;
    }
    get includePages() {
        return this.#includePages;
    }
    get includeNetworkRequests() {
        return this.#networkRequestsOptions?.include ?? false;
    }
    get includeConsoleData() {
        return this.#consoleDataOptions?.include ?? false;
    }
    get includeWebSocketConnections() {
        return this.#webSocketOptions?.include ?? false;
    }
    get attachedNetworkRequestId() {
        return this.#attachedNetworkRequestId;
    }
    get attachedWebSocketId() {
        return this.#attachedWebSocketId;
    }
    get networkRequestsPageIdx() {
        return this.#networkRequestsOptions?.pagination?.pageIdx;
    }
    get consoleMessagesPageIdx() {
        return this.#consoleDataOptions?.pagination?.pageIdx;
    }
    get consoleMessagesTypes() {
        return this.#consoleDataOptions?.types;
    }
    appendResponseLine(value) {
        this.#textResponseLines.push(value);
    }
    attachImage(value) {
        this.#images.push(value);
    }
    get responseLines() {
        return this.#textResponseLines;
    }
    get images() {
        return this.#images;
    }
    async handle(toolName, context) {
        if (this.#includePages) {
            await context.createPagesSnapshot();
        }
        const bodies = {};
        if (this.#attachedNetworkRequestId) {
            const request = context.getNetworkRequestById(this.#attachedNetworkRequestId);
            bodies.requestBody = await getFormattedRequestBody(request);
            // In Playwright, request.response() is async
            const response = await request.response();
            if (response) {
                bodies.responseBody = await getFormattedResponseBody(response);
            }
        }
        let consoleData;
        if (this.#attachedConsoleMessageId) {
            const message = context.getConsoleMessageById(this.#attachedConsoleMessageId);
            const consoleMessageStableId = this.#attachedConsoleMessageId;
            if ('args' in message) {
                const consoleMessage = message;
                consoleData = {
                    consoleMessageStableId,
                    type: consoleMessage.type(),
                    message: consoleMessage.text(),
                    args: await Promise.all(consoleMessage.args().map(async (arg) => {
                        const stringArg = await arg.jsonValue().catch(() => {
                            // Ignore errors.
                        });
                        return typeof stringArg === 'object'
                            ? JSON.stringify(stringArg)
                            : String(stringArg);
                    })),
                };
            }
            else if (message instanceof AggregatedIssue) {
                const mappedIssueMessage = mapIssueToMessageObject(message);
                if (!mappedIssueMessage)
                    throw new Error("Can't prpovide detals for the msgid " + consoleMessageStableId);
                consoleData = {
                    consoleMessageStableId,
                    ...mappedIssueMessage,
                };
            }
            else {
                consoleData = {
                    consoleMessageStableId,
                    type: 'error',
                    message: message.message,
                    args: [],
                };
            }
        }
        let consoleListData;
        if (this.#consoleDataOptions?.include) {
            let messages = context.getConsoleData(this.#consoleDataOptions.includePreservedMessages);
            if (this.#consoleDataOptions.types?.length) {
                const normalizedTypes = new Set(this.#consoleDataOptions.types);
                messages = messages.filter(message => {
                    if ('type' in message) {
                        return normalizedTypes.has(message.type());
                    }
                    if (message instanceof AggregatedIssue) {
                        return normalizedTypes.has('issue');
                    }
                    return normalizedTypes.has('error');
                });
            }
            consoleListData = (await Promise.all(messages.map(async (item) => {
                const consoleMessageStableId = context.getConsoleMessageStableId(item);
                if ('args' in item) {
                    const consoleMessage = item;
                    return {
                        consoleMessageStableId,
                        type: consoleMessage.type(),
                        message: consoleMessage.text(),
                        args: await Promise.all(consoleMessage.args().map(async (arg) => {
                            const stringArg = await arg.jsonValue().catch(() => {
                                // Ignore errors.
                            });
                            return typeof stringArg === 'object'
                                ? JSON.stringify(stringArg)
                                : String(stringArg);
                        })),
                    };
                }
                if (item instanceof AggregatedIssue) {
                    const mappedIssueMessage = mapIssueToMessageObject(item);
                    if (!mappedIssueMessage)
                        return null;
                    return {
                        consoleMessageStableId,
                        ...mappedIssueMessage,
                    };
                }
                return {
                    consoleMessageStableId,
                    type: 'error',
                    message: item.message,
                    args: [],
                };
            }))).filter(item => item !== null);
        }
        return this.format(toolName, context, {
            bodies,
            consoleData,
            consoleListData,
        });
    }
    format(toolName, context, data) {
        const response = [`# ${toolName} response`];
        for (const line of this.#textResponseLines) {
            response.push(line);
        }
        const networkConditions = context.getNetworkConditions();
        if (networkConditions) {
            response.push(`## Network emulation`);
            response.push(`Emulating: ${networkConditions}`);
            response.push(`Default navigation timeout set to ${context.getNavigationTimeout()} ms`);
        }
        const cpuThrottlingRate = context.getCpuThrottlingRate();
        if (cpuThrottlingRate > 1) {
            response.push(`## CPU emulation`);
            response.push(`Emulating: ${cpuThrottlingRate}x slowdown`);
        }
        if (this.#includePages) {
            const parts = [`## Pages`];
            let idx = 0;
            for (const page of context.getPages()) {
                parts.push(`${idx}: ${page.url()}${context.isPageSelected(page) ? ' [selected]' : ''}`);
                idx++;
            }
            response.push(...parts);
            // Show selected frame if not main frame
            const selectedFrame = context.getSelectedFrame();
            const mainFrame = context.getSelectedPage().mainFrame();
            if (selectedFrame !== mainFrame) {
                const name = selectedFrame.name() ? ` name="${selectedFrame.name()}"` : '';
                response.push(`## Selected Frame`);
                response.push(`${selectedFrame.url()}${name}`);
            }
        }
        response.push(...this.#formatNetworkRequestData(context, data.bodies));
        response.push(...this.#formatConsoleData(data.consoleData));
        if (this.#networkRequestsOptions?.include) {
            let requests = context.getNetworkRequests(this.#networkRequestsOptions?.includePreservedRequests);
            // Apply resource type filtering if specified
            if (this.#networkRequestsOptions.resourceTypes?.length) {
                const normalizedTypes = new Set(this.#networkRequestsOptions.resourceTypes);
                requests = requests.filter(request => {
                    const type = request.resourceType();
                    return normalizedTypes.has(type);
                });
            }
            // Apply URL filter if specified
            if (this.#networkRequestsOptions.urlFilter) {
                const filterPattern = this.#networkRequestsOptions.urlFilter.toLowerCase();
                requests = requests.filter(r => r.url().toLowerCase().includes(filterPattern));
            }
            // Show newest requests first
            requests.reverse();
            response.push('## Network requests');
            if (requests.length) {
                const data = this.#dataWithPagination(requests, this.#networkRequestsOptions.pagination ?? { pageSize: 20, pageIdx: 0 });
                response.push(...data.info);
                for (const request of data.items) {
                    response.push(getShortDescriptionForRequest(request, context.getNetworkRequestStableId(request), context.getNetworkRequestStableId(request) ===
                        this.#networkRequestsOptions?.networkRequestIdInDevToolsUI));
                }
            }
            else {
                response.push('No requests found.');
            }
        }
        if (this.#consoleDataOptions?.include) {
            const messages = data.consoleListData ?? [];
            response.push('## Console messages');
            if (messages.length) {
                const data = this.#dataWithPagination(messages, this.#consoleDataOptions.pagination);
                response.push(...data.info);
                response.push(...data.items.map(message => formatConsoleEventShort(message)));
            }
            else {
                response.push('<no console messages found>');
            }
        }
        // WebSocket connections list
        if (this.#webSocketOptions?.include) {
            let connections = context.getWebSocketConnections(this.#webSocketOptions.includePreservedConnections);
            // Apply URL filter if specified
            if (this.#webSocketOptions.urlFilter) {
                const filterPattern = this.#webSocketOptions.urlFilter.toLowerCase();
                connections = connections.filter(ws => ws.connection.url.toLowerCase().includes(filterPattern));
            }
            response.push('## WebSocket connections');
            if (connections.length) {
                const paginatedData = this.#dataWithPagination(connections, this.#webSocketOptions.pagination);
                response.push(...paginatedData.info);
                for (const ws of paginatedData.items) {
                    response.push(formatWebSocketConnectionShort(ws, context.getWebSocketStableId(ws)));
                }
                // 提示使用 analyze 工具
                response.push(``);
                response.push(`> 提示: 使用 \`analyze_websocket_messages(wsid=N)\` 分析消息模式后再查看具体内容`);
            }
            else {
                response.push('<no WebSocket connections found>');
            }
        }
        // Single WebSocket connection details
        if (this.#attachedWebSocketId !== undefined) {
            const ws = context.getWebSocketById(this.#attachedWebSocketId);
            response.push(...formatWebSocketConnectionVerbose(ws, this.#attachedWebSocketId));
        }
        const text = {
            type: 'text',
            text: response.join('\n'),
        };
        const images = this.#images.map(imageData => {
            return {
                type: 'image',
                ...imageData,
            };
        });
        return [text, ...images];
    }
    #dataWithPagination(data, pagination) {
        const response = [];
        const paginationResult = paginate(data, pagination);
        if (paginationResult.invalidPage) {
            response.push('Invalid page number provided. Showing first page.');
        }
        const { startIndex, endIndex, currentPage, totalPages } = paginationResult;
        response.push(`Showing ${startIndex + 1}-${endIndex} of ${data.length} (Page ${currentPage + 1} of ${totalPages}).`);
        if (pagination) {
            if (paginationResult.hasNextPage) {
                response.push(`Next page: ${currentPage + 1}`);
            }
            if (paginationResult.hasPreviousPage) {
                response.push(`Previous page: ${currentPage - 1}`);
            }
        }
        return {
            info: response,
            items: paginationResult.items,
        };
    }
    #formatConsoleData(data) {
        const response = [];
        if (!data) {
            return response;
        }
        response.push(formatConsoleEventVerbose(data));
        return response;
    }
    #formatNetworkRequestData(context, data) {
        const response = [];
        const id = this.#attachedNetworkRequestId;
        if (!id) {
            return response;
        }
        const httpRequest = context.getNetworkRequestById(id);
        response.push(`## Request ${httpRequest.url()}`);
        response.push(`Status:  ${getStatusFromRequest(httpRequest)}`);
        response.push(`### Request Headers`);
        for (const line of getFormattedHeaderValue(httpRequest.headers())) {
            response.push(line);
        }
        if (data.requestBody) {
            response.push(`### Request Body`);
            response.push(data.requestBody);
        }
        // Note: response headers are handled in the async path above
        // since request.response() is async in Playwright
        if (data.responseBody) {
            response.push(`### Response Body`);
            response.push(data.responseBody);
        }
        const failure = httpRequest.failure();
        if (failure) {
            response.push(`### Request failed with`);
            response.push(failure.errorText);
        }
        // In Playwright, there's no redirectChain() - use redirectedFrom() instead
        const redirectChain = [];
        let current = httpRequest.redirectedFrom();
        while (current) {
            redirectChain.push(current);
            current = current.redirectedFrom();
        }
        if (redirectChain.length) {
            response.push(`### Redirect chain`);
            let indent = 0;
            for (const request of redirectChain.reverse()) {
                response.push(`${'  '.repeat(indent)}${getShortDescriptionForRequest(request, context.getNetworkRequestStableId(request))}`);
                indent++;
            }
        }
        return response;
    }
    resetResponseLineForTesting() {
        this.#textResponseLines = [];
    }
}
