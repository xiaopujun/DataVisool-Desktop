/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {globalMessage} from "@datavisool/ui";
import {resumeAiDrivenTask} from "@/api/AiDrivenTaskApi.ts";
import {tx} from "@/i18n/miniI18n.ts";
import aiDrivenProgressTerminalStore from "./AiDrivenProgressTerminalStore.ts";
import {
    clearActiveTaskScope,
    clearPendingTaskScope,
    isActiveTaskStatus,
    persistActiveTaskScope,
    persistPendingTaskScope,
    taskErrorMessage,
    type AiDrivenTaskRunMonitor,
    type PersistedAiDrivenTaskScope,
} from "./AiDrivenTaskRunMonitor.ts";
import type {IAiDrivenTurn} from "./thread/AiDrivenThreadTypes.ts";
import type {IAiDrivenBudgetExtension} from "./types/AiDrivenTaskSubmitTypes.ts";

export interface AiDrivenResumeHost {
    running: boolean;
    mounted: boolean;
    currentRequestId: string;
    aiDrivenSessionId: string;
    taskRunMonitor: Pick<AiDrivenTaskRunMonitor, "stop" | "watch">;
    startRunningTask: (requestId: string, projectId: string, threadId: string, turnId?: string, runId?: string) => void;
    acceptRunningTask: (requestId: string, threadId: string, turnId: string, runId: string) => void;
    finishRunningTask: (requestId: string) => void;
}

/** 从历史面板显式恢复应用重启中断的任务。 */
export const resumeInterruptedTask = async (
    host: AiDrivenResumeHost,
    projectId: string | null,
    turn: IAiDrivenTurn,
    budgetExtension?: IAiDrivenBudgetExtension,
) => {
    if (host.running) {
        globalMessage.messageApi?.warning(tx("请先等待当前任务结束"));
        return;
    }
    if (!projectId || (turn.status !== "interrupted" && turn.status !== "budgetExhausted"))
        return;
    if (turn.status === "budgetExhausted" && !budgetExtension) {
        globalMessage.messageApi?.warning(tx("请先明确增加至少一项预算后继续"));
        return;
    }

    const pendingScope = {requestId: turn.requestId, projectId, threadId: turn.threadId};
    host.taskRunMonitor.stop();
    clearActiveTaskScope(projectId);
    persistPendingTaskScope(pendingScope);
    host.startRunningTask(turn.requestId, projectId, turn.threadId, turn.turnId);
    aiDrivenProgressTerminalStore.appendLog({
        level: "info",
        message: turn.status === "budgetExhausted" ? tx("正在使用新增预算继续任务") : tx("正在恢复中断任务"),
    });
    try {
        const result = await resumeAiDrivenTask({
            projectId,
            threadId: turn.threadId,
            turnId: turn.turnId,
            sessionId: host.aiDrivenSessionId,
            ...(turn.checkpointVersion === undefined ? {} : {expectedCheckpointVersion: turn.checkpointVersion}),
            ...(budgetExtension === undefined ? {} : {budgetExtension}),
        }, {suppressErrorMessage: true});
        if (host.currentRequestId !== turn.requestId)
            return;
        clearPendingTaskScope(projectId);
        if (!result) {
            globalMessage.messageApi?.error(tx("任务恢复失败"));
            host.finishRunningTask(turn.requestId);
            return;
        }
        aiDrivenProgressTerminalStore.setRuntimeSnapshot(result.runtime);
        if (isActiveTaskStatus(result.status)) {
            const scope: PersistedAiDrivenTaskScope = {
                ...pendingScope,
                turnId: result.turnId,
                runId: result.runId,
                eventCursor: result.eventCursor,
                status: result.status,
            };
            persistActiveTaskScope(scope);
            host.acceptRunningTask(turn.requestId, result.threadId, result.turnId, result.runId);
            if (host.mounted)
                host.taskRunMonitor.watch(scope);
            return;
        }
        const message = taskErrorMessage(result);
        if (result.status === "budgetExhausted") {
            aiDrivenProgressTerminalStore.appendLog({level: "warning", message: tx("预算仍已用尽，请增加新的累计上限")});
            aiDrivenProgressTerminalStore.finishRun("budgetExhausted", message === "提交失败"
                ? tx("预算已用尽，可增加预算后继续")
                : message);
        } else {
            globalMessage.messageApi?.error(message);
            aiDrivenProgressTerminalStore.finishRun("failed", message);
        }
        host.finishRunningTask(turn.requestId);
    } catch (error) {
        console.error("AI driven task resume failed:", error);
        clearPendingTaskScope(projectId);
        if (host.currentRequestId === turn.requestId)
            host.finishRunningTask(turn.requestId);
    }
};
