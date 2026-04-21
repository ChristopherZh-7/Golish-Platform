/**
 * @license
 * Copyright 2025 Google LLC
 * SPDX-License-Identifier: Apache-2.0
 */
export var ToolCategory;
(function (ToolCategory) {
    ToolCategory["NAVIGATION"] = "navigation";
    ToolCategory["NETWORK"] = "network";
    ToolCategory["DEBUGGING"] = "debugging";
    ToolCategory["REVERSE_ENGINEERING"] = "reverse_engineering";
})(ToolCategory || (ToolCategory = {}));
export const labels = {
    [ToolCategory.NAVIGATION]: 'Navigation automation',
    [ToolCategory.NETWORK]: 'Network',
    [ToolCategory.DEBUGGING]: 'Debugging',
    [ToolCategory.REVERSE_ENGINEERING]: 'JS Reverse Engineering',
};
