/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {beforeEach, describe, expect, test, vi} from "vitest";
import {listen} from "@tauri-apps/api/event";
import type {IAiDrivenThreadEvent} from "@/designer/ai-driven/thread/AiDrivenThreadTypes.ts";

const mocks = vi.hoisted(() => ({
    getEvents: vi.fn(),
    listener: undefined as ((event: {payload: IAiDrivenThreadEvent}) => void) | undefined,
    unlisten: vi.fn(),
}));

vi.mock("@/api/AiDrivenThreadApi.ts", () => ({
    getAiDrivenThreadEvents: mocks.getEvents,
}));

vi.mock("@tauri-apps/api/event", () => ({
    listen: vi.fn(async (_channel: string, listener: (event: {payload: IAiDrivenThreadEvent}) => void) => {
        mocks.listener = listener;
        return mocks.unlisten;
    }),
}));

import {AiDrivenThreadEventStream} from "@/designer/ai-driven/thread/AiDrivenThreadEventStream.ts";

const event = (threadSequence: number, runId: string): IAiDrivenThreadEvent => ({
    eventId: `event-${threadSequence}`,
    threadId: "thread-a",
    turnId: `turn-${runId}`,
    itemId: null,
    runId,
    sequence: threadSequence,
    threadSequence,
    kind: "turn.started",
    payload: {},
    dedupeKey: `dedupe-${threadSequence}`,
    schemaVersion: 1,
    createdAt: threadSequence,
});

describe("AI driven thread event stream", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        mocks.listener = undefined;
    });

    test("advances thread cursor across events from different runs", async () => {
        let resolveReplay!: (value: {data: IAiDrivenThreadEvent[]; nextCursor: number | null}) => void;
        mocks.getEvents.mockReturnValue(new Promise((resolve) => resolveReplay = resolve));
        const stream = new AiDrivenThreadEventStream();
        const received: Array<[number, string]> = [];
        const cursors: number[] = [];

        const started = stream.start({
            projectId: "project-a",
            threadId: "thread-a",
            eventCursor: 0,
        }, {
            onEvent: (value) => received.push([value.threadSequence, value.runId]),
            onCursor: (cursor) => cursors.push(cursor),
        });
        await vi.waitFor(() => expect(mocks.listener).toBeTypeOf("function"));
        expect(listen).toHaveBeenCalledWith("datavisool:ai-driven-task-event", expect.any(Function));
        mocks.listener?.({payload: event(3, "run-a")});
        resolveReplay({
            data: [event(1, "run-a"), event(2, "run-b"), event(3, "run-a")],
            nextCursor: 3,
        });
        await started;

        expect(received).toEqual([[1, "run-a"], [2, "run-b"], [3, "run-a"]]);
        expect(cursors).toEqual([1, 2, 3]);
        stream.stop();
        expect(mocks.unlisten).toHaveBeenCalledTimes(1);
    });
});
