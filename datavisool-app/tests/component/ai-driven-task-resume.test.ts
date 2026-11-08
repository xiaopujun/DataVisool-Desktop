/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {beforeEach, describe, expect, test, vi} from "vitest";

const mocks = vi.hoisted(() => ({
    resume: vi.fn(),
    appendLog: vi.fn(),
    setRuntimeSnapshot: vi.fn(),
    finishRun: vi.fn(),
    warning: vi.fn(),
    error: vi.fn(),
}));

vi.mock("@/api/AiDrivenTaskApi.ts", () => ({resumeAiDrivenTask: mocks.resume}));
vi.mock("@/designer/ai-driven/AiDrivenProgressTerminalStore.ts", () => ({
    default: {
        appendLog: mocks.appendLog,
        setRuntimeSnapshot: mocks.setRuntimeSnapshot,
        finishRun: mocks.finishRun,
    },
}));
vi.mock("@/i18n/miniI18n.ts", () => ({tx: (message: string) => message}));
vi.mock("@datavisool/ui", () => ({
    globalMessage: {messageApi: {warning: mocks.warning, error: mocks.error}},
}));

import {resumeInterruptedTask} from "@/designer/ai-driven/AiDrivenTaskResumeController.ts";
import type {IAiDrivenTurn} from "@/designer/ai-driven/thread/AiDrivenThreadTypes.ts";

const turn: IAiDrivenTurn = {
    turnId: "turn-a",
    projectId: "project-a",
    threadId: "thread-a",
    turnSequence: 1,
    requestId: "request-a",
    status: "interrupted",
    request: {},
    inputSummary: "创建图表",
    modelConfigId: "model-config",
    runtimeModelId: "runtime-model",
    provider: "openai",
    entryDigest: "sha256:a",
    usage: null,
    lastArtifactId: null,
    lastArtifactRevision: null,
    errorCode: "agent.restart_interrupted",
    errorMessage: "应用重启",
    rowVersion: 1,
    createdAt: 1,
    startedAt: 2,
    updatedAt: 3,
    endedAt: 3,
};

describe("AI driven interrupted task resume", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        localStorage.clear();
    });

    test("creates a new active scope and starts monitoring the resumed attempt", async () => {
        const runtime = {usage: {inputTokens: 0, outputTokens: 0, totalTokens: 0}, operations: []};
        mocks.resume.mockResolvedValue({
            taskId: "task-b",
            runId: "run-b",
            threadId: "thread-a",
            turnId: "turn-a",
            eventCursor: 8,
            requestId: "request-a",
            projectId: "project-a",
            clientSessionId: "session-a",
            status: "running",
            createdAt: 4,
            recoverable: false,
            runtime,
        });
        const host = {
            running: false,
            mounted: true,
            currentRequestId: "request-a",
            aiDrivenSessionId: "session-a",
            taskRunMonitor: {stop: vi.fn(), watch: vi.fn()},
            startRunningTask: vi.fn(),
            acceptRunningTask: vi.fn(),
            finishRunningTask: vi.fn(),
        };

        await resumeInterruptedTask(host, "project-a", turn);

        expect(mocks.resume).toHaveBeenCalledWith({
            projectId: "project-a",
            threadId: "thread-a",
            turnId: "turn-a",
            sessionId: "session-a",
        }, {suppressErrorMessage: true});
        expect(host.acceptRunningTask).toHaveBeenCalledWith("request-a", "thread-a", "turn-a", "run-b");
        expect(host.taskRunMonitor.watch).toHaveBeenCalledWith(expect.objectContaining({runId: "run-b", eventCursor: 8}));
        expect(JSON.parse(localStorage.getItem("AI_DRIVEN_ACTIVE_TASK_V1:project-a") ?? "{}")).toMatchObject({runId: "run-b"});
        expect(mocks.setRuntimeSnapshot).toHaveBeenCalledWith(runtime);
    });

    test("does not resume a budget exhausted turn without an explicit extension", async () => {
        const host = {
            running: false,
            mounted: true,
            currentRequestId: "request-a",
            aiDrivenSessionId: "session-a",
            taskRunMonitor: {stop: vi.fn(), watch: vi.fn()},
            startRunningTask: vi.fn(),
            acceptRunningTask: vi.fn(),
            finishRunningTask: vi.fn(),
        };

        await resumeInterruptedTask(host, "project-a", {...turn, status: "budgetExhausted"});

        expect(mocks.resume).not.toHaveBeenCalled();
        expect(mocks.warning).toHaveBeenCalledWith("请先明确增加至少一项预算后继续");
        expect(host.startRunningTask).not.toHaveBeenCalled();
    });

    test("sends explicit cumulative budget extension when continuing", async () => {
        mocks.resume.mockResolvedValue({
            taskId: "task-c",
            runId: "run-c",
            threadId: "thread-a",
            turnId: "turn-a",
            eventCursor: 10,
            requestId: "request-a",
            projectId: "project-a",
            clientSessionId: "session-a",
            status: "running",
            createdAt: 4,
            recoverable: false,
            runtime: {usage: {inputTokens: 0, outputTokens: 0, totalTokens: 0}, operations: []},
        });
        const host = {
            running: false,
            mounted: true,
            currentRequestId: "request-a",
            aiDrivenSessionId: "session-a",
            taskRunMonitor: {stop: vi.fn(), watch: vi.fn()},
            startRunningTask: vi.fn(),
            acceptRunningTask: vi.fn(),
            finishRunningTask: vi.fn(),
        };
        const extension = {maxTotalTokens: 5000, maxDurationMs: 600_000};

        await resumeInterruptedTask(host, "project-a", {
            ...turn,
            status: "budgetExhausted",
            checkpointVersion: 7,
        }, extension);

        expect(mocks.resume).toHaveBeenCalledWith({
            projectId: "project-a",
            threadId: "thread-a",
            turnId: "turn-a",
            sessionId: "session-a",
            expectedCheckpointVersion: 7,
            budgetExtension: extension,
        }, {suppressErrorMessage: true});
        expect(host.acceptRunningTask).toHaveBeenCalledWith("request-a", "thread-a", "turn-a", "run-c");
    });
});
