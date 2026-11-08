/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {beforeEach, describe, expect, test, vi} from "vitest";
import {listen} from "@tauri-apps/api/event";
import type {IAiDrivenTaskEvent} from "@/designer/ai-driven/types/AiDrivenTaskSubmitTypes.ts";

const mocks = vi.hoisted(() => ({
    getEvents: vi.fn(),
    listener: undefined as ((event: {payload: IAiDrivenTaskEvent}) => void) | undefined,
    unlisten: vi.fn(),
}));

vi.mock("@/api/AiDrivenTaskApi.ts", () => ({
    getAiDrivenTaskEvents: mocks.getEvents,
}));

vi.mock("@tauri-apps/api/event", () => ({
    listen: vi.fn(async (_channel: string, listener: (event: {payload: IAiDrivenTaskEvent}) => void) => {
        mocks.listener = listener;
        return mocks.unlisten;
    }),
}));

import {
    AiDrivenTaskEventStream,
    aiDrivenTaskNoticeKind,
    describeAiDrivenTaskEvent,
} from "@/designer/ai-driven/AiDrivenTaskEventStream.ts";

const scope = {
    projectId: "project-a",
    threadId: "thread-a",
    turnId: "turn-a",
    runId: "run-a",
    eventCursor: 0,
};

const event = (sequence: number, kind = "agent.model_call.started"): IAiDrivenTaskEvent => ({
    eventId: `event-${sequence}`,
    ...scope,
    requestId: "request-a",
    sequence,
    threadSequence: sequence,
    kind,
    payload: {stepIndex: sequence},
    dedupeKey: `event-${sequence}`,
    schemaVersion: 1,
    createdAt: sequence,
});

describe("AI driven task event stream", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        mocks.listener = undefined;
    });

    test("orders and deduplicates live events with cursor replay", async () => {
        let resolveReplay: ((value: {events: IAiDrivenTaskEvent[]; nextCursor: number}) => void) | undefined;
        mocks.getEvents.mockReturnValue(new Promise((resolve) => {
            resolveReplay = resolve;
        }));
        const stream = new AiDrivenTaskEventStream();
        const received: number[] = [];
        const cursors: number[] = [];

        const started = stream.start(scope, {
            onEvent: (item) => received.push(item.sequence),
            onCursor: (cursor) => cursors.push(cursor),
            onTerminal: vi.fn(),
        });
        await vi.waitFor(() => expect(mocks.listener).toBeTypeOf("function"));
        expect(listen).toHaveBeenCalledWith("datavisool:ai-driven-task-event", expect.any(Function));
        mocks.listener?.({payload: event(3)});
        resolveReplay?.({events: [event(1), event(2), event(3)], nextCursor: 3});
        await started;

        expect(received).toEqual([1, 2, 3]);
        expect(cursors).toEqual([1, 2, 3]);
        stream.stop();
        expect(mocks.unlisten).toHaveBeenCalledTimes(1);
    });

    test("treats turn.completed as terminal instead of agent.completed", async () => {
        mocks.getEvents.mockResolvedValue({events: [], nextCursor: 0});
        const stream = new AiDrivenTaskEventStream();
        const onTerminal = vi.fn();

        await stream.start(scope, {
            onEvent: vi.fn(),
            onCursor: vi.fn(),
            onTerminal,
        });
        mocks.listener?.({payload: event(1, "agent.completed")});
        expect(onTerminal).not.toHaveBeenCalled();

        mocks.listener?.({payload: event(2, "turn.completed")});
        expect(onTerminal).toHaveBeenCalledTimes(1);
        stream.stop();
    });

    test("discards events from an older run and callbacks delivered after the scope stops", async () => {
        mocks.getEvents.mockResolvedValue({events: [], nextCursor: 0});
        const stream = new AiDrivenTaskEventStream();
        const received: number[] = [];
        await stream.start(scope, {
            onEvent: (item) => received.push(item.sequence),
            onCursor: vi.fn(),
            onTerminal: vi.fn(),
        });
        const listener = mocks.listener;
        listener?.({payload: {...event(1), runId: "run-old"}});
        listener?.({payload: event(1)});
        expect(received).toEqual([1]);

        stream.stop();
        listener?.({payload: event(2)});
        expect(received).toEqual([1]);
    });

    test("describes tool progress from the unified item event only", () => {
        expect(describeAiDrivenTaskEvent(event(3))).toBe("AI 正在分析需求（第 3 轮）");
        expect(describeAiDrivenTaskEvent({
            ...event(1, "item.started"),
            payload: {
                item: {
                    itemId: "tool-call-1",
                    itemType: "toolCall",
                    status: "inProgress",
                    payload: {toolName: "update_plan"},
                },
            },
        })).toBe("正在更新任务计划");
        expect(describeAiDrivenTaskEvent({
            ...event(2, "agent.tool_call.started"),
            payload: {toolName: "update_plan"},
        })).toBeNull();
        expect(describeAiDrivenTaskEvent({
            ...event(4, "agent.context_compaction.started"),
            payload: {stepIndex: 4},
        })).toBe("上下文接近容量上限，正在自动压缩");
        expect(describeAiDrivenTaskEvent({
            ...event(5, "turn.waiting"),
            payload: {status: "waitingUserInput"},
        })).toBe("等待你补充信息");
        expect(describeAiDrivenTaskEvent({
            ...event(6, "turn.resumed"),
            payload: {status: "running"},
        })).toBe("继续执行");
        expect(describeAiDrivenTaskEvent({
            ...event(7, "turn.verifying"),
            payload: {status: "verifying"},
        })).toBe("正在核验任务完成条件");
        expect(describeAiDrivenTaskEvent({
            ...event(8, "agent.completion.fact_check"),
            payload: {check: {status: "waiting"}},
        })).toBe("正在等待画布动作返回结果");
        expect(describeAiDrivenTaskEvent({
            ...event(9, "agent.completion.fact_check"),
            payload: {check: {status: "passed"}},
        })).toBe("动作事实检查已通过");
        expect(describeAiDrivenTaskEvent({
            ...event(10, "item.started"),
            payload: {
                item: {
                    itemId: "legacy-finish-task",
                    itemType: "toolCall",
                    status: "inProgress",
                    payload: {toolName: "finish_task"},
                },
            },
        })).toBe("正在执行 finish_task");
    });

    test("describes persisted canvas operation facts separately from model text", () => {
        expect(describeAiDrivenTaskEvent({
            ...event(8, "agent.canvas_operation.applied"),
            payload: {operation: {status: "applied", componentIds: ["A", "B"]}},
        })).toBe("画布动作已真实生效（2 个组件）");
        expect(describeAiDrivenTaskEvent({
            ...event(9, "agent.canvas_operation.rejected"),
            payload: {operation: {status: "rejected", componentIds: []}},
        })).toBe("用户已拒绝画布预览，未修改真实画布");
    });

    test("distinguishes structured waiting reasons and failed completion facts", () => {
        const missingConstraint = {
            ...event(10, "turn.waiting"),
            payload: {reason: "missingConstraint"},
        };
        expect(aiDrivenTaskNoticeKind(missingConstraint)).toBe("missingConstraint");
        expect(describeAiDrivenTaskEvent(missingConstraint)).toBe("缺少结构化要求，等待补充后继续");

        const frontendToolWaiting = {
            ...event(11, "turn.waiting"),
            payload: {pending: {kind: "frontendTool"}},
        };
        expect(aiDrivenTaskNoticeKind(frontendToolWaiting)).toBe("waiting");
        expect(describeAiDrivenTaskEvent(frontendToolWaiting)).toBe("正在等待前端工具返回");

        expect(describeAiDrivenTaskEvent({
            ...event(11, "turn.waiting"),
            payload: {status: "waitingFrontendTool"},
        })).toBe("正在等待前端工具返回");

        const userConfirmationWaiting = {
            ...event(12, "agent.waiting_user"),
            payload: {pending: {kind: "userConfirmation"}},
        };
        expect(describeAiDrivenTaskEvent(userConfirmationWaiting)).toBe("等待你确认画布动作");

        const missingFactCheck = {
            ...event(13, "agent.completion.fact_check"),
            payload: {check: {status: "waiting", missingRequiredCanvasEffects: ["createComponents"]}},
        };
        expect(aiDrivenTaskNoticeKind(missingFactCheck)).toBe("missingConstraint");
        expect(describeAiDrivenTaskEvent(missingFactCheck)).toBe("缺少必需的真实结果，等待继续处理");

        const failedFactCheck = {
            ...event(14, "agent.completion.fact_check"),
            payload: {check: {status: "failed", reason: "executionError"}},
        };
        expect(aiDrivenTaskNoticeKind(failedFactCheck)).toBe("executionError");
        expect(describeAiDrivenTaskEvent(failedFactCheck)).toBe("画布动作执行失败，任务未完成");

        expect(aiDrivenTaskNoticeKind(event(15, "agent.failed"))).toBe("executionError");
        expect(describeAiDrivenTaskEvent(event(15, "agent.failed"))).toBe("Agent 执行出现问题，任务未完成");
    });
});
