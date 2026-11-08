/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {action, computed, makeObservable, observable} from "mobx";
import {tx} from "@/i18n/miniI18n.ts";
import type {
    IAiDrivenCanvasOperationReceipt,
    IAiDrivenRuntimeSnapshot,
    IAiDrivenTaskEvent,
    IAiDrivenTokenTotals,
    IAiDrivenTokenUsage,
} from "./types/AiDrivenTaskSubmitTypes.ts";
import {
    aiDrivenTaskNoticeKind,
    describeAiDrivenTaskEvent,
    type AiDrivenTaskNoticeKind,
} from "./AiDrivenTaskEventStream.ts";

export type AiDrivenProgressLogLevel = "info" | "success" | "warning" | "error";

const AI_DRIVEN_PROGRESS_LOG_RENDER_INTERVAL_MS = 150;

export interface IAiDrivenProgressLogInput {
    /** 日志级别，用于控制终端行的状态颜色。 */
    level?: AiDrivenProgressLogLevel;
    /** 日志内容，空内容会被忽略。 */
    message: string;
    /** 日志时间文本，未传入时使用当前时间。 */
    timeText?: string;
}

export interface IAiDrivenProgressLogEntry {
    /** 日志行稳定 id，用于追加渲染时维持 React key。 */
    id: number;
    /** 日志产生时间，按终端窗口里的短时间格式展示。 */
    timeText: string;
    /** 日志级别，用于区分普通进度、成功、警告和错误状态。 */
    level: AiDrivenProgressLogLevel;
    /** 当前进度日志的用户可读内容。 */
    message: string;
}

export type AiDrivenItemStatus = "inProgress" | "completed" | "failed";
export type AiDrivenPlanStepStatus = "pending" | "inProgress" | "completed";
export type AiDrivenTimelineItemStatus =
    | "created"
    | "inProgress"
    | "waiting"
    | "completed"
    | "failed"
    | "cancelled"
    | "declined";
export type AiDrivenRunState = "idle" | "running" | "waiting" | "completed" | "failed" | "cancelled" | "budgetExhausted";

export interface IAiDrivenAgentMessageItem {
    id: string;
    status: AiDrivenItemStatus;
    text: string;
    truncated: boolean;
}

export interface IAiDrivenTimelineItem {
    id: string;
    type: string;
    status: AiDrivenTimelineItemStatus;
    payload: Record<string, unknown>;
    createdAt: number;
    updatedAt: number;
}

export interface IAiDrivenPlanStep {
    step: string;
    status: AiDrivenPlanStepStatus;
}

export interface IAiDrivenRunResult {
    status: "completed" | "failed" | "cancelled" | "budgetExhausted";
    message: string;
    endedAt: number;
}

export interface IAiDrivenRunNotice {
    kind: AiDrivenTaskNoticeKind;
    message: string;
}

export interface IAiDrivenInlineInteractionCandidate {
    componentType: string;
    displayName: string;
    description?: string;
    defaultSelected: boolean;
}

export type AiDrivenInlineInteractionRequest =
    | {
        kind: "userInput";
        title: string;
        questions: Array<{
            header: string;
            id: string;
            question: string;
            options: Array<{label: string; description: string}>;
        }>;
    }
    | {
        kind: "preview";
        title: string;
        message: string;
        draftId: string;
        layerCount: number;
    }
    | {
        kind: "componentSelection";
        title: string;
        message: string;
        candidates: IAiDrivenInlineInteractionCandidate[];
        minSelected: number;
        maxSelected: number;
        initialSelected: string[];
    };

export type IAiDrivenInlineInteraction = AiDrivenInlineInteractionRequest & {id: string};

export interface IAiDrivenInlineInteractionCompletion {
    action: "submit" | "cancel" | "abort" | "unavailable";
    data?: unknown;
}

const MAX_AGENT_MESSAGE_ITEMS = 12;
const MAX_AGENT_MESSAGE_LENGTH = 32_000;
const MAX_TIMELINE_ITEMS = 80;

// 固定使用 HH:mm:ss，避免不同浏览器 locale 输出导致终端列宽抖动。
const createAiDrivenProgressLogTimeText = () => {
    const date = new Date();
    const padTime = (value: number) => String(value).padStart(2, "0");
    return `${padTime(date.getHours())}:${padTime(date.getMinutes())}:${padTime(date.getSeconds())}`;
};

class AiDrivenProgressTerminalStore {
    constructor() {
        makeObservable(this, {
            visible: observable,
            progressTitle: observable,
            displayProgressTitle: computed,
            logs: observable.shallow,
            agentMessages: observable.shallow,
            timelineItems: observable.shallow,
            canvasOperations: observable.shallow,
            planExplanation: observable,
            planSteps: observable.shallow,
            runState: observable,
            runStartedAt: observable,
            runResult: observable.ref,
            runNotice: observable.ref,
            tokenUsage: observable.ref,
            pendingInlineInteraction: observable.ref,
            setVisible: action,
            toggleVisible: action,
            setProgressTitle: action,
            startRun: action,
            finishRun: action,
            appendLog: action,
            applyTaskEvent: action,
            setRuntimeSnapshot: action,
            setCanvasOperations: action,
            setTokenUsage: action,
            renderNextPendingLog: action,
            clearLogs: action,
            clearLiveItems: action,
            setInlineHostAvailable: action,
            requestInlineInteraction: action,
            completeInlineInteraction: action,
            reset: action,
        });
    }

    /** 控制 AI驱动进度终端是否显示。 */
    visible: boolean = false;

    /** 当前任务进度自定义标题，空值时展示默认标题。 */
    progressTitle: string = "";

    /** 当前任务进度展示标题，用于终端头部和入口按钮提示。 */
    get displayProgressTitle() {
        if (this.progressTitle)
            return this.progressTitle;
        if (this.runResult?.message)
            return this.runResult.message;
        if (this.runState === "running")
            return tx("Agent 正在处理当前任务");
        if (this.runState === "waiting")
            return tx("等待你的操作后继续");
        return `${tx("当前进度")}：${tx("待开始")}`;
    }

    /** 当前终端日志列表，供外部前端工具持续追加。 */
    logs: IAiDrivenProgressLogEntry[] = [];

    /** 由 item.* 事件聚合得到的 Agent 文本。 */
    agentMessages: IAiDrivenAgentMessageItem[] = [];

    /** 当前 Turn 的可展示 Item 时间线。 */
    timelineItems: IAiDrivenTimelineItem[] = [];

    /** 与最终文字分离展示的真实画布动作回执。 */
    canvasOperations: IAiDrivenCanvasOperationReceipt[] = [];

    /** Agent 最近一次提交的结构化计划说明。 */
    planExplanation: string = "";

    /** Agent 最近一次提交的结构化计划步骤。 */
    planSteps: IAiDrivenPlanStep[] = [];

    /** 当前任务面向用户的运行状态。 */
    runState: AiDrivenRunState = "idle";

    /** 当前任务开始时间，用于展示真实耗时。 */
    runStartedAt: number = 0;

    /** 当前任务终态摘要。 */
    runResult: IAiDrivenRunResult | null = null;

    /** 最近一次由宿主事实说明的等待、拒绝或执行问题。 */
    runNotice: IAiDrivenRunNotice | null = null;

    /** 当前任务累计 Token 用量。 */
    tokenUsage: IAiDrivenTokenUsage | null = null;

    /** 当前由 Agent 发起、可在任务面板中完成的交互。 */
    pendingInlineInteraction: IAiDrivenInlineInteraction | null = null;

    /** 等待按节奏渲染到终端的日志队列。 */
    private pendingLogs: IAiDrivenProgressLogEntry[] = [];

    /** 当前日志渲染定时器，用于保证日志逐条展示。 */
    private renderTimer: ReturnType<typeof setTimeout> | null = null;

    /** 下一条日志的自增 id，保证日志 key 稳定。 */
    private nextLogId: number = 0;

    /** 当前任务唯一键，避免恢复监听时重复清空时间线。 */
    private activeRunKey: string = "";

    /** 内联交互宿主是否处于可展示状态。 */
    private inlineHostAvailable: boolean = false;

    /** 当前内联交互 Promise 的完成函数。 */
    private inlineInteractionResolver: ((result: IAiDrivenInlineInteractionCompletion) => void) | null = null;

    /** 当前内联交互取消监听清理函数。 */
    private inlineInteractionAbortCleanup: (() => void) | null = null;

    /** 内联交互自增 id。 */
    private nextInlineInteractionId: number = 0;

    /** 设置 AI驱动进度终端显示状态。 */
    setVisible = (visible: boolean) => {
        this.visible = visible;
    };

    /** 切换 AI驱动进度终端显示状态。 */
    toggleVisible = () => {
        this.visible = !this.visible;
    };

    /** 设置当前任务进度标题，空标题会回退到默认标题。 */
    setProgressTitle = (progressTitle: string) => {
        this.progressTitle = progressTitle.trim();
    };

    /** 开始展示一个新任务；同一任务恢复监听时保留已有内容。 */
    startRun = (runKey: string, runState: "running" | "waiting" = "running") => {
        const normalizedRunKey = runKey.trim();
        if (!normalizedRunKey)
            return;
        if (this.activeRunKey !== normalizedRunKey) {
            this.clearLogs();
            this.clearLiveItems();
            this.tokenUsage = {inputTokens: 0, outputTokens: 0, totalTokens: 0};
            this.runNotice = null;
            this.progressTitle = "";
            this.activeRunKey = normalizedRunKey;
            this.runStartedAt = Date.now();
        }
        this.runState = runState;
        this.runResult = null;
        this.visible = true;
    };

    /** 收口任务终态并保留时间线，便于用户复核结果。 */
    finishRun = (status: IAiDrivenRunResult["status"], message: string) => {
        this.runState = status;
        this.runNotice = null;
        this.runResult = {
            status,
            message: message.trim() || defaultResultMessage(status),
            endedAt: Date.now(),
        };
        if (this.pendingInlineInteraction)
            this.completeInlineInteraction({action: "abort"});
    };

    /** 向 AI驱动进度终端追加一条日志。 */
    appendLog = (log: IAiDrivenProgressLogInput) => {
        if (!log.message.trim())
            return;

        this.pendingLogs.push({
            id: ++this.nextLogId,
            timeText: log.timeText?.trim() || createAiDrivenProgressLogTimeText(),
            level: log.level ?? "info",
            message: log.message,
        });

        // 首条日志立即展示；后续日志通过定时器保持 400ms 间隔。
        if (this.logs.length === 0 && this.pendingLogs.length === 1 && !this.renderTimer) {
            this.renderNextPendingLog();
            return;
        }
        this.scheduleNextLogRender();
    };

    /** 聚合统一 item.* 事件，不依赖后端内部 Agent 事件名称。 */
    applyTaskEvent = (event: IAiDrivenTaskEvent) => {
        this.setTokenUsage(event.payload.usage);
        this.applyRunNotice(event);
        this.applyRunStateEvent(event);
        this.applyCanvasOperationEvent(event);
        if (event.kind === "agent.completion.fact_check") {
            this.discardRejectedCompletionCandidate(event.payload);
            return;
        }
        if (event.kind === "item.delta") {
            const item = asRecord(event.payload.item);
            if (item && itemType(item) === "agentMessage") {
                this.applyAgentMessageItem(item, event.createdAt);
            } else {
                this.applyAgentMessageDelta(event.payload, event.createdAt);
            }
            return;
        }
        if (event.kind !== "item.started"
            && event.kind !== "item.completed"
            && event.kind !== "item.failed")
            return;

        const item = asRecord(event.payload.item);
        if (!item)
            return;
        const type = itemType(item);
        if (type === "agentMessage")
            this.applyAgentMessageItem(item, event.createdAt);
        else if (type === "plan" && event.kind === "item.completed")
            this.applyPlanItem(item);
        else if (type && type !== "plan")
            this.replaceTimelineItem(parseTimelineItem(item, event.createdAt));
    };

    /** 用任务状态接口返回的 runtime 快照恢复动作和 Token 用量。 */
    setRuntimeSnapshot = (runtime?: IAiDrivenRuntimeSnapshot) => {
        if (!runtime)
            return;
        this.setCanvasOperations(runtime.operations);
        this.setTokenUsage(runtime.usage);
    };

    /** 用任务状态接口返回的 operation journal 快照恢复真实动作。 */
    setCanvasOperations = (operations: IAiDrivenCanvasOperationReceipt[] = []) => {
        const unique = new Map<string, IAiDrivenCanvasOperationReceipt>();
        operations.forEach((operation) => {
            if (operation.operationId)
                unique.set(operation.operationId, operation);
        });
        this.canvasOperations = [...unique.values()];
    };

    /** 合并 Token 用量快照；累计值不回退，省略的部分字段沿用最新值。 */
    setTokenUsage = (usage: unknown) => {
        const nextUsage = parseTokenUsage(usage);
        if (!nextUsage)
            return;
        const current: IAiDrivenTokenUsage = this.tokenUsage ?? {inputTokens: 0, outputTokens: 0, totalTokens: 0};
        const {threadUsage: incomingThreadUsage, ...topUsage} = nextUsage;
        const incomingTotals = tokenTotals(topUsage);
        const currentIsOlder = incomingTotals !== null && isOlderTotals(current, incomingTotals);
        const merged: IAiDrivenTokenUsage = currentIsOlder ? {...current} : {...current, ...topUsage};
        if (incomingThreadUsage) {
            const currentThreadUsage = current.threadUsage;
            const mergedThreadUsage = {...currentThreadUsage, ...incomingThreadUsage};
            if (hasCompleteTotals(mergedThreadUsage)
                && (!currentThreadUsage || !isOlderTotals(currentThreadUsage, incomingThreadUsage)))
                merged.threadUsage = mergedThreadUsage;
        }
        this.tokenUsage = merged as IAiDrivenTokenUsage;
    };

    /** 渲染下一条待展示日志，并继续安排下一次渲染。 */
    renderNextPendingLog = () => {
        this.renderTimer = null;
        const nextLog = this.pendingLogs.shift();
        if (!nextLog)
            return;

        this.logs = [...this.logs, nextLog];
        this.scheduleNextLogRender();
    };

    /** 清空 AI驱动进度终端日志。 */
    clearLogs = () => {
        this.cancelLogRenderTimer();
        this.pendingLogs = [];
        this.logs = [];
        this.nextLogId = 0;
    };

    /** 清空仅属于当前任务的流式文本和计划。 */
    clearLiveItems = () => {
        this.agentMessages = [];
        this.timelineItems = [];
        this.canvasOperations = [];
        this.planExplanation = "";
        this.planSteps = [];
        this.runResult = null;
        this.runNotice = null;
        this.tokenUsage = null;
    };

    /** 注册或释放内联交互宿主；宿主消失时交回原弹窗链路。 */
    setInlineHostAvailable = (available: boolean) => {
        this.inlineHostAvailable = available;
        if (!available && this.pendingInlineInteraction)
            this.completeInlineInteraction({action: "unavailable"});
    };

    /** 在任务时间线中发起一次交互；面板不可用时返回 null，由调用方降级到弹窗。 */
    requestInlineInteraction = (
        request: AiDrivenInlineInteractionRequest,
        signal?: AbortSignal,
    ): Promise<IAiDrivenInlineInteractionCompletion> | null => {
        if (!this.inlineHostAvailable || this.pendingInlineInteraction)
            return null;
        if (signal?.aborted)
            return Promise.resolve({action: "abort"});

        this.pendingInlineInteraction = {
            ...request,
            id: `ai-driven-inline-${++this.nextInlineInteractionId}`,
        } as IAiDrivenInlineInteraction;
        this.runState = "waiting";
        this.visible = true;

        return new Promise((resolve) => {
            this.inlineInteractionResolver = resolve;
            if (!signal)
                return;
            const handleAbort = () => this.completeInlineInteraction({action: "abort"});
            signal.addEventListener("abort", handleAbort, {once: true});
            this.inlineInteractionAbortCleanup = () => signal.removeEventListener("abort", handleAbort);
        });
    };

    /** 完成当前内联交互，并恢复任务运行状态。 */
    completeInlineInteraction = (completion: IAiDrivenInlineInteractionCompletion) => {
        const resolve = this.inlineInteractionResolver;
        this.inlineInteractionAbortCleanup?.();
        this.inlineInteractionAbortCleanup = null;
        this.inlineInteractionResolver = null;
        this.pendingInlineInteraction = null;
        if (this.runState === "waiting")
            this.runState = "running";
        resolve?.(completion);
    };

    /** 重置 AI驱动进度终端的显示状态、标题和日志。 */
    reset = () => {
        if (this.pendingInlineInteraction)
            this.completeInlineInteraction({action: "abort"});
        this.visible = false;
        this.progressTitle = "";
        this.runState = "idle";
        this.runStartedAt = 0;
        this.activeRunKey = "";
        this.clearLogs();
        this.clearLiveItems();
    };

    private applyAgentMessageDelta = (payload: Record<string, unknown>, createdAt: number) => {
        if (payload.itemType !== "agentMessage")
            return;
        const id = stringValue(payload.itemId);
        const delta = stringValue(payload.delta);
        if (!id || !delta)
            return;

        const current = this.agentMessages.find((item) => item.id === id);
        const next: IAiDrivenAgentMessageItem = current
            ? {...current, text: `${current.text}${delta}`.slice(0, MAX_AGENT_MESSAGE_LENGTH)}
            : {id, status: "inProgress", text: delta.slice(0, MAX_AGENT_MESSAGE_LENGTH), truncated: false};
        this.replaceAgentMessage(next);
        this.replaceTimelineItem({
            id,
            type: "agentMessage",
            status: next.status,
            payload: {text: next.text, truncated: next.truncated},
            createdAt: this.timelineItems.find((item) => item.id === id)?.createdAt ?? createdAt,
            updatedAt: createdAt,
        });
    };

    private applyAgentMessageItem = (item: Record<string, unknown>, createdAt: number) => {
        const id = stringValue(item.itemId);
        if (!id)
            return;
        const current = this.agentMessages.find((message) => message.id === id);
        const payload = asRecord(item.payload);
        const text = typeof payload?.text === "string" ? payload.text : current?.text ?? "";
        this.replaceAgentMessage({
            id,
            status: itemStatus(item.status),
            text: text.slice(0, MAX_AGENT_MESSAGE_LENGTH),
            truncated: payload?.truncated === true,
        });
        this.replaceTimelineItem(parseTimelineItem(item, createdAt));
    };

    private replaceAgentMessage = (next: IAiDrivenAgentMessageItem) => {
        const currentIndex = this.agentMessages.findIndex((item) => item.id === next.id);
        const messages = currentIndex >= 0
            ? this.agentMessages.map((item, index) => index === currentIndex ? next : item)
            : [...this.agentMessages, next];
        this.agentMessages = messages.slice(-MAX_AGENT_MESSAGE_ITEMS);
    };

    /** 完成门未通过时，撤销刚刚流式展示但未被宿主接受的结束候选。 */
    private discardRejectedCompletionCandidate = (payload: Record<string, unknown>) => {
        const check = asRecord(payload.check);
        if (!check || check.status === "passed")
            return;
        const candidate = this.timelineItems.at(-1);
        if (!candidate || candidate.type !== "agentMessage" || candidate.status !== "completed")
            return;
        this.agentMessages = this.agentMessages.filter((message) => message.id !== candidate.id);
        this.timelineItems = this.timelineItems.filter((item) => item.id !== candidate.id);
    };

    private applyPlanItem = (item: Record<string, unknown>) => {
        const payload = asRecord(item.payload);
        const steps = Array.isArray(payload?.steps)
            ? payload.steps.map(parsePlanStep).filter((step): step is IAiDrivenPlanStep => Boolean(step))
            : [];
        if (steps.length === 0)
            return;
        this.planExplanation = stringValue(payload?.explanation);
        this.planSteps = steps;
    };

    private replaceTimelineItem = (next: IAiDrivenTimelineItem | null) => {
        if (!next)
            return;
        const currentIndex = this.timelineItems.findIndex((item) => item.id === next.id);
        const items = currentIndex >= 0
            ? this.timelineItems.map((item, index) => index === currentIndex
                ? {...next, createdAt: item.createdAt}
                : item)
            : [...this.timelineItems, next];
        this.timelineItems = items.slice(-MAX_TIMELINE_ITEMS);
    };

    private applyCanvasOperationEvent = (event: IAiDrivenTaskEvent) => {
        if (!event.kind.startsWith("agent.canvas_operation."))
            return;
        const operation = parseCanvasOperation(event.payload.operation);
        if (!operation)
            return;
        const currentIndex = this.canvasOperations.findIndex((item) => item.operationId === operation.operationId);
        this.canvasOperations = currentIndex >= 0
            ? this.canvasOperations.map((item, index) => index === currentIndex ? operation : item)
            : [...this.canvasOperations, operation];
    };

    private applyRunStateEvent = (event: IAiDrivenTaskEvent) => {
        switch (event.kind) {
            case "agent.started":
            case "turn.started":
            case "agent.resumed":
            case "turn.resumed":
            case "turn.verifying":
                this.runNotice = null;
                this.runState = "running";
                break;
            case "agent.waiting_user":
            case "turn.waiting":
                this.runState = "waiting";
                break;
            case "turn.completed":
                this.finishRun("completed", stringValue(event.payload.message));
                break;
            case "turn.failed":
                this.finishRun("failed", stringValue(event.payload.message));
                break;
            case "turn.budget_exhausted":
            case "turn.budgetExhausted":
            case "agent.budget_exhausted":
                this.finishRun("budgetExhausted", stringValue(event.payload.message));
                break;
            case "turn.cancelled":
            case "turn.interrupted":
                this.finishRun("cancelled", stringValue(event.payload.message));
                break;
        }
    };

    private applyRunNotice = (event: IAiDrivenTaskEvent) => {
        const kind = aiDrivenTaskNoticeKind(event);
        if (!kind)
            return;
        const message = describeAiDrivenTaskEvent(event);
        if (message)
            this.runNotice = {kind, message};
    };

    /** 安排下一条日志的延迟渲染。 */
    private scheduleNextLogRender = () => {
        if (this.renderTimer || this.pendingLogs.length === 0)
            return;

        this.renderTimer = setTimeout(
            this.renderNextPendingLog,
            AI_DRIVEN_PROGRESS_LOG_RENDER_INTERVAL_MS,
        );
    };

    /** 取消尚未触发的日志渲染定时器。 */
    private cancelLogRenderTimer = () => {
        if (!this.renderTimer)
            return;

        clearTimeout(this.renderTimer);
        this.renderTimer = null;
    };
}

const asRecord = (value: unknown): Record<string, unknown> | null => {
    return value !== null && typeof value === "object" && !Array.isArray(value)
        ? value as Record<string, unknown>
        : null;
};

const stringValue = (value: unknown) => typeof value === "string" ? value : "";

type IAiDrivenTokenUsagePatch = Partial<Omit<IAiDrivenTokenUsage, "threadUsage">> & {
    threadUsage?: Partial<IAiDrivenTokenTotals>;
};

const parseTokenUsage = (value: unknown): IAiDrivenTokenUsagePatch | null => {
    const usage = asRecord(value);
    if (!usage)
        return null;
    const parsed: IAiDrivenTokenUsagePatch = {...parseTokenTotals(usage)};
    const threadUsage = asRecord(usage.threadUsage);
    const parsedThreadUsage = threadUsage ? parseTokenTotals(threadUsage) : null;
    if (parsedThreadUsage)
        parsed.threadUsage = parsedThreadUsage;

    for (const field of [
        "contextInputTokens", "activeContextTokens", "contextCapacityTokens", "compactionCount",
        "compactionBeforeTokens", "compactionAfterTokens",
    ] as const) {
        if (!(field in usage))
            continue;
        const count = tokenCount(usage[field]);
        if (count !== null)
            parsed[field] = count;
    }
    if (typeof usage.contextCapacitySource === "string")
        parsed.contextCapacitySource = usage.contextCapacitySource;
    for (const field of ["tokenBudgetEnabled", "callBudgetEnabled", "durationBudgetEnabled"] as const) {
        const enabled = booleanValue(usage[field]);
        if (enabled !== null)
            parsed[field] = enabled;
    }
    Object.assign(parsed, remainingUsageFields(usage));
    return Object.keys(parsed).length === 0 ? null : parsed;
};

const parseTokenTotals = (usage: Record<string, unknown>): Partial<IAiDrivenTokenTotals> | null => {
    const parsed: Partial<IAiDrivenTokenTotals> = {};
    for (const field of ["inputTokens", "outputTokens", "totalTokens", "cachedInputTokens", "cacheCreationInputTokens"] as const) {
        if (!(field in usage))
            continue;
        const count = tokenCount(usage[field]);
        if (count !== null)
            parsed[field] = count;
    }
    if (parsed.totalTokens === undefined
        && parsed.inputTokens !== undefined
        && parsed.outputTokens !== undefined)
        parsed.totalTokens = parsed.inputTokens + parsed.outputTokens;
    return Object.keys(parsed).length === 0 ? null : parsed;
};

const tokenTotals = (usage: IAiDrivenTokenUsagePatch): Partial<IAiDrivenTokenTotals> | null => {
    const totals: Partial<IAiDrivenTokenTotals> = {};
    for (const field of ["inputTokens", "outputTokens", "totalTokens", "cachedInputTokens", "cacheCreationInputTokens"] as const) {
        const value = usage[field];
        if (value !== undefined)
            totals[field] = value;
    }
    return Object.keys(totals).length === 0 ? null : totals;
};

const isOlderTotals = (
    current: Partial<IAiDrivenTokenTotals>,
    incoming: Partial<IAiDrivenTokenTotals>,
) => {
    return (["inputTokens", "outputTokens", "totalTokens", "cachedInputTokens", "cacheCreationInputTokens"] as const)
        .some((field) => current[field] !== undefined
            && incoming[field] !== undefined
            && incoming[field]! < current[field]!);
};

const hasCompleteTotals = (totals: Partial<IAiDrivenTokenTotals>): totals is IAiDrivenTokenTotals => {
    return totals.inputTokens !== undefined
        && totals.outputTokens !== undefined
        && totals.totalTokens !== undefined;
};

const remainingUsageFields = (usage: Record<string, unknown> | null): Partial<IAiDrivenTokenUsage> => {
    if (!usage)
        return {};
    const fields: Array<[keyof IAiDrivenTokenUsage, string]> = [
        ["remainingModelCalls", "remainingModelCalls"],
        ["remainingToolCalls", "remainingToolCalls"],
        ["remainingWriteCalls", "remainingWriteCalls"],
        ["remainingTotalTokens", "remainingTotalTokens"],
        ["remainingFrontendInteractions", "remainingFrontendInteractions"],
        ["remainingDurationMs", "remainingDurationMs"],
    ];
    return Object.fromEntries(fields.flatMap(([field, source]) => {
        if (!(source in usage))
            return [];
        const value = usage[source] === null ? null : tokenCount(usage[source]);
        return [[field, value]];
    })) as Partial<IAiDrivenTokenUsage>;
};

const booleanValue = (value: unknown) => typeof value === "boolean" ? value : null;

const tokenCount = (value: unknown) => {
    return typeof value === "number" && Number.isFinite(value) && value >= 0
        ? Math.floor(value)
        : null;
};

const parseCanvasOperation = (value: unknown): IAiDrivenCanvasOperationReceipt | null => {
    const operation = asRecord(value);
    const operationId = stringValue(operation?.operationId);
    const toolCallId = stringValue(operation?.toolCallId);
    const projectId = stringValue(operation?.projectId);
    const kind = operation?.kind;
    const status = operation?.status;
    if (!operationId || !toolCallId || !projectId
        || (kind !== "createComponents" && kind !== "patchComponents" && kind !== "deleteComponents")
        || !["prepared", "applying", "applied", "rejected", "failed"].includes(String(status)))
        return null;
    return operation as unknown as IAiDrivenCanvasOperationReceipt;
};

const itemStatus = (value: unknown): AiDrivenItemStatus => {
    return value === "completed" || value === "failed" ? value : "inProgress";
};

const timelineItemStatus = (value: unknown): AiDrivenTimelineItemStatus => {
    switch (value) {
        case "created":
        case "waiting":
        case "completed":
        case "failed":
        case "cancelled":
        case "declined":
            return value;
        default:
            return "inProgress";
    }
};

const itemType = (item: Record<string, unknown>) => stringValue(item.itemType);

const parseTimelineItem = (item: Record<string, unknown>, createdAt: number): IAiDrivenTimelineItem | null => {
    const id = stringValue(item.itemId);
    const type = itemType(item);
    if (!id || !type)
        return null;
    const payload = asRecord(item.payload) ?? {};
    return {
        id,
        type,
        status: timelineItemStatus(item.status),
        payload,
        createdAt,
        updatedAt: createdAt,
    };
};

const defaultResultMessage = (status: IAiDrivenRunResult["status"]) => {
    if (status === "completed")
        return "任务已完成，请核对画布动作和最终回复";
    if (status === "cancelled")
        return "任务已停止，已完成的步骤仍会保留";
    if (status === "budgetExhausted")
        return "预算已用尽，可增加预算后继续";
    return "任务执行失败，请查看运行详情后重试";
};

const parsePlanStep = (value: unknown): IAiDrivenPlanStep | null => {
    const item = asRecord(value);
    const step = stringValue(item?.step).trim();
    const status = item?.status;
    const normalizedStatus = status === "in_progress" ? "inProgress" : status;
    if (!step || (normalizedStatus !== "pending" && normalizedStatus !== "inProgress" && normalizedStatus !== "completed"))
        return null;
    return {step, status: normalizedStatus};
};

const aiDrivenProgressTerminalStore = new AiDrivenProgressTerminalStore();
export default aiDrivenProgressTerminalStore;
