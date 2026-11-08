/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {beforeEach, describe, expect, test, vi} from "vitest";
import {AiDrivenThreadStore, type AiDrivenThreadStoreApi} from "@/designer/ai-driven/thread/AiDrivenThreadStore.ts";
import type {
    IAiDrivenThread,
    IAiDrivenThreadSummary,
} from "@/designer/ai-driven/thread/AiDrivenThreadTypes.ts";

const thread = (overrides: Partial<IAiDrivenThread> = {}): IAiDrivenThread => ({
    threadId: "thread-a",
    projectId: "project-a",
    title: "生成销售大屏",
    titleSource: "auto",
    preview: "已完成",
    status: "active",
    createRequestId: "create-a",
    rowVersion: 1,
    createdAt: 1,
    updatedAt: 1,
    lastActivityAt: 1,
    archivedAt: null,
    ...overrides,
});

const summary = (overrides: Partial<IAiDrivenThreadSummary> = {}): IAiDrivenThreadSummary => ({
    ...thread(overrides),
    runtimeStatus: "idle",
    activeTurnId: null,
    currentRunId: null,
    ...overrides,
});

const api = (): AiDrivenThreadStoreApi => ({
    create: vi.fn(),
    list: vi.fn(async () => ({data: [], nextCursor: null})),
    read: vi.fn(),
    update: vi.fn(),
    archive: vi.fn(),
});

const storageValues = new Map<string, string>();
Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: {
        clear: () => storageValues.clear(),
        getItem: (key: string) => storageValues.get(key) ?? null,
        removeItem: (key: string) => storageValues.delete(key),
        setItem: (key: string, value: string) => storageValues.set(key, value),
    },
});

describe("AI driven thread store", () => {
    beforeEach(() => localStorage.clear());

    test("reuses the same clientRequestId when thread creation is retried", async () => {
        const mockApi = api();
        vi.mocked(mockApi.create)
            .mockResolvedValueOnce(null)
            .mockResolvedValueOnce(thread());
        const store = new AiDrivenThreadStore(mockApi);
        await store.initialize("project-a");

        expect(await store.ensureThread()).toBeNull();
        const created = await store.ensureThread();

        expect(created?.threadId).toBe("thread-a");
        expect(mockApi.create).toHaveBeenCalledTimes(2);
        expect(vi.mocked(mockApi.create).mock.calls[0][0].clientRequestId)
            .toBe(vi.mocked(mockApi.create).mock.calls[1][0].clientRequestId);
        expect(store.selectedThreadId).toBe("thread-a");
    });

    test("restores the last selected thread within the project", async () => {
        localStorage.setItem("AI_DRIVEN_SELECTED_THREAD_V1:project-a", "thread-b");
        const mockApi = api();
        vi.mocked(mockApi.list).mockResolvedValue({
            data: [summary(), summary({threadId: "thread-b", title: "历史对话"})],
            nextCursor: null,
        });
        const store = new AiDrivenThreadStore(mockApi);

        await store.initialize("project-a");

        expect(store.selectedThreadId).toBe("thread-b");
        expect(store.selectedThread?.title).toBe("历史对话");
        expect(mockApi.read).not.toHaveBeenCalled();
    });

    test("uses rowVersion for rename and archive mutations", async () => {
        const mockApi = api();
        vi.mocked(mockApi.list).mockResolvedValue({data: [summary()], nextCursor: null});
        vi.mocked(mockApi.update).mockResolvedValue(thread({
            title: "新标题",
            titleSource: "user",
            rowVersion: 2,
        }));
        vi.mocked(mockApi.archive).mockResolvedValue(thread({
            title: "新标题",
            titleSource: "user",
            status: "archived",
            rowVersion: 3,
            archivedAt: 3,
        }));
        const store = new AiDrivenThreadStore(mockApi);
        await store.initialize("project-a");

        await store.renameThread("thread-a", " 新标题 ");
        await store.setThreadArchived("thread-a", true);

        expect(mockApi.update).toHaveBeenCalledWith(expect.objectContaining({
            threadId: "thread-a",
            title: "新标题",
            expectedVersion: 1,
        }), expect.anything());
        expect(mockApi.archive).toHaveBeenCalledWith(expect.objectContaining({
            threadId: "thread-a",
            archived: true,
            expectedVersion: 2,
        }), expect.anything());
        expect(store.selectedThread?.status).toBe("archived");
        expect(store.threads).toHaveLength(0);
    });
});
