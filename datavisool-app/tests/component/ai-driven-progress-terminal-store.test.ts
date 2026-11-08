/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, beforeEach, describe, expect, test, vi} from "vitest";
import aiDrivenProgressTerminalStore from "@/designer/ai-driven/AiDrivenProgressTerminalStore.ts";
import type {IAiDrivenTaskEvent} from "@/designer/ai-driven/types/AiDrivenTaskSubmitTypes.ts";

vi.mock("@/designer/ai-driven/AiDrivenUtil.ts", () => ({
    getAiDrivenProgressTitle: () => "任务处理中",
}));

const taskEvent = (kind: string, payload: Record<string, unknown>, sequence: number): IAiDrivenTaskEvent => ({
    eventId: `event-${sequence}`,
    threadId: "thread-a",
    turnId: "turn-a",
    runId: "run-a",
    requestId: "request-a",
    projectId: "project-a",
    sequence,
    threadSequence: sequence,
    kind,
    payload,
    schemaVersion: 1,
    createdAt: sequence,
});

describe("AI driven progress terminal item aggregation", () => {
    beforeEach(() => {
        aiDrivenProgressTerminalStore.reset();
        aiDrivenProgressTerminalStore.setInlineHostAvailable(false);
    });
    afterEach(() => {
        aiDrivenProgressTerminalStore.setInlineHostAvailable(false);
        aiDrivenProgressTerminalStore.reset();
    });

    test("aggregates agent message deltas and accepts the completed snapshot", () => {
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("item.started", {
            item: {itemId: "message-1", itemType: "agentMessage", status: "inProgress", payload: {}},
        }, 1));
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("item.delta", {
            itemId: "message-1",
            itemType: "agentMessage",
            delta: "正在分析",
        }, 2));
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("item.completed", {
            item: {
                itemId: "message-1",
                itemType: "agentMessage",
                status: "completed",
                payload: {text: "正在分析需求", truncated: false},
            },
        }, 3));

        expect(aiDrivenProgressTerminalStore.agentMessages).toEqual([{
            id: "message-1",
            status: "completed",
            text: "正在分析需求",
            truncated: false,
        }]);
    });

    test("removes a completion candidate rejected by the host fact check", () => {
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("item.completed", {
            item: {
                itemId: "message-rejected",
                itemType: "agentMessage",
                status: "completed",
                payload: {text: "缺少画布工具，任务无法完成", truncated: false},
            },
        }, 1));
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("agent.completion.fact_check", {
            check: {status: "waiting", missingRequiredCanvasEffects: ["createComponents"]},
        }, 2));

        expect(aiDrivenProgressTerminalStore.agentMessages).toEqual([]);
        expect(aiDrivenProgressTerminalStore.timelineItems).toEqual([]);
    });

    test("keeps the final response accepted by the host fact check", () => {
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("item.completed", {
            item: {
                itemId: "message-accepted",
                itemType: "agentMessage",
                status: "completed",
                payload: {text: "KPI 卡片已创建", truncated: false},
            },
        }, 1));
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("agent.completion.fact_check", {
            check: {status: "passed"},
        }, 2));

        expect(aiDrivenProgressTerminalStore.agentMessages).toHaveLength(1);
        expect(aiDrivenProgressTerminalStore.timelineItems).toHaveLength(1);
    });

    test("shows budget exhaustion as a resumable terminal state", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("turn.budgetExhausted", {
            message: "Token 预算已用尽",
        }, 1));

        expect(aiDrivenProgressTerminalStore.runState).toBe("budgetExhausted");
        expect(aiDrivenProgressTerminalStore.runResult).toEqual(expect.objectContaining({
            status: "budgetExhausted",
            message: "Token 预算已用尽",
        }));
    });

    test("accepts the agent budget exhaustion event used by the runtime", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("agent.budget_exhausted", {}, 1));

        expect(aiDrivenProgressTerminalStore.runState).toBe("budgetExhausted");
        expect(aiDrivenProgressTerminalStore.runResult?.message).toBe("预算已用尽，可增加预算后继续");
    });

    test("replaces the current plan with validated plan steps", () => {
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("item.completed", {
            item: {
                itemId: "plan:run-a",
                itemType: "plan",
                status: "completed",
                payload: {
                explanation: "先分析再生成",
                steps: [
                    {step: "分析页面", status: "completed"},
                    {step: "生成页面", status: "inProgress"},
                    {step: "校验结果", status: "pending"},
                ],
                },
            },
        }, 1));

        expect(aiDrivenProgressTerminalStore.planExplanation).toBe("先分析再生成");
        expect(aiDrivenProgressTerminalStore.planSteps).toEqual([
            {step: "分析页面", status: "completed"},
            {step: "生成页面", status: "inProgress"},
            {step: "校验结果", status: "pending"},
        ]);
    });

    test("updates one semantic timeline item throughout a tool call", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("item.started", {
            item: {
                itemId: "tool-1",
                itemType: "toolCall",
                status: "inProgress",
                payload: {toolName: "inspect_current_page", category: "read"},
            },
        }, 1));
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("item.completed", {
            item: {
                itemId: "tool-1",
                itemType: "toolCall",
                status: "completed",
                payload: {toolName: "inspect_current_page", category: "read", durationMs: 840},
            },
        }, 2));

        expect(aiDrivenProgressTerminalStore.timelineItems).toEqual([expect.objectContaining({
            id: "tool-1",
            type: "toolCall",
            status: "completed",
            payload: expect.objectContaining({
                toolName: "inspect_current_page",
                durationMs: 840,
            }),
        })]);
    });

    test("keeps the timeline and exposes a review result after completion", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("turn.completed", {
            message: "页面已生成",
        }, 1));

        expect(aiDrivenProgressTerminalStore.runState).toBe("completed");
        expect(aiDrivenProgressTerminalStore.runResult).toEqual(expect.objectContaining({
            status: "completed",
            message: "页面已生成",
        }));
        expect(aiDrivenProgressTerminalStore.visible).toBe(true);
    });

    test("tracks cumulative token usage and ignores stale snapshots", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("agent.model_call.completed", {
            usage: {inputTokens: 1200, outputTokens: 300, totalTokens: 1500},
        }, 1));
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("agent.tool_call.completed", {
            usage: {inputTokens: 800, outputTokens: 200, totalTokens: 1000},
        }, 2));

        expect(aiDrivenProgressTerminalStore.tokenUsage).toEqual({
            inputTokens: 1200,
            outputTokens: 300,
            totalTokens: 1500,
        });
    });

    test("preserves cache usage as input sub-items without double counting", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.setTokenUsage({
            inputTokens: 100,
            outputTokens: 20,
            totalTokens: 140,
            cachedInputTokens: 15,
            cacheCreationInputTokens: 5,
        });

        expect(aiDrivenProgressTerminalStore.tokenUsage).toEqual({
            inputTokens: 100,
            outputTokens: 20,
            totalTokens: 140,
            cachedInputTokens: 15,
            cacheCreationInputTokens: 5,
        });
    });

    test("keeps cumulative provider input separate from current context size", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.setTokenUsage({
            inputTokens: 10,
            cachedInputTokens: 90,
            contextInputTokens: 100,
            outputTokens: 5,
            totalTokens: 105,
        });

        expect(aiDrivenProgressTerminalStore.tokenUsage).toEqual({
            inputTokens: 10,
            contextInputTokens: 100,
            outputTokens: 5,
            totalTokens: 105,
            cachedInputTokens: 90,
        });
    });

    test("retains context capacity and compaction metrics independently", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.setTokenUsage({
            inputTokens: 400,
            outputTokens: 30,
            totalTokens: 430,
            activeContextTokens: 180,
            contextCapacityTokens: 256,
            contextCapacitySource: "modelCatalog",
            compactionCount: 2,
            compactionBeforeTokens: 240,
            compactionAfterTokens: 120,
        });

        expect(aiDrivenProgressTerminalStore.tokenUsage).toEqual(expect.objectContaining({
            inputTokens: 400,
            activeContextTokens: 180,
            contextCapacityTokens: 256,
            contextCapacitySource: "modelCatalog",
            compactionCount: 2,
            compactionBeforeTokens: 240,
            compactionAfterTokens: 120,
        }));
    });

    test("keeps disabled budget remaining values unavailable", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.setTokenUsage({
            inputTokens: 1,
            outputTokens: 1,
            totalTokens: 2,
            tokenBudgetEnabled: false,
            callBudgetEnabled: false,
            durationBudgetEnabled: false,
            remainingTotalTokens: null,
            remainingModelCalls: null,
            remainingToolCalls: null,
            remainingWriteCalls: null,
            remainingFrontendInteractions: null,
            remainingDurationMs: null,
        });

        expect(aiDrivenProgressTerminalStore.tokenUsage).toEqual(expect.objectContaining({
            tokenBudgetEnabled: false,
            callBudgetEnabled: false,
            durationBudgetEnabled: false,
            remainingTotalTokens: null,
            remainingDurationMs: null,
        }));
    });

    test("keeps one durable receipt per canvas operation", () => {
        const operation = {
            operationId: "run-a:call-1",
            toolCallId: "call-1",
            kind: "patchComponents",
            status: "applying",
            projectId: "project-a",
            componentIds: ["component-1"],
        };
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("agent.canvas_operation.applying", {operation}, 1));
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("agent.canvas_operation.applied", {
            operation: {...operation, status: "applied", resultRevision: "sha256:result"},
        }, 2));

        expect(aiDrivenProgressTerminalStore.canvasOperations).toEqual([expect.objectContaining({
            operationId: "run-a:call-1",
            status: "applied",
            resultRevision: "sha256:result",
        })]);
    });

    test("keeps rejected and failed delete receipts visible", () => {
        const operation = {
            operationId: "run-a:delete-1",
            toolCallId: "delete-1",
            kind: "deleteComponents",
            status: "rejected",
            projectId: "project-a",
            componentIds: ["component-1"],
        };
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("agent.canvas_operation.rejected", {operation}, 1));
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("agent.canvas_operation.failed", {
            operation: {...operation, status: "failed", error: {message: "删除失败"}},
        }, 2));

        expect(aiDrivenProgressTerminalStore.canvasOperations).toEqual([expect.objectContaining({
            operationId: "run-a:delete-1",
            kind: "deleteComponents",
            status: "failed",
            error: {message: "删除失败"},
        })]);
    });

    test("waits and resumes from turn events and only completes with turn.completed", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("agent.completed", {
            message: "Agent 内部步骤完成",
        }, 1));
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("agent.failed", {
            message: "Agent 内部失败意图",
        }, 2));
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("agent.cancelled", {
            message: "Agent 内部取消意图",
        }, 3));

        expect(aiDrivenProgressTerminalStore.runState).toBe("running");
        expect(aiDrivenProgressTerminalStore.runResult).toBeNull();

        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("turn.waiting", {
            status: "waitingUserInput",
        }, 4));
        expect(aiDrivenProgressTerminalStore.runState).toBe("waiting");

        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("turn.resumed", {
            status: "running",
        }, 5));
        expect(aiDrivenProgressTerminalStore.runState).toBe("running");

        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("turn.completed", {
            message: "Turn 已完成",
        }, 6));
        expect(aiDrivenProgressTerminalStore.runState).toBe("completed");
        expect(aiDrivenProgressTerminalStore.runResult?.message).toBe("Turn 已完成");
    });

    test("exposes structured waiting and execution notices", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("turn.waiting", {
            reason: "missingConstraint",
        }, 1));
        expect(aiDrivenProgressTerminalStore.runNotice).toEqual({
            kind: "missingConstraint",
            message: "缺少结构化要求，等待补充后继续",
        });

        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("agent.canvas_operation.failed", {
            operation: {
                operationId: "run-a:patch-1",
                toolCallId: "patch-1",
                kind: "patchComponents",
                status: "failed",
                projectId: "project-a",
                componentIds: ["component-1"],
            },
        }, 2));
        expect(aiDrivenProgressTerminalStore.runNotice).toEqual({
            kind: "executionError",
            message: "画布动作未生效（1 个组件）",
        });
    });

    test("resolves an inline interaction and returns to running state", async () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.setInlineHostAvailable(true);
        const completion = aiDrivenProgressTerminalStore.requestInlineInteraction({
            kind: "userInput",
            title: "补充信息",
            message: "请选择主题",
        });

        expect(completion).not.toBeNull();
        expect(aiDrivenProgressTerminalStore.runState).toBe("waiting");
        expect(aiDrivenProgressTerminalStore.pendingInlineInteraction).toEqual(expect.objectContaining({
            kind: "userInput",
            message: "请选择主题",
        }));

        aiDrivenProgressTerminalStore.completeInlineInteraction({
            action: "submit",
            data: {message: "请选择主题", supplement: "深色科技风"},
        });

        await expect(completion).resolves.toEqual({
            action: "submit",
            data: {message: "请选择主题", supplement: "深色科技风"},
        });
        expect(aiDrivenProgressTerminalStore.runState).toBe("running");
        expect(aiDrivenProgressTerminalStore.pendingInlineInteraction).toBeNull();
    });
});
