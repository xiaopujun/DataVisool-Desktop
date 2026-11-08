/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import type {
    IAiDrivenTaskCancelPayload,
    IAiDrivenTaskCancelResult,
    IAiDrivenTaskEventsPayload,
    IAiDrivenTaskEventsResult,
    IAiDrivenTaskResumePayload,
    IAiDrivenTaskStatusPayload,
    IAiDrivenTaskSteerPayload,
    IAiDrivenTaskSteerResult,
    IAiDrivenTaskSubmitPayload,
    IAiDrivenTaskSubmitResult,
} from "@/designer/ai-driven/types/AiDrivenTaskSubmitTypes.ts";
import AuthFetchUtil, {type AuthRequestInit} from "@/util/AuthFetchUtil.ts";

/**
 * 提交 AI驱动控制台任务上下文到桌面端后端临时接口。
 */
export const submitAiDrivenTask = async (
    payload: IAiDrivenTaskSubmitPayload,
    options?: AuthRequestInit,
): Promise<IAiDrivenTaskSubmitResult | null> => {
    return await AuthFetchUtil.post<IAiDrivenTaskSubmitResult>(
        "/api/aiDriven/task/submit",
        payload,
        options,
    );
};

/** 显式恢复应用重启中断的任务。 */
export const resumeAiDrivenTask = async (
    payload: IAiDrivenTaskResumePayload,
    options?: AuthRequestInit,
): Promise<IAiDrivenTaskSubmitResult | null> => {
    return await AuthFetchUtil.post<IAiDrivenTaskSubmitResult>(
        "/api/aiDriven/task/resume",
        payload,
        options,
    );
};

/**
 * 请求停止一轮正在执行的 AI驱动任务。
 */
export const cancelAiDrivenTask = async (
    payload: IAiDrivenTaskCancelPayload,
    options?: AuthRequestInit,
): Promise<IAiDrivenTaskCancelResult | null> => {
    return await AuthFetchUtil.post<IAiDrivenTaskCancelResult>(
        "/api/aiDriven/task/cancel",
        payload,
        options,
    );
};

/**
 * 按 project + thread + turn + run 作用域查询持久化状态。
 */
export const getAiDrivenTaskStatus = async (
    payload: IAiDrivenTaskStatusPayload,
    options?: AuthRequestInit,
): Promise<IAiDrivenTaskSubmitResult | null> => {
    return await AuthFetchUtil.post<IAiDrivenTaskSubmitResult>(
        "/api/aiDriven/task/status",
        payload,
        options,
    );
};

/** 向当前活动 Turn 追加要求。 */
export const steerAiDrivenTask = async (
    payload: IAiDrivenTaskSteerPayload,
    options?: AuthRequestInit,
): Promise<IAiDrivenTaskSteerResult | null> => {
    return await AuthFetchUtil.post<IAiDrivenTaskSteerResult>(
        "/api/aiDriven/task/steer",
        payload,
        options,
    );
};

/** 按 cursor 回放任务事件，补齐实时监听前后的缺口。 */
export const getAiDrivenTaskEvents = async (
    payload: IAiDrivenTaskEventsPayload,
    options?: AuthRequestInit,
): Promise<IAiDrivenTaskEventsResult | null> => {
    return await AuthFetchUtil.post<IAiDrivenTaskEventsResult>(
        "/api/aiDriven/task/events",
        payload,
        options,
    );
};
