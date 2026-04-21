/**
 * @license
 * Copyright 2025 Google LLC
 * SPDX-License-Identifier: Apache-2.0
 */
// The short format for a console message, based on a previous format.
export function formatConsoleEventShort(msg) {
    if (msg.type === 'issue') {
        return `msgid=${msg.consoleMessageStableId} [${msg.type}] ${msg.message} (count: ${msg.count})`;
    }
    return `msgid=${msg.consoleMessageStableId} [${msg.type}] ${msg.message} (${msg.args?.length ?? 0} args)`;
}
function getArgs(msg) {
    const args = [...(msg.args ?? [])];
    // If there is no text, the first argument serves as text (see formatMessage).
    if (!msg.message) {
        args.shift();
    }
    return args;
}
// The verbose format for a console message, including all details.
export function formatConsoleEventVerbose(msg) {
    const aggregatedIssue = msg.item;
    const result = [
        `ID: ${msg.consoleMessageStableId}`,
        `Message: ${msg.type}> ${aggregatedIssue ? formatIssue(aggregatedIssue, msg.description) : msg.message}`,
        aggregatedIssue ? undefined : formatArgs(msg),
    ].filter(line => !!line);
    return result.join('\n');
}
function formatArg(arg) {
    return typeof arg === 'object' ? JSON.stringify(arg) : String(arg);
}
function formatArgs(consoleData) {
    const args = getArgs(consoleData);
    if (!args.length) {
        return '';
    }
    const result = ['### Arguments'];
    for (const [key, arg] of args.entries()) {
        result.push(`Arg #${key}: ${formatArg(arg)}`);
    }
    return result.join('\n');
}
export function formatIssue(issue, description) {
    const result = [];
    let processedMarkdown = description?.trim();
    // Remove heading in order not to conflict with the whole console message response markdown
    if (processedMarkdown?.startsWith('# ')) {
        processedMarkdown = processedMarkdown.substring(2).trimStart();
    }
    if (processedMarkdown)
        result.push(processedMarkdown);
    const links = issue.getDescription()?.links;
    if (links && links.length > 0) {
        result.push('Learn more:');
        for (const link of links) {
            result.push(`[${link.linkTitle}](${link.link})`);
        }
    }
    if (result.length === 0)
        return 'No details provided for the issue ' + issue.code();
    return result.join('\n');
}
