/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import AuthFetchUtil from "@/util/AuthFetchUtil.ts";

export type DashboardRecord = {
    id?: string | number;
    name?: string;
    componentName?: string;
    createTime?: string | null;
    updateTime?: string | null;
};

export type DashboardPage<T extends DashboardRecord> = {
    total?: number;
    records?: T[];
};

export type DashboardCollection<T extends DashboardRecord> = {
    total: number;
    records: T[];
};

const PAGE_SIZE = 50;
const SQLITE_TIMESTAMP = /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d+)?$/;

export async function loadDashboardCollection<T extends DashboardRecord>(url: string): Promise<DashboardCollection<T>> {
    const records: T[] = [];
    let total = 0;
    let current = 1;
    do {
        const page = await AuthFetchUtil.post<DashboardPage<T> | null>(url, {current, size: PAGE_SIZE});
        if (!page || !Array.isArray(page.records)) throw new Error(`Failed to load dashboard page: ${url}`);
        if (current === 1) {
            total = typeof page.total === "number" && Number.isFinite(page.total)
                ? Math.max(0, Math.floor(page.total))
                : page.records.length;
        }
        if (page.records.length === 0 && records.length < total) {
            throw new Error(`Incomplete dashboard collection: ${url}`);
        }
        records.push(...page.records);
        current += 1;
    } while (records.length < total);
    return {total, records};
}

export function parseDashboardTimestamp(value?: string | null): number | null {
    if (!value?.trim()) return null;
    const text = value.trim();
    // SQLite CURRENT_TIMESTAMP has no timezone suffix, but represents UTC.
    const timestamp = new Date(SQLITE_TIMESTAMP.test(text) ? `${text.replace(" ", "T")}Z` : text).getTime();
    return Number.isFinite(timestamp) ? timestamp : null;
}

export function getRecordTimestamp(record: DashboardRecord): number | null {
    return parseDashboardTimestamp(record.updateTime) ?? parseDashboardTimestamp(record.createTime);
}

export function formatDashboardTimestamp(timestamp: number | null): string {
    if (timestamp === null) return "-";
    const date = new Date(timestamp);
    const hours = `${date.getHours()}`.padStart(2, "0");
    const minutes = `${date.getMinutes()}`.padStart(2, "0");
    return `${date.getFullYear()}/${date.getMonth() + 1}/${date.getDate()} ${hours}:${minutes}`;
}
