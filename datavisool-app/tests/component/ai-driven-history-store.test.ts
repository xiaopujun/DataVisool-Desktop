/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {describe, expect, test, vi} from "vitest";
import {AiDrivenHistoryStore, type AiDrivenHistoryStoreApi} from "@/designer/ai-driven/thread/AiDrivenHistoryStore.ts";
import type {
    IAiDrivenItem,
    IAiDrivenThreadEvent,
    IAiDrivenTurn,
    IAiDrivenTurnListResult,
} from "@/designer/ai-driven/thread/AiDrivenThreadTypes.ts";

const deferred = <T,>() => {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>((done) => resolve = done);
    return {promise, resolve};
};

const turn = (threadId: string, turnId: string): IAiDrivenTurn => ({
    turnId,
    projectId: "project-a",
    threadId,
    turnSequence: 1,
    requestId: `request-${turnId}`,
    status: "running",
    request: {},
    inputSummary: "生成页面",
    modelConfigId: "model-config",
    runtimeModelId: "model",
    provider: null,
    entryDigest: "digest",
    usage: null,
    lastArtifactId: null,
    lastArtifactRevision: null,
    errorCode: null,
    errorMessage: null,
    rowVersion: 1,
    createdAt: 1,
    startedAt: 1,
    updatedAt: 1,
    endedAt: null,
});

const item = (): IAiDrivenItem => ({
    itemId: "item-a",
    turnId: "turn-b",
    itemSequence: 1,
    sourceRunId: "run-b",
    logicalKey: null,
    itemType: "agentMessage",
    status: "inProgress",
    schemaVersion: 1,
    payload: {text: "正在"},
    lastEventSequence: 1,
    rowVersion: 1,
    createdAt: 1,
    updatedAt: 1,
    completedAt: null,
});

const event = (
    threadSequence: number,
    kind: string,
    payload: Record<string, unknown>,
): IAiDrivenThreadEvent => ({
    eventId: `event-${threadSequence}`,
    threadId: "thread-b",
    turnId: "turn-b",
    itemId: "item-a",
    runId: "run-b",
    sequence: threadSequence,
    threadSequence,
    kind,
    payload,
    dedupeKey: `dedupe-${threadSequence}`,
    schemaVersion: 1,
    createdAt: threadSequence,
});

describe("AI driven history store", () => {
    test("aborts and ignores an old thread response", async () => {
        const first = deferred<IAiDrivenTurnListResult | null>();
        const second = deferred<IAiDrivenTurnListResult | null>();
        const signals: AbortSignal[] = [];
        const mockApi: AiDrivenHistoryStoreApi = {
            listTurns: vi.fn((payload, options) => {
                signals.push(options!.signal!);
                return payload.threadId === "thread-a" ? first.promise : second.promise;
            }),
            readTurn: vi.fn(),
            listItems: vi.fn(),
        };
        const store = new AiDrivenHistoryStore(mockApi);

        const openingFirst = store.openThread("project-a", "thread-a");
        const openingSecond = store.openThread("project-a", "thread-b");
        second.resolve({data: [turn("thread-b", "turn-b")], nextCursor: null});
        await openingSecond;
        first.resolve({data: [turn("thread-a", "turn-a")], nextCursor: null});
        await openingFirst;

        expect(signals[0].aborted).toBe(true);
        expect(store.threadId).toBe("thread-b");
        expect(store.turns.map((value) => value.turnId)).toEqual(["turn-b"]);
    });

    test("merges item delta and completed snapshot by threadSequence", async () => {
        const completed = {...item(), status: "completed" as const, payload: {text: "正在分析需求"}, lastEventSequence: 3, completedAt: 3};
        const mockApi: AiDrivenHistoryStoreApi = {
            listTurns: vi.fn(async () => ({data: [turn("thread-b", "turn-b")], nextCursor: null})),
            readTurn: vi.fn(),
            listItems: vi.fn(async () => ({data: [item()], nextCursor: null})),
        };
        const store = new AiDrivenHistoryStore(mockApi);
        await store.openThread("project-a", "thread-b");
        await store.loadItems("turn-b", true);

        store.applyEvent(event(2, "item.delta", {itemType: "agentMessage", delta: "分析"}));
        store.applyEvent(event(3, "item.completed", {item: completed}));
        store.applyEvent(event(2, "item.delta", {itemType: "agentMessage", delta: "重复"}));

        expect(store.itemsByTurn["turn-b"][0].payload.text).toBe("正在分析需求");
        expect(store.itemsByTurn["turn-b"][0].status).toBe("completed");
        expect(store.threadEventCursor).toBe(3);
    });

    test("still loads persisted items when live events arrive before first expansion", async () => {
        const liveItem = {...item(), itemSequence: 2, payload: {text: "实时回复"}};
        const userItem = {
            ...item(),
            itemId: "item-user",
            itemSequence: 1,
            itemType: "userMessage",
            status: "completed" as const,
            payload: {text: "生成页面"},
            completedAt: 1,
        };
        const mockApi: AiDrivenHistoryStoreApi = {
            listTurns: vi.fn(async () => ({data: [turn("thread-b", "turn-b")], nextCursor: null})),
            readTurn: vi.fn(),
            listItems: vi.fn(async () => ({data: [userItem], nextCursor: null})),
        };
        const store = new AiDrivenHistoryStore(mockApi);
        await store.openThread("project-a", "thread-b");

        store.applyEvent(event(1, "item.started", {item: liveItem}));
        expect(store.loadedTurnIds.has("turn-b")).toBe(false);

        store.toggleTurn("turn-b");
        await vi.waitFor(() => expect(store.loadedTurnIds.has("turn-b")).toBe(true));

        expect(mockApi.listItems).toHaveBeenCalledTimes(1);
        expect(store.itemsByTurn["turn-b"].map((value) => value.itemId)).toEqual([
            "item-user",
            "item-a",
        ]);
    });

    test("projects operation events as durable history facts", async () => {
        const mockApi: AiDrivenHistoryStoreApi = {
            listTurns: vi.fn(async () => ({data: [turn("thread-b", "turn-b")], nextCursor: null})),
            readTurn: vi.fn(),
            listItems: vi.fn(async () => ({data: [], nextCursor: null})),
        };
        const store = new AiDrivenHistoryStore(mockApi);
        await store.openThread("project-a", "thread-b");
        const operation = {
            operationId: "run-b:call-1",
            toolCallId: "call-1",
            kind: "createComponents",
            status: "applying",
            projectId: "project-a",
            componentIds: ["draft-1"],
        };

        store.applyEvent(event(2, "agent.canvas_operation.applying", {operation}));
        store.applyEvent(event(3, "agent.canvas_operation.applied", {
            operation: {...operation, status: "applied", componentIds: ["component-1"], resultRevision: "sha256:result"},
        }));

        expect(store.itemsByTurn["turn-b"]).toEqual([expect.objectContaining({
            itemType: "canvasOperation",
            status: "completed",
            payload: expect.objectContaining({resultRevision: "sha256:result"}),
        })]);
    });

    test("updates the turn status from waiting and resumed events", async () => {
        const mockApi: AiDrivenHistoryStoreApi = {
            listTurns: vi.fn(async () => ({data: [turn("thread-b", "turn-b")], nextCursor: null})),
            readTurn: vi.fn(),
            listItems: vi.fn(),
        };
        const store = new AiDrivenHistoryStore(mockApi);
        await store.openThread("project-a", "thread-b");

        store.applyEvent(event(2, "turn.waiting", {status: "waitingUserConfirmation"}));
        expect(store.turns[0].status).toBe("waitingUserConfirmation");

        store.applyEvent(event(3, "turn.resumed", {status: "running"}));
        expect(store.turns[0].status).toBe("running");
    });

    test("loads a newly created Turn that was not in the initial page", async () => {
        const missing = {...turn("thread-b", "turn-new"), turnSequence: 2, status: "completed" as const};
        const mockApi: AiDrivenHistoryStoreApi = {
            listTurns: vi.fn(async () => ({data: [turn("thread-b", "turn-old")], nextCursor: null})),
            readTurn: vi.fn(async () => missing),
            listItems: vi.fn(),
        };
        const store = new AiDrivenHistoryStore(mockApi);
        await store.openThread("project-a", "thread-b");

        store.applyEvent({...event(2, "turn.completed", {status: "completed"}), turnId: "turn-new"});
        await vi.waitFor(() => expect(store.turns.some((value) => value.turnId === "turn-new")).toBe(true));

        expect(mockApi.readTurn).toHaveBeenCalledWith({
            projectId: "project-a",
            threadId: "thread-b",
            turnId: "turn-new",
        }, {suppressErrorMessage: true});
    });

    test("coalesces missing Turn reads and keeps the latest live status", async () => {
        const pending = deferred<IAiDrivenTurn | null>();
        const mockApi: AiDrivenHistoryStoreApi = {
            listTurns: vi.fn(async () => ({data: [turn("thread-b", "turn-old")], nextCursor: null})),
            readTurn: vi.fn(() => pending.promise),
            listItems: vi.fn(),
        };
        const store = new AiDrivenHistoryStore(mockApi);
        await store.openThread("project-a", "thread-b");

        store.applyEvent({...event(2, "turn.started", {status: "running"}), turnId: "turn-new"});
        store.applyEvent({...event(3, "turn.waiting", {status: "waitingUserConfirmation"}), turnId: "turn-new"});
        pending.resolve({...turn("thread-b", "turn-new"), turnSequence: 2});
        await vi.waitFor(() => expect(store.turns.some((value) => value.turnId === "turn-new")).toBe(true));

        expect(mockApi.readTurn).toHaveBeenCalledTimes(1);
        expect(store.turns.find((value) => value.turnId === "turn-new")?.status).toBe("waitingUserConfirmation");
    });

    test("reloads the Turn page when a missing Turn read fails", async () => {
        const listTurns = vi.fn()
            .mockResolvedValueOnce({data: [turn("thread-b", "turn-old")], nextCursor: null})
            .mockResolvedValueOnce({data: [{...turn("thread-b", "turn-new"), turnSequence: 2}], nextCursor: null});
        const mockApi: AiDrivenHistoryStoreApi = {
            listTurns,
            readTurn: vi.fn(async () => null),
            listItems: vi.fn(),
        };
        const store = new AiDrivenHistoryStore(mockApi);
        await store.openThread("project-a", "thread-b");

        store.applyEvent({...event(2, "turn.created", {status: "created"}), turnId: "turn-new"});
        await vi.waitFor(() => expect(store.turns.some((value) => value.turnId === "turn-new")).toBe(true));

        expect(listTurns).toHaveBeenCalledTimes(2);
    });

    test("retries unresolved missing Turns when reopening the same Thread", async () => {
        const listTurns = vi.fn()
            .mockResolvedValueOnce({data: [turn("thread-b", "turn-old")], nextCursor: null})
            .mockResolvedValueOnce(null)
            .mockResolvedValueOnce({data: [{...turn("thread-b", "turn-new"), turnSequence: 2}], nextCursor: null});
        const mockApi: AiDrivenHistoryStoreApi = {
            listTurns,
            readTurn: vi.fn(async () => null),
            listItems: vi.fn(),
        };
        const store = new AiDrivenHistoryStore(mockApi);
        await store.openThread("project-a", "thread-b");

        store.applyEvent({...event(2, "turn.created", {status: "created"}), turnId: "turn-new"});
        await vi.waitFor(() => expect(listTurns).toHaveBeenCalledTimes(2));
        await store.openThread("project-a", "thread-b");

        expect(store.turns.some((value) => value.turnId === "turn-new")).toBe(true);
        expect(listTurns).toHaveBeenCalledTimes(3);
    });

    test("applies the latest live status over a deferred fallback page", async () => {
        const fallback = deferred<IAiDrivenTurnListResult | null>();
        const listTurns = vi.fn()
            .mockResolvedValueOnce({data: [turn("thread-b", "turn-old")], nextCursor: null})
            .mockImplementationOnce(() => fallback.promise);
        const mockApi: AiDrivenHistoryStoreApi = {
            listTurns,
            readTurn: vi.fn(async () => null),
            listItems: vi.fn(),
        };
        const store = new AiDrivenHistoryStore(mockApi);
        await store.openThread("project-a", "thread-b");

        store.applyEvent({...event(2, "turn.started", {status: "running"}), turnId: "turn-new"});
        await vi.waitFor(() => expect(listTurns).toHaveBeenCalledTimes(2));
        store.applyEvent({...event(3, "turn.waiting", {status: "waitingUserConfirmation"}), turnId: "turn-new"});
        fallback.resolve({data: [{...turn("thread-b", "turn-new"), turnSequence: 2}], nextCursor: null});
        await vi.waitFor(() => expect(store.turns.some((value) => value.turnId === "turn-new")).toBe(true));

        expect(store.turns.find((value) => value.turnId === "turn-new")?.status).toBe("waitingUserConfirmation");
        expect(mockApi.readTurn).toHaveBeenCalledTimes(1);
    });
});
