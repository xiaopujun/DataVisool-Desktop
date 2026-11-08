/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {act, cleanup, fireEvent, render, screen, waitFor, within} from "@testing-library/react";
import {afterEach, beforeEach, describe, expect, test, vi} from "vitest";
import {positionAiDrivenTokenDetails} from "@/designer/ai-driven/AiDrivenTokenUsageLayout.ts";
import type {IAiDrivenTaskEvent} from "@/designer/ai-driven/types/AiDrivenTaskSubmitTypes.ts";

vi.mock("@/i18n/miniI18n.ts", () => ({tx: (value: string) => value}));
vi.mock("@/i18n/useLocaleVersion.ts", () => ({useLocaleVersion: () => 0}));

import Terminal from "@/designer/ai-driven/AiDrivenProgressTerminal.tsx";
import store from "@/designer/ai-driven/AiDrivenProgressTerminalStore.ts";

const event = (kind: string, payload: Record<string, unknown> = {}): IAiDrivenTaskEvent => ({
    eventId: "event",
    threadId: "thread",
    turnId: "turn",
    runId: "run",
    projectId: "project",
    requestId: "request",
    kind,
    payload,
    sequence: 1,
    threadSequence: 1,
    dedupeKey: "event",
    schemaVersion: 3,
    createdAt: Date.now(),
});

const usage = {inputTokens: 100, outputTokens: 20, totalTokens: 120};

describe("token details layout", () => {
    test("clamps a measured wide panel to both viewport gutters and flips above the trigger", () => {
        const position = positionAiDrivenTokenDetails(
            {top: 550, bottom: 570, right: 230},
            {width: 330, height: 200},
            {width: 360, height: 640},
        );
        expect(position).toEqual({left: 12, top: 342});
        expect(position.left).toBeGreaterThanOrEqual(12);
        expect(position.left + 330).toBeLessThanOrEqual(360 - 12);
        expect(position.top).toBeGreaterThanOrEqual(12);
        expect(position.top + 200).toBeLessThanOrEqual(640 - 12);
    });

    test("uses remaining vertical space when the panel fits neither above nor below", () => {
        const position = positionAiDrivenTokenDetails(
            {top: 280, bottom: 300, right: 340},
            {width: 280, height: 600},
            {width: 360, height: 640},
        );
        expect(position).toEqual({left: 60, top: 28});
        expect(position.top + 600).toBeLessThanOrEqual(640 - 12);
    });
});

describe("desktop token usage display", () => {
    beforeEach(() => {
        store.reset();
        vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) =>
            window.setTimeout(() => callback(performance.now()), 16));
        vi.spyOn(window, "cancelAnimationFrame").mockImplementation((id) => window.clearTimeout(id));
        store.startRun("request");
    });
    afterEach(() => {
        cleanup();
        store.reset();
        vi.restoreAllMocks();
    });

    test("shows compact totals and opens the current-run and session details", () => {
        store.applyTaskEvent(event("agent.model_call.completed", {usage: {
            ...usage,
            cachedInputTokens: 60,
            cacheCreationInputTokens: 10,
            contextInputTokens: 500,
            activeContextTokens: 100,
            contextCapacityTokens: 200_000,
            compactionCount: 2,
            remainingTotalTokens: 1_000,
            remainingModelCalls: 3,
            remainingToolCalls: 5,
            remainingWriteCalls: 2,
            remainingFrontendInteractions: 1,
            remainingDurationMs: 60_000,
            threadUsage: {inputTokens: 1_000, outputTokens: 200, totalTokens: 1_200, cachedInputTokens: 600},
        }}));
        render(<Terminal/>);
        const trigger = screen.getByRole("button", {name: "查看 Token 消耗明细"});
        expect(within(trigger).getByText("本轮")).toBeInTheDocument();
        expect(within(trigger).getByText("120")).toBeInTheDocument();
        expect(within(trigger).getByText("会话累计")).toBeInTheDocument();
        expect(within(trigger).getByText("1,200")).toBeInTheDocument();
        expect(screen.queryByText("输入")).toBeNull();
        expect(screen.queryByText("当前上下文")).toBeNull();

        fireEvent.click(trigger);
        const details = screen.getByRole("dialog", {name: "Token 消耗明细"});
        for (const label of ["缓存命中", "缓存新建", "压缩次数", "剩余 Token", "剩余模型调用",
            "剩余工具调用", "剩余写入调用", "剩余前端交互", "剩余时长"])
            expect(within(details).getByText(label)).toBeInTheDocument();
        expect(within(details).queryByText("当前上下文")).toBeNull();
        expect(within(details).getByRole("row", {name: "输入 100 1,000"})).toBeInTheDocument();
        expect(within(details).getByRole("row", {name: "缓存命中 60 600"})).toBeInTheDocument();
    });

    test("uses dashes for absent session data and cache details", () => {
        store.setTokenUsage(usage);
        render(<Terminal/>);
        const trigger = screen.getByRole("button", {name: "查看 Token 消耗明细"});
        expect(within(trigger).getByText("—")).toBeInTheDocument();
        fireEvent.click(trigger);
        const details = screen.getByRole("dialog", {name: "Token 消耗明细"});
        expect(within(details).queryByText("缓存命中")).toBeNull();
        expect(within(details).getByRole("row", {name: "输入 100 —"})).toBeInTheDocument();
    });

    test("uses the rendered panel dimensions when placing details in a narrow viewport", () => {
        const widthDescriptor = Object.getOwnPropertyDescriptor(window, "innerWidth");
        const heightDescriptor = Object.getOwnPropertyDescriptor(window, "innerHeight");
        Object.defineProperty(window, "innerWidth", {configurable: true, value: 360});
        Object.defineProperty(window, "innerHeight", {configurable: true, value: 640});
        const originalGetBoundingClientRect = HTMLElement.prototype.getBoundingClientRect;
        vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
            if (this.classList.contains("ai-driven-run-token-usage"))
                return makeRect(80, 550, 150, 20);
            if (this.classList.contains("ai-driven-token-details"))
                return makeRect(0, 0, 330, 200);
            return originalGetBoundingClientRect.call(this);
        });
        try {
            store.setTokenUsage(usage);
            render(<Terminal/>);
            fireEvent.click(screen.getByRole("button", {name: "查看 Token 消耗明细"}));
            const details = screen.getByRole("dialog", {name: "Token 消耗明细"});
            expect(details).toHaveStyle({left: "12px", top: "342px"});
            expect(Number.parseFloat(details.style.left)).toBeGreaterThanOrEqual(12);
            expect(Number.parseFloat(details.style.left) + 330).toBeLessThanOrEqual(348);
        } finally {
            if (widthDescriptor)
                Object.defineProperty(window, "innerWidth", widthDescriptor);
            if (heightDescriptor)
                Object.defineProperty(window, "innerHeight", heightDescriptor);
        }
    });

    test("remeasures and flips the open panel when token metrics expand its content", () => {
        const widthDescriptor = Object.getOwnPropertyDescriptor(window, "innerWidth");
        const heightDescriptor = Object.getOwnPropertyDescriptor(window, "innerHeight");
        Object.defineProperty(window, "innerWidth", {configurable: true, value: 360});
        Object.defineProperty(window, "innerHeight", {configurable: true, value: 640});
        const originalGetBoundingClientRect = HTMLElement.prototype.getBoundingClientRect;
        const measuredHeights: number[] = [];
        vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
            if (this.classList.contains("ai-driven-run-token-usage"))
                return makeRect(180, 420, 160, 20);
            if (this.classList.contains("ai-driven-token-details")) {
                const height = this.querySelector(".ai-driven-token-context") ? 300 : 100;
                measuredHeights.push(height);
                return makeRect(0, 0, 300, height);
            }
            return originalGetBoundingClientRect.call(this);
        });
        try {
            store.setTokenUsage(usage);
            render(<Terminal/>);
            fireEvent.click(screen.getByRole("button", {name: "查看 Token 消耗明细"}));
            const details = screen.getByRole("dialog", {name: "Token 消耗明细"});
            expect(details).toHaveStyle({top: "448px"});

            act(() => store.setTokenUsage({...usage, cachedInputTokens: 60, cacheCreationInputTokens: 10,
                compactionCount: 2, remainingTotalTokens: 1_000, remainingModelCalls: 3,
                remainingToolCalls: 5, remainingWriteCalls: 2, remainingFrontendInteractions: 1,
                remainingDurationMs: 60_000}));

            expect(details.querySelector(".ai-driven-token-context")).toBeInTheDocument();
            expect(details).toHaveStyle({top: "112px"});
            expect(measuredHeights).toEqual(expect.arrayContaining([100, 300]));
        } finally {
            if (widthDescriptor)
                Object.defineProperty(window, "innerWidth", widthDescriptor);
            if (heightDescriptor)
                Object.defineProperty(window, "innerHeight", heightDescriptor);
        }
    });

    test("opens on hover and focus, then closes on Escape and outside pointer", async () => {
        store.setTokenUsage(usage);
        render(<Terminal/>);
        const trigger = screen.getByRole("button", {name: "查看 Token 消耗明细"});
        fireEvent.mouseEnter(trigger);
        const hoverDetails = screen.getByRole("dialog", {name: "Token 消耗明细"});
        expect(hoverDetails).toBeInTheDocument();
        fireEvent.mouseLeave(trigger);
        fireEvent.mouseEnter(hoverDetails);
        expect(hoverDetails).toBeInTheDocument();
        fireEvent.mouseLeave(hoverDetails);
        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

        fireEvent.focus(trigger);
        expect(screen.getByRole("dialog", {name: "Token 消耗明细"})).toBeInTheDocument();
        fireEvent.keyDown(document, {key: "Escape"});
        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
        expect(trigger).toHaveAttribute("aria-expanded", "false");

        fireEvent.click(trigger);
        expect(screen.getByRole("dialog", {name: "Token 消耗明细"})).toBeInTheDocument();
        fireEvent.click(trigger);
        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
        fireEvent.click(trigger);
        expect(screen.getByRole("dialog", {name: "Token 消耗明细"})).toBeInTheDocument();
        act(() => fireEvent.pointerDown(document.body));
        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    });

    test("merges partial cache data and rejects older totals without losing newer details", () => {
        store.setTokenUsage({...usage, cachedInputTokens: 60, cacheCreationInputTokens: 10,
            threadUsage: {inputTokens: 1_000, outputTokens: 200, totalTokens: 1_200, cachedInputTokens: 600}});
        store.setTokenUsage({cachedInputTokens: 75, threadUsage: {cachedInputTokens: 650}});
        expect(store.tokenUsage).toMatchObject({
            ...usage,
            cachedInputTokens: 75,
            cacheCreationInputTokens: 10,
            threadUsage: {inputTokens: 1_000, outputTokens: 200, totalTokens: 1_200, cachedInputTokens: 650},
        });

        store.setTokenUsage({...usage, totalTokens: 2, cachedInputTokens: 1, activeContextTokens: 900,
            threadUsage: {inputTokens: 900, outputTokens: 100, totalTokens: 1_000, cachedInputTokens: 100}});
        expect(store.tokenUsage).toMatchObject({
            inputTokens: 100,
            outputTokens: 20,
            totalTokens: 120,
            cachedInputTokens: 75,
            threadUsage: {totalTokens: 1_200, cachedInputTokens: 650},
        });
        expect(store.tokenUsage?.activeContextTokens).toBeUndefined();
    });

    test("ignores duplicate snapshots, resists older session totals, and resets between requests", () => {
        const snapshot = {...usage, threadUsage: {inputTokens: 1_000, outputTokens: 200, totalTokens: 1_200}};
        store.setRuntimeSnapshot({usage: snapshot, operations: []});
        store.applyTaskEvent(event("agent.model_call.completed", {usage: snapshot}));
        store.applyTaskEvent(event("agent.model_call.completed", {usage: snapshot}));
        store.setTokenUsage({...usage, threadUsage: {inputTokens: 900, outputTokens: 180, totalTokens: 1_080}});
        expect(store.tokenUsage?.threadUsage?.totalTokens).toBe(1_200);

        store.startRun("next-request");
        expect(store.tokenUsage).toMatchObject({inputTokens: 0, outputTokens: 0, totalTokens: 0});
        expect(store.tokenUsage?.threadUsage).toBeUndefined();
        store.setTokenUsage({...usage, threadUsage: {inputTokens: 100, outputTokens: 20, totalTokens: 120}});
        store.startRun("different-session-request");
        expect(store.tokenUsage?.threadUsage).toBeUndefined();
    });
});

const makeRect = (left: number, top: number, width: number, height: number) => ({
    x: left,
    y: top,
    top,
    right: left + width,
    bottom: top + height,
    left,
    width,
    height,
    toJSON: () => ({}),
}) as DOMRect;
