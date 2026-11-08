/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {cleanup, fireEvent, render, screen} from "@testing-library/react";
import {afterEach, beforeEach, describe, expect, test} from "vitest";
import AiDrivenProgressTerminal, {formatDuration} from "@/designer/ai-driven/AiDrivenProgressTerminal.tsx";
import aiDrivenProgressTerminalStore from "@/designer/ai-driven/AiDrivenProgressTerminalStore.ts";
import type {IAiDrivenTaskEvent} from "@/designer/ai-driven/types/AiDrivenTaskSubmitTypes.ts";

const taskEvent = (kind: string, item: Record<string, unknown>, sequence: number): IAiDrivenTaskEvent => ({
    eventId: `event-${sequence}`,
    threadId: "thread-a",
    turnId: "turn-a",
    runId: "run-a",
    requestId: "request-a",
    projectId: "project-a",
    sequence,
    threadSequence: sequence,
    kind,
    payload: {item: {
        itemId: item.id,
        itemType: item.type,
        status: item.status,
        payload: Object.fromEntries(Object.entries(item).filter(([key]) => !["id", "type", "status"].includes(key))),
    }},
    schemaVersion: 1,
    createdAt: sequence,
});

describe("AI driven progress timeline", () => {
    beforeEach(() => aiDrivenProgressTerminalStore.reset());
    afterEach(() => {
        cleanup();
        aiDrivenProgressTerminalStore.reset();
    });

    test("separates execution steps from messages and hides internal details", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("item.completed", {
            id: "user-message-1",
            type: "userMessage",
            status: "completed",
            text: "优化组件样式",
        }, 1));
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("item.completed", {
            id: "agent-message-1",
            type: "agentMessage",
            status: "completed",
            text: "我先读取当前页面。\n[Tool call: inspect_current_page]",
        }, 2));
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("item.completed", {
            id: "tool-1",
            type: "toolCall",
            status: "completed",
            toolName: "inspect_current_page",
        }, 3));
        aiDrivenProgressTerminalStore.appendLog({message: "内部调试信息"});

        render(<AiDrivenProgressTerminal/>);

        expect(screen.getByText("执行过程")).toBeInTheDocument();
        expect(screen.queryByText("任务列表")).not.toBeInTheDocument();
        expect(screen.queryByText("执行当前任务")).not.toBeInTheDocument();
        expect(screen.queryByRole("progressbar", {name: /任务进度|Task progress/})).not.toBeInTheDocument();
        expect(screen.getByText("读取当前页面")).toBeInTheDocument();
        expect(screen.getByText("我先读取当前页面。")).toBeInTheDocument();
        expect(screen.queryByText("任务动态")).not.toBeInTheDocument();
        expect(screen.queryByText(/Tool call/)).not.toBeInTheDocument();
        expect(screen.queryByText("运行详情")).not.toBeInTheDocument();
        expect(screen.queryByText("内部调试信息")).not.toBeInTheDocument();
    });

    test("hides the execution scope declaration from the activity timeline", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("item.completed", {
            id: "scope-1",
            type: "toolCall",
            status: "completed",
            toolName: "declare_execution_scope",
        }, 1));

        render(<AiDrivenProgressTerminal/>);

        expect(screen.queryByText("声明执行范围")).not.toBeInTheDocument();
        expect(screen.queryByText("确认本轮允许的写入动作")).not.toBeInTheDocument();
    });

    test("shows structured plan progress in the right task column", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("item.completed", {
            id: "plan-1",
            type: "plan",
            status: "completed",
            explanation: "先分析，再生成并校验",
            steps: [
                {step: "分析当前页面", status: "completed"},
                {step: "生成页面内容", status: "inProgress"},
                {step: "校验最终结果", status: "pending"},
            ],
        }, 1));

        render(<AiDrivenProgressTerminal/>);

        expect(screen.getByText("分析当前页面")).toBeInTheDocument();
        expect(screen.getByText("生成页面内容")).toBeInTheDocument();
        expect(screen.getByText("校验最终结果")).toBeInTheDocument();
        expect(screen.getByRole("progressbar", {name: /任务进度|Task progress/})).toHaveAttribute("aria-valuenow", "33");
        expect(screen.getByRole("img", {name: /执行中|Running/})).toBeInTheDocument();
    });

    test("shows the turn and session totals, with the breakdown in its details", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.setTokenUsage({
            inputTokens: 1234,
            outputTokens: 567,
            totalTokens: 1801,
        });

        render(<AiDrivenProgressTerminal/>);

        const trigger = screen.getByRole("button", {name: "查看 Token 消耗明细"});
        expect(trigger).toHaveTextContent(/本轮1,801会话累计—/);
        expect(screen.queryByText("输入")).not.toBeInTheDocument();
        fireEvent.click(trigger);
        expect(screen.getByRole("row", {name: /(?:输入|Input) 1,234 —/})).toBeInTheDocument();
        expect(screen.getByRole("row", {name: /(?:输出|Output) 567 —/})).toBeInTheDocument();
    });

    test("shows cache details without adding them to total usage", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.setTokenUsage({
            inputTokens: 100,
            outputTokens: 20,
            totalTokens: 140,
            cachedInputTokens: 15,
            cacheCreationInputTokens: 5,
        });

        render(<AiDrivenProgressTerminal/>);

        const trigger = screen.getByRole("button", {name: "查看 Token 消耗明细"});
        expect(trigger).toHaveTextContent("140");
        expect(trigger).not.toHaveTextContent("缓存命中");
        fireEvent.click(trigger);
        expect(screen.getByRole("row", {name: /(?:缓存命中|Cache hit) 15 —/})).toBeInTheDocument();
        expect(screen.getByRole("row", {name: /(?:缓存新建|Cache created) 5 —/})).toBeInTheDocument();
    });

    test("shows the structured reason while waiting for user confirmation", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.applyTaskEvent({
            ...taskEvent("turn.waiting", {}, 1),
            payload: {pending: {kind: "userConfirmation"}},
        });

        render(<AiDrivenProgressTerminal/>);

        expect(screen.getByText("等待你确认画布动作")).toBeInTheDocument();
    });

    test("formats durations with a unit matching their scale", () => {
        expect(formatDuration(999)).toBe("999 ms");
        expect(formatDuration(1_000)).toBe("1.0 s");
        expect(formatDuration(59_999)).toBe("60.0 s");
        expect(formatDuration(60_000)).toBe("1.0 min");
    });

    test("shows canvas receipts separately from the agent explanation", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.setCanvasOperations([{
            operationId: "run-a:call-1",
            toolCallId: "call-1",
            kind: "createComponents",
            status: "applied",
            projectId: "project-a",
            componentIds: ["component-1", "component-2"],
            resultRevision: "sha256:result",
        }]);
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("item.completed", {
            id: "agent-message-receipt",
            type: "agentMessage",
            status: "completed",
            text: "页面创建完成。",
        }, 1));

        render(<AiDrivenProgressTerminal/>);

        const receipts = screen.getByRole("region", {name: "画布动作回执"});
        expect(receipts).toHaveTextContent(/创建组件|create component/i);
        expect(receipts).toHaveTextContent("2 个组件");
        expect(receipts).toHaveTextContent("已生效");
        expect(screen.getByText("页面创建完成。")).toBeInTheDocument();
    });

    test("renders agent messages as Markdown", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("item.completed", {
            id: "agent-message-markdown",
            type: "agentMessage",
            status: "completed",
            text: [
                "**任务完成！** ✅",
                "",
                "| 调整项 | 变更内容 |",
                "|---|---|",
                "| **配色方案** | 银灰色渐变 |",
            ].join("\n"),
        }, 1));

        render(<AiDrivenProgressTerminal/>);

        expect(screen.getByText("任务完成！").tagName).toBe("STRONG");
        expect(screen.getByRole("table")).toHaveTextContent("调整项变更内容配色方案银灰色渐变");
        expect(screen.getByText("配色方案").tagName).toBe("STRONG");
    });

    test("removes loading markers when a run has completed", () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.applyTaskEvent(taskEvent("item.completed", {
            id: "plan-1",
            type: "plan",
            status: "completed",
            steps: [
                {step: "分析当前页面", status: "completed"},
                {step: "生成页面内容", status: "inProgress"},
            ],
        }, 1));
        aiDrivenProgressTerminalStore.finishRun("completed", "任务已完成");

        render(<AiDrivenProgressTerminal/>);

        expect(screen.getByRole("progressbar", {name: /任务进度|Task progress/})).toHaveAttribute("aria-valuenow", "100");
        expect(screen.queryByRole("img", {name: /执行中|Running/})).not.toBeInTheDocument();
    });
});
