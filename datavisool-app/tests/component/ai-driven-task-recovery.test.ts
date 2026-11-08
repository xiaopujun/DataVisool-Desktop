/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, beforeEach, describe, expect, test, vi} from "vitest";
import type {
    AiDrivenTaskSubmitStatus,
    IAiDrivenTaskSubmitResult,
} from "@/designer/ai-driven/types/AiDrivenTaskSubmitTypes.ts";

const mocks = vi.hoisted(() => ({
    projectId: "project-a",
    getTaskStatus: vi.fn(),
    getTaskEvents: vi.fn(),
    cancelTask: vi.fn(),
    steerTask: vi.fn(),
    submitTask: vi.fn(),
    createThread: vi.fn(),
    listThreads: vi.fn(),
    readThread: vi.fn(),
    buildSubmitPayload: vi.fn(),
    resolveShortcut: vi.fn(),
    appendLog: vi.fn(),
    applyTaskEvent: vi.fn(),
    startRun: vi.fn(),
    finishRun: vi.fn(),
    setCanvasOperations: vi.fn(),
    setTokenUsage: vi.fn(),
    setRuntimeSnapshot: vi.fn(),
    messageError: vi.fn(),
    taskEventListener: undefined as ((event: {payload: unknown}) => void) | undefined,
}));

vi.mock("@/api/AiDrivenTaskApi.ts", () => ({
    getAiDrivenTaskStatus: mocks.getTaskStatus,
    getAiDrivenTaskEvents: mocks.getTaskEvents,
    cancelAiDrivenTask: mocks.cancelTask,
    steerAiDrivenTask: mocks.steerTask,
    submitAiDrivenTask: mocks.submitTask,
}));

vi.mock("@/api/AiDrivenThreadApi.ts", () => ({
    createAiDrivenThread: mocks.createThread,
    listAiDrivenThreads: mocks.listThreads,
    readAiDrivenThread: mocks.readThread,
    updateAiDrivenThread: vi.fn(),
    archiveAiDrivenThread: vi.fn(),
    getAiDrivenThreadEvents: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
    listen: vi.fn(async (_channel, callback) => {
        mocks.taskEventListener = callback;
        return vi.fn();
    }),
}));

vi.mock("@/api/AiDrivenAttachmentApi.ts", () => ({
    uploadAiDrivenImageAttachment: vi.fn(),
}));

vi.mock("@/api/AiModelApi.ts", () => ({
    getAiDrivenProviderModelProviders: vi.fn(),
    getAiModelConfiguredList: vi.fn(),
}));

vi.mock("@/designer/ai-driven/AiDrivenUtil.ts", () => ({
    MAX_IMAGE_ATTACHMENTS: 3,
    buildAiDrivenModelConfigOptions: vi.fn(() => []),
    buildAiDrivenRuntimeModelOptions: vi.fn(() => []),
    buildAiDrivenTaskSubmitPayload: mocks.buildSubmitPayload,
    createAiDrivenSessionId: vi.fn(() => "new-session"),
    createAiDrivenSteerInputId: vi.fn(() => "steer-input"),
    createAiDrivenSubmitRequestId: vi.fn(() => "new-request"),
    createAiDrivenUploadRequestId: vi.fn(() => "upload-request"),
    fileToDataUrl: vi.fn(),
    getAiDrivenDefaultRuntimeModelId: vi.fn(() => ""),
    getAiDrivenProviderModelConfig: vi.fn(),
    getAiDrivenSelectedModelProvider: vi.fn(() => ""),
    getCurrentProjectId: () => mocks.projectId,
    getPastedImageFileName: vi.fn(),
}));

vi.mock("@/designer/ai-driven/AiDrivenShortcutCommands.ts", () => ({
    resolveAiDrivenShortcutSubmission: mocks.resolveShortcut,
}));

vi.mock("@/designer/ai-driven/AiDrivenProgressTerminalStore.ts", () => ({
    default: {
        appendLog: mocks.appendLog,
        applyTaskEvent: mocks.applyTaskEvent,
        startRun: mocks.startRun,
        finishRun: mocks.finishRun,
        setCanvasOperations: mocks.setCanvasOperations,
        setTokenUsage: mocks.setTokenUsage,
        setRuntimeSnapshot: mocks.setRuntimeSnapshot,
        clearLiveItems: vi.fn(),
        setVisible: vi.fn(),
    },
}));

vi.mock("@/i18n/miniI18n.ts", () => ({
    tx: (message: string) => message,
}));

vi.mock("@datavisool/ui", () => ({
    globalMessage: {
        messageApi: {
            error: mocks.messageError,
            success: vi.fn(),
            warning: vi.fn(),
        },
    },
}));

import AiDrivenBarStore from "@/designer/ai-driven/AiDrivenBarStore.ts";

const ACTIVE_TASK_STORAGE_KEY = "AI_DRIVEN_ACTIVE_TASK_V1:project-a";
const PENDING_TASK_STORAGE_KEY = "AI_DRIVEN_PENDING_TASK_V1:project-a";
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
const scope = {
    requestId: "request-a",
    projectId: "project-a",
    threadId: "thread-a",
    turnId: "turn-a",
    runId: "run-a",
    eventCursor: 0,
};

const thread = {
    threadId: "thread-a",
    projectId: "project-a",
    title: "测试对话",
    titleSource: "auto" as const,
    preview: "生成页面",
    status: "active" as const,
    createRequestId: "create-a",
    rowVersion: 1,
    lastActivityAt: 1,
    createdAt: 1,
    updatedAt: 1,
    archivedAt: null,
    runtimeStatus: "running" as const,
    activeTurnId: "turn-a",
    currentRunId: "run-a",
};

const taskResult = (
    status: AiDrivenTaskSubmitStatus,
    diagnostics?: IAiDrivenTaskSubmitResult["diagnostics"],
): IAiDrivenTaskSubmitResult => ({
    taskId: "task-a",
    runId: "run-a",
    threadId: "thread-a",
    turnId: "turn-a",
    eventCursor: 0,
    ...scope,
    clientSessionId: "session-a",
    status,
    createdAt: 1,
    diagnostics: diagnostics ?? {eventsRecorded: 1},
});

const emitTaskEvent = (sequence: number, kind: string, payload: Record<string, unknown>) => {
    mocks.taskEventListener?.({
        payload: {
            id: `event-${sequence}`,
            requestId: "request-a",
            projectId: "project-a",
            threadId: "thread-a",
            turnId: "turn-a",
            runId: "run-a",
            sequence,
            kind,
            payload,
            createdAt: sequence,
        },
    });
};

describe("AI driven persisted task recovery", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        vi.clearAllMocks();
        mocks.taskEventListener = undefined;
        localStorage.clear();
        mocks.projectId = "project-a";
        mocks.resolveShortcut.mockReturnValue({requirement: "生成页面"});
        mocks.buildSubmitPayload.mockReturnValue({schemaVersion: "ai-driven-task/v3"});
        mocks.getTaskEvents.mockResolvedValue({events: [], nextCursor: 0});
        mocks.listThreads.mockResolvedValue({data: [thread], nextCursor: null});
        mocks.readThread.mockResolvedValue(thread);
        mocks.createThread.mockResolvedValue(thread);
        localStorage.setItem(ACTIVE_TASK_STORAGE_KEY, JSON.stringify(scope));
    });

    afterEach(() => {
        vi.useRealTimers();
        localStorage.clear();
    });

    test("polls a running task until completion and presents diagnostics", async () => {
        mocks.getTaskStatus
            .mockResolvedValueOnce(taskResult("running"))
            .mockResolvedValueOnce(taskResult("completed", {
                eventsRecorded: 4,
                persistenceError: {code: "checkpoint-warning", message: "结果已完成，但检查点写入失败"},
            }));
        const store = new AiDrivenBarStore();

        await store.restorePersistedTask();

        expect(store.running).toBe(true);
        expect(mocks.getTaskStatus).toHaveBeenCalledTimes(1);
        await vi.advanceTimersByTimeAsync(1000);

        expect(mocks.getTaskStatus).toHaveBeenCalledTimes(2);
        expect(store.running).toBe(false);
        expect(localStorage.getItem(ACTIVE_TASK_STORAGE_KEY)).toBeNull();
        expect(mocks.appendLog).toHaveBeenCalledWith({level: "success", message: "任务已完成"});
        expect(mocks.appendLog).toHaveBeenCalledWith({
            level: "warning",
            message: "结果已完成，但检查点写入失败",
        });
        expect(vi.getTimerCount()).toBe(0);
    });

    test("restores and preserves the backend waiting state while polling", async () => {
        const restoredResult: IAiDrivenTaskSubmitResult = {
            ...taskResult("waitingUserInput"),
            runtime: {
                usage: {inputTokens: 1200, outputTokens: 300, totalTokens: 1500},
                operations: [],
            },
        };
        mocks.getTaskStatus
            .mockResolvedValueOnce(restoredResult)
            .mockResolvedValueOnce(taskResult("waitingUserInput"));
        const store = new AiDrivenBarStore();

        await store.restorePersistedTask();

        expect(mocks.startRun).toHaveBeenLastCalledWith("request-a", "waiting");
        expect(mocks.setRuntimeSnapshot).toHaveBeenCalledWith(restoredResult.runtime);
        expect(JSON.parse(localStorage.getItem(ACTIVE_TASK_STORAGE_KEY) ?? "{}")).toMatchObject({
            status: "waitingUserInput",
        });

        await vi.advanceTimersByTimeAsync(1000);

        expect(mocks.startRun).toHaveBeenLastCalledWith("request-a", "waiting");
        expect(mocks.getTaskStatus).toHaveBeenCalledTimes(2);
        store.destroy();
    });

    test("terminal event closes an in-flight task and ignores the late running response", async () => {
        let resolvePoll: ((result: IAiDrivenTaskSubmitResult) => void) | undefined;
        mocks.getTaskStatus
            .mockResolvedValueOnce(taskResult("running"))
            .mockImplementationOnce(() => new Promise<IAiDrivenTaskSubmitResult>((resolve) => resolvePoll = resolve))
            .mockResolvedValueOnce(taskResult("completed"));
        const store = new AiDrivenBarStore();
        await store.restorePersistedTask();
        await vi.advanceTimersByTimeAsync(1000);

        emitTaskEvent(1, "turn.completed", {status: "completed"});
        resolvePoll?.(taskResult("running"));
        await Promise.resolve();
        await vi.advanceTimersByTimeAsync(2000);

        expect(store.running).toBe(false);
        expect(localStorage.getItem(ACTIVE_TASK_STORAGE_KEY)).toBeNull();
        expect(mocks.finishRun).toHaveBeenCalledWith("completed", "任务已完成，请核对画布动作和最终回复");
    });

    test("waiting event persists status before cursor so failed restore keeps waiting", async () => {
        mocks.getTaskStatus.mockResolvedValue(taskResult("running"));
        const store = new AiDrivenBarStore();
        await store.restorePersistedTask();

        emitTaskEvent(1, "turn.waiting", {status: "waitingUserConfirmation"});
        expect(JSON.parse(localStorage.getItem(ACTIVE_TASK_STORAGE_KEY) ?? "{}")).toMatchObject({
            eventCursor: 1,
            status: "waitingUserConfirmation",
        });
        expect(mocks.startRun).toHaveBeenLastCalledWith("request-a", "waiting");
        store.destroy();
    });

    test("keeps a newly admitted task running until status reaches a terminal state", async () => {
        localStorage.clear();
        const currentTaskResult = (status: AiDrivenTaskSubmitStatus): IAiDrivenTaskSubmitResult => ({
            ...taskResult(status),
            requestId: "new-request",
            clientSessionId: "new-session",
        });
        mocks.submitTask.mockResolvedValue(currentTaskResult("running"));
        mocks.getTaskStatus.mockResolvedValue(currentTaskResult("completed"));
        const store = new AiDrivenBarStore(true);
        store.modelConfigId = "model-config-1";
        store.runtimeModelId = "model-1";
        store.setContent("生成页面");

        await store.submitPrompt();

        expect(store.running).toBe(true);
        expect(store.submitting).toBe(false);
        expect(store.currentThreadId).toBe("thread-a");
        expect(store.currentTurnId).toBe("turn-a");
        expect(store.content).toBe("");
        expect(mocks.getTaskStatus).not.toHaveBeenCalled();

        await vi.advanceTimersByTimeAsync(1000);

        expect(mocks.getTaskStatus).toHaveBeenCalledTimes(1);
        expect(store.running).toBe(false);
        expect(localStorage.getItem(ACTIVE_TASK_STORAGE_KEY)).toBeNull();
    });

    test("sends new input to the active turn without creating another task", async () => {
        localStorage.clear();
        mocks.submitTask.mockResolvedValue({
            ...taskResult("running"),
            requestId: "new-request",
            clientSessionId: "new-session",
        });
        mocks.steerTask.mockResolvedValue({
            threadId: "thread-a",
            turnId: "turn-a",
            acceptedInputId: "steer-input",
        });
        const store = new AiDrivenBarStore(true);
        store.modelConfigId = "model-config-1";
        store.runtimeModelId = "model-1";
        store.setContent("生成页面");
        await store.submitPrompt();

        store.setContent("改为深色主题");
        await store.submitPrompt();

        expect(mocks.submitTask).toHaveBeenCalledTimes(1);
        expect(mocks.steerTask).toHaveBeenCalledWith({
            threadId: "thread-a",
            expectedTurnId: "turn-a",
            runId: "run-a",
            projectId: "project-a",
            inputId: "steer-input",
            input: "改为深色主题",
        });
        expect(store.content).toBe("");
        expect(store.running).toBe(true);
        store.destroy();
    });

    test("recovers turn and run from the thread snapshot when both submit responses are lost", async () => {
        localStorage.clear();
        const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
        const consoleWarn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
        mocks.submitTask.mockRejectedValue(new Error("response lost"));
        mocks.getTaskStatus.mockResolvedValue({
            ...taskResult("running"),
            requestId: "new-request",
            clientSessionId: "new-session",
        });
        const store = new AiDrivenBarStore(true);
        store.modelConfigId = "model-config-1";
        store.runtimeModelId = "model-1";
        store.setContent("生成页面");

        await store.submitPrompt();

        expect(mocks.readThread).toHaveBeenCalledWith({
            projectId: "project-a",
            threadId: "thread-a",
        }, {suppressErrorMessage: true});
        expect(store.currentTurnId).toBe("turn-a");
        expect(store.currentRunId).toBe("run-a");
        expect(localStorage.getItem(PENDING_TASK_STORAGE_KEY)).toBeNull();
        expect(JSON.parse(localStorage.getItem(ACTIVE_TASK_STORAGE_KEY) ?? "{}")).toMatchObject({
            requestId: "new-request",
            projectId: "project-a",
            threadId: "thread-a",
            turnId: "turn-a",
            runId: "run-a",
        });
        store.destroy();
        consoleWarn.mockRestore();
        consoleError.mockRestore();
    });

    test("uses the same request payload when the idempotent submit retry succeeds", async () => {
        localStorage.clear();
        const consoleWarn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
        mocks.submitTask
            .mockRejectedValueOnce(new Error("first response lost"))
            .mockResolvedValueOnce({
                ...taskResult("running"),
                requestId: "new-request",
                clientSessionId: "new-session",
            });
        const store = new AiDrivenBarStore(true);
        store.modelConfigId = "model-config-1";
        store.runtimeModelId = "model-1";
        store.setContent("生成页面");

        await store.submitPrompt();

        expect(mocks.submitTask).toHaveBeenCalledTimes(2);
        expect(mocks.submitTask.mock.calls[1][0]).toBe(mocks.submitTask.mock.calls[0][0]);
        expect(mocks.readThread).not.toHaveBeenCalled();
        expect(store.currentTurnId).toBe("turn-a");
        expect(store.currentRunId).toBe("run-a");
        store.destroy();
        consoleWarn.mockRestore();
    });

    test("closing the panel stops monitoring without clearing thread identity", async () => {
        localStorage.clear();
        mocks.submitTask.mockResolvedValue({
            ...taskResult("running"),
            requestId: "new-request",
            clientSessionId: "new-session",
        });
        const store = new AiDrivenBarStore(true);
        store.modelConfigId = "model-config-1";
        store.runtimeModelId = "model-1";
        store.setContent("生成页面");
        await store.submitPrompt();

        store.resetAfterClose();

        expect(store.threadStore.selectedThreadId).toBe("thread-a");
        expect(store.currentThreadId).toBe("thread-a");
        expect(store.currentTurnId).toBe("turn-a");
        expect(store.currentRunId).toBe("run-a");
        expect(localStorage.getItem(ACTIVE_TASK_STORAGE_KEY)).not.toBeNull();
        expect(store.mounted).toBe(false);
        store.destroy();
    });

    test("retries an initial 500 null response and completes without reopening", async () => {
        mocks.getTaskStatus
            .mockImplementationOnce((
                _scope: typeof scope,
                options: {suppressErrorMessage?: (response: {code: number}) => boolean},
            ) => {
                options.suppressErrorMessage?.({code: 500});
                return Promise.resolve(null);
            })
            .mockResolvedValueOnce(taskResult("completed"));
        const store = new AiDrivenBarStore();

        await store.restorePersistedTask();
        expect(store.running).toBe(true);
        expect(mocks.getTaskStatus).toHaveBeenCalledTimes(1);

        await vi.advanceTimersByTimeAsync(1000);

        expect(mocks.getTaskStatus).toHaveBeenCalledTimes(2);
        expect(store.running).toBe(false);
        expect(localStorage.getItem(ACTIVE_TASK_STORAGE_KEY)).toBeNull();
        expect(vi.getTimerCount()).toBe(0);
    });

    test("retries an initial network exception and completes without reopening", async () => {
        const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
        mocks.getTaskStatus
            .mockRejectedValueOnce(new Error("network unavailable"))
            .mockResolvedValueOnce(taskResult("completed"));
        const store = new AiDrivenBarStore();

        await store.restorePersistedTask();
        expect(store.running).toBe(true);

        await vi.advanceTimersByTimeAsync(1000);

        expect(mocks.getTaskStatus).toHaveBeenCalledTimes(2);
        expect(store.running).toBe(false);
        expect(localStorage.getItem(ACTIVE_TASK_STORAGE_KEY)).toBeNull();
        expect(vi.getTimerCount()).toBe(0);
        expect(consoleError).toHaveBeenCalledWith(
            "AI driven task status restore failed:",
            expect.any(Error),
        );
        consoleError.mockRestore();
    });

    test("stopping a restored task cancels its pending status poll", async () => {
        mocks.getTaskStatus.mockResolvedValue(taskResult("waitingUserInput"));
        mocks.cancelTask.mockResolvedValue(taskResult("cancelled"));
        const store = new AiDrivenBarStore();
        await store.restorePersistedTask();

        await store.stopCurrentTask();
        await vi.advanceTimersByTimeAsync(4000);

        expect(mocks.cancelTask).toHaveBeenCalledWith({
            projectId: "project-a",
            threadId: "thread-a",
            turnId: "turn-a",
            runId: "run-a",
        }, {suppressErrorMessage: true});
        expect(mocks.getTaskStatus).toHaveBeenCalledTimes(1);
        expect(store.running).toBe(false);
        expect(localStorage.getItem(ACTIVE_TASK_STORAGE_KEY)).toBeNull();
        expect(vi.getTimerCount()).toBe(0);
    });

    test("project switching stops polling without clearing the previous project record", async () => {
        mocks.getTaskStatus.mockResolvedValue(taskResult("running"));
        const store = new AiDrivenBarStore();
        await store.restorePersistedTask();

        mocks.projectId = "project-b";
        await vi.advanceTimersByTimeAsync(1000);

        expect(mocks.getTaskStatus).toHaveBeenCalledTimes(1);
        expect(store.running).toBe(false);
        expect(localStorage.getItem(ACTIVE_TASK_STORAGE_KEY)).not.toBeNull();
        expect(vi.getTimerCount()).toBe(0);
    });

    test("destroy aborts an in-flight poll and ignores its late result", async () => {
        let resolvePoll: ((result: IAiDrivenTaskSubmitResult) => void) | undefined;
        mocks.getTaskStatus
            .mockResolvedValueOnce(taskResult("running"))
            .mockImplementationOnce(() => new Promise<IAiDrivenTaskSubmitResult>((resolve) => {
                resolvePoll = resolve;
            }));
        const store = new AiDrivenBarStore();
        await store.restorePersistedTask();
        await vi.advanceTimersByTimeAsync(1000);
        const pollSignal = mocks.getTaskStatus.mock.calls[1]?.[1]?.signal as AbortSignal;

        store.destroy();
        resolvePoll?.(taskResult("running"));
        await Promise.resolve();
        await vi.advanceTimersByTimeAsync(4000);

        expect(pollSignal.aborted).toBe(true);
        expect(mocks.getTaskStatus).toHaveBeenCalledTimes(2);
        expect(store.running).toBe(false);
        expect(vi.getTimerCount()).toBe(0);
    });
});
