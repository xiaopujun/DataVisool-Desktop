/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {cleanup, render, screen} from "@testing-library/react";
import {afterEach, describe, expect, test} from "vitest";
import {reduceAiDrivenItemEvent} from "@/designer/ai-driven/thread/AiDrivenItemEventReducer.ts";
import AiDrivenItemView from "@/designer/ai-driven/thread/AiDrivenItemView.tsx";
import type {
    IAiDrivenItem,
    IAiDrivenThreadEvent,
} from "@/designer/ai-driven/thread/AiDrivenThreadTypes.ts";

const item = (itemType: string, payload: Record<string, unknown> = {}, itemId = `item-${itemType}`): IAiDrivenItem => ({
    itemId,
    turnId: "turn-a",
    itemSequence: 1,
    sourceRunId: "run-a",
    logicalKey: null,
    itemType,
    status: "completed",
    schemaVersion: 1,
    payload,
    lastEventSequence: 2,
    rowVersion: 2,
    createdAt: 1,
    updatedAt: 2,
    completedAt: 2,
});

const event = (threadSequence: number, kind: string, payload: Record<string, unknown>): IAiDrivenThreadEvent => ({
    eventId: `event-${threadSequence}`,
    threadId: "thread-a",
    turnId: "turn-a",
    itemId: "item-agentMessage",
    runId: "run-a",
    sequence: threadSequence,
    threadSequence,
    kind,
    payload,
    dedupeKey: `dedupe-${threadSequence}`,
    schemaVersion: 1,
    createdAt: threadSequence,
});

describe("AI driven item projection and view", () => {
    afterEach(cleanup);

    test("completed snapshot replaces accumulated deltas and blocks later deltas", () => {
        const active = {...item("agentMessage", {text: "正在"}), status: "inProgress" as const, lastEventSequence: 1, completedAt: null};
        const withDelta = reduceAiDrivenItemEvent([active], event(2, "item.delta", {
            itemType: "agentMessage",
            delta: "分析",
        }));
        const completed = item("agentMessage", {text: "分析完成"});
        const withSnapshot = reduceAiDrivenItemEvent(withDelta, event(3, "item.completed", {item: completed}));
        const afterLateDelta = reduceAiDrivenItemEvent(withSnapshot, event(4, "item.delta", {
            itemType: "agentMessage",
            delta: "不应出现",
        }));

        expect(withDelta[0].payload.text).toBe("正在分析");
        expect(withSnapshot[0].payload.text).toBe("分析完成");
        expect(afterLateDelta[0].payload.text).toBe("分析完成");
    });

    test("reads the current item identity and nested payload", () => {
        const projected = reduceAiDrivenItemEvent([], event(1, "item.started", {
            item: {
                itemId: "item-new",
                itemType: "agentMessage",
                status: "inProgress",
                payload: {text: "当前文本"},
            },
        }));

        expect(projected).toHaveLength(1);
        expect(projected[0].itemId).toBe("item-new");
        expect(projected[0].itemType).toBe("agentMessage");
        expect(projected[0].payload).toEqual({text: "当前文本"});
    });

    test("accepts a completed item snapshot placed directly in event payload", () => {
        const active = {
            ...item("agentMessage", {text: "处理中"}),
            status: "inProgress" as const,
            lastEventSequence: 1,
            completedAt: null,
        };
        const projected = reduceAiDrivenItemEvent([active], event(2, "item.completed", {
            itemId: "item-agentMessage",
            itemType: "agentMessage",
            status: "completed",
            payload: {text: "直接完成"},
        }));

        expect(projected[0].status).toBe("completed");
        expect(projected[0].payload.text).toBe("直接完成");
    });

    test("renders all known item types with safe summaries", () => {
        const knownItems = [
            item("userMessage", {text: "生成页面"}),
            item("agentMessage", {text: "已完成"}),
            item("plan", {steps: [{step: "生成结构", status: "completed"}]}),
            item("toolCall", {toolName: "create_canvas_components"}),
            item("toolCall", {toolName: "declare_execution_scope"}, "item-toolCall-legacy-scope"),
            item("canvasOperation", {kind: "createComponents", status: "applied", componentIds: ["component-1"]}),
            item("canvasOperation", {kind: "deleteComponents", status: "rejected", componentIds: ["component-2"]}, "item-canvasOperation-delete"),
            item("userInput", {prompt: "请选择组件"}),
            item("artifactChange", {artifactId: "artifact-a"}),
            item("preview", {previewId: "preview-a"}),
            item("approval", {reason: "需要确认"}),
            item("error", {code: "agent.failed", message: "执行失败"}),
        ];

        const {container} = render(<>{knownItems.map((value) => <AiDrivenItemView key={value.itemId} item={value}/>)}</>);

        expect(container.querySelectorAll(".ai-driven-history-item")).toHaveLength(12);
        expect(screen.getByText("生成页面")).toBeInTheDocument();
        expect(screen.getByText("create_canvas_components")).toBeInTheDocument();
        expect(screen.getByText("declare_execution_scope")).toBeInTheDocument();
        expect(screen.getByText("已真实生效")).toBeInTheDocument();
        expect(screen.getByText("删除组件")).toBeInTheDocument();
        expect(screen.getByText("用户已拒绝，未修改画布")).toBeInTheDocument();
        expect(screen.getByText("执行失败")).toBeInTheDocument();
    });

    test("does not expose raw payload for an unknown item type", () => {
        render(<AiDrivenItemView item={item("futureItem", {secret: "do-not-render"})}/>);

        expect(screen.getByText("futureItem")).toBeInTheDocument();
        expect(screen.queryByText("do-not-render")).not.toBeInTheDocument();
        expect(screen.getByText("暂不支持展示此类型的详细内容")).toBeInTheDocument();
    });
});
