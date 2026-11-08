/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {tx} from "@/i18n/miniI18n.ts";

export function formatAiModelTime(value?: string): string {
    if (!value) return tx("暂无时间");

    // SQLite CURRENT_TIMESTAMP is UTC, but its textual form has no timezone suffix.
    const sqliteTimestamp = /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d+)?$/;
    const date = new Date(sqliteTimestamp.test(value) ? `${value.replace(" ", "T")}Z` : value);
    if (Number.isNaN(date.getTime())) return value;
    const month = String(date.getMonth() + 1).padStart(2, "0");
    const day = String(date.getDate()).padStart(2, "0");
    const hour = String(date.getHours()).padStart(2, "0");
    const minute = String(date.getMinutes()).padStart(2, "0");
    return `${month}/${day} ${hour}:${minute}`;
}
