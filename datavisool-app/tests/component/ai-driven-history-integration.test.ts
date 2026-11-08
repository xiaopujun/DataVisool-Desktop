/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {beforeEach, describe, expect, test, vi} from "vitest";

const streamMocks = vi.hoisted(() => ({
    start: vi.fn(),
    stop: vi.fn(),
}));

vi.mock("@/designer/ai-driven/thread/AiDrivenThreadEventStream.ts", () => ({
    AiDrivenThreadEventStream: class {
        start = streamMocks.start;
        stop = streamMocks.stop;
    },
}));

import {AiDrivenHistoryIntegration} from "@/designer/ai-driven/AiDrivenHistoryIntegration.ts";

describe("AI driven history integration", () => {
    beforeEach(() => vi.clearAllMocks());

    test("starts the independent thread cursor after loading history snapshots", async () => {
        const historyStore = {
            projectId: "",
            threadId: "",
            threadEventCursor: 0,
            openThread: vi.fn(async function(this: typeof historyStore, projectId: string, threadId: string) {
                this.projectId = projectId;
                this.threadId = threadId;
                this.threadEventCursor = 12;
            }),
            applyEvent: vi.fn(),
            clear: vi.fn(),
        };
        const integration = new AiDrivenHistoryIntegration(historyStore as never);

        await integration.open("project-a", "thread-a");

        expect(historyStore.openThread).toHaveBeenCalledWith("project-a", "thread-a");
        expect(streamMocks.start).toHaveBeenCalledWith({
            projectId: "project-a",
            threadId: "thread-a",
            eventCursor: 12,
        }, expect.objectContaining({onEvent: historyStore.applyEvent}));

        integration.close();
        expect(streamMocks.stop).toHaveBeenCalled();
    });
});
