/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {cleanup, fireEvent, render, screen} from "@testing-library/react";
import {afterEach, describe, expect, test, vi} from "vitest";
import AiDrivenTurnView from "@/designer/ai-driven/thread/AiDrivenTurnView.tsx";
import type {IAiDrivenTurn} from "@/designer/ai-driven/thread/AiDrivenThreadTypes.ts";

vi.mock("@/i18n/miniI18n.ts", () => ({tx: (message: string) => message}));
vi.mock("@/designer/ai-driven/thread/AiDrivenItemView.tsx", () => ({default: () => null}));

const turn = (limits?: IAiDrivenTurn["limits"]): IAiDrivenTurn => ({
    turnId: "turn-a",
    projectId: "project-a",
    threadId: "thread-a",
    turnSequence: 1,
    requestId: "request-a",
    status: "budgetExhausted",
    limits,
    checkpointVersion: 4,
    recoverable: true,
    request: {},
    inputSummary: "创建图表",
    modelConfigId: "model-config",
    runtimeModelId: "runtime-model",
    provider: "openai",
    entryDigest: "sha256:a",
    usage: {
        totalTokens: 900,
        modelCalls: 6,
        toolCalls: 3,
        writeCalls: 1,
        frontendInteractions: 2,
        elapsedMs: 120_000,
    },
    lastArtifactId: null,
    lastArtifactRevision: null,
    errorCode: "agent_execution.token_limit",
    errorMessage: "预算耗尽",
    rowVersion: 2,
    createdAt: 1,
    startedAt: 2,
    updatedAt: 3,
    endedAt: 3,
});

const renderTurn = (currentTurn: IAiDrivenTurn, onResume = vi.fn()) => render(
    <AiDrivenTurnView
        turn={currentTurn}
        items={[]}
        expanded
        loadingItems={false}
        itemsLoaded
        hasMoreItems={false}
        onToggle={vi.fn()}
        onLoadMoreItems={vi.fn()}
        onResume={onResume}
    />,
);

describe("AI driven budget exhausted history turn", () => {
    afterEach(cleanup);

    test("shows only enabled authorized budget fields with current limit and usage", () => {
        const onResume = vi.fn();
        renderTurn(turn({
            tokenBudgetEnabled: true,
            callBudgetEnabled: false,
            durationBudgetEnabled: false,
            maxTotalTokens: 1_000,
            maxModelCalls: 10,
            maxToolCalls: 20,
            maxWriteCalls: 8,
            maxDurationMs: 600_000,
            maxFrontendInteractions: 6,
        }), onResume);

        expect(screen.getByLabelText("累计 Token 上限")).toBeInTheDocument();
        expect(screen.queryByLabelText("模型调用上限")).not.toBeInTheDocument();
        expect(screen.getByText(/当前上限 1,000，已用 900/)).toBeInTheDocument();

        fireEvent.change(screen.getByLabelText("累计 Token 上限"), {target: {value: "2000"}});
        fireEvent.click(screen.getByRole("button", {name: "增加预算后继续"}));

        expect(onResume).toHaveBeenCalledWith({maxTotalTokens: 2_000});
    });

    test("does not invent extension fields when historical authorization is unavailable", () => {
        renderTurn(turn());

        expect(screen.getByText("预算授权信息不可用，请刷新历史记录后重试")).toBeInTheDocument();
        expect(screen.queryAllByRole("spinbutton")).toHaveLength(0);
    });

    test("does not render fields when a legacy limit snapshot has no authorization flags", () => {
        renderTurn(turn({
            maxTotalTokens: 1_000,
            maxModelCalls: 10,
            maxToolCalls: 20,
            maxWriteCalls: 8,
            maxFrontendInteractions: 6,
            maxDurationMs: 600_000,
        } as IAiDrivenTurn["limits"]));

        expect(screen.getByText("预算授权信息不可用，请刷新历史记录后重试")).toBeInTheDocument();
        expect(screen.queryAllByRole("spinbutton")).toHaveLength(0);
    });
});
