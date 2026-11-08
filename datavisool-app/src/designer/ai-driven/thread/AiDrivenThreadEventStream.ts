/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
/*
 * Copyright © 2023-2025 puyinzhen
 * All rights reserved.
 */

import {listen, type UnlistenFn} from "@tauri-apps/api/event";
import {getAiDrivenThreadEvents} from "@/api/AiDrivenThreadApi.ts";
import type {IAiDrivenThreadEvent} from "./AiDrivenThreadTypes.ts";

export const AI_DRIVEN_THREAD_EVENT_CHANNEL = "datavisool:ai-driven-task-event";
const THREAD_EVENT_PAGE_SIZE = 100;

export interface AiDrivenThreadEventScope {
    projectId: string;
    threadId: string;
    eventCursor: number;
}

export interface AiDrivenThreadEventCallbacks {
    onEvent: (event: IAiDrivenThreadEvent) => void;
    onCursor: (threadSequence: number) => void;
    onError?: (error: unknown) => void;
}

/** 先建立实时监听，再回放 Thread cursor；所有 Run 的事件都会参与 cursor 推进。 */
export class AiDrivenThreadEventStream {
    private version = 0;
    private unlisten: UnlistenFn | null = null;
    private replayController: AbortController | null = null;
    private cursor = 0;
    private pending = new Map<number, IAiDrivenThreadEvent>();

    async start(scope: AiDrivenThreadEventScope, callbacks: AiDrivenThreadEventCallbacks) {
        this.stop();
        const version = this.version;
        this.cursor = Math.max(0, scope.eventCursor);
        const consume = (event: IAiDrivenThreadEvent) => {
            // 这里只按 Thread 过滤；提前按 Turn/Run 过滤会制造 threadSequence 缺口。
            if (version !== this.version || event.threadId !== scope.threadId || event.threadSequence <= this.cursor)
                return;
            this.pending.set(event.threadSequence, event);
            this.flush(callbacks);
        };

        try {
            const unlisten = await listen<IAiDrivenThreadEvent>(AI_DRIVEN_THREAD_EVENT_CHANNEL, (event) => consume(event.payload));
            if (version !== this.version) {
                unlisten();
                return;
            }
            this.unlisten = unlisten;
        } catch (error) {
            callbacks.onError?.(error);
        }
        if (version !== this.version)
            return;

        const controller = new AbortController();
        this.replayController = controller;
        let replayCursor = Math.max(0, scope.eventCursor);
        try {
            while (version === this.version) {
                const result = await getAiDrivenThreadEvents({
                    projectId: scope.projectId,
                    threadId: scope.threadId,
                    afterThreadSequence: replayCursor,
                    limit: THREAD_EVENT_PAGE_SIZE,
                }, {signal: controller.signal, suppressErrorMessage: true});
                if (version !== this.version || controller.signal.aborted || !result)
                    return;
                result.data.forEach(consume);
                const nextCursor = result.nextCursor;
                if (nextCursor === null
                    || nextCursor <= replayCursor
                    || result.data.length < THREAD_EVENT_PAGE_SIZE)
                    return;
                replayCursor = nextCursor;
            }
        } catch (error) {
            if (!controller.signal.aborted)
                callbacks.onError?.(error);
        } finally {
            if (this.replayController === controller)
                this.replayController = null;
        }
    }

    stop() {
        this.version += 1;
        this.unlisten?.();
        this.unlisten = null;
        this.replayController?.abort();
        this.replayController = null;
        this.pending.clear();
    }

    private flush(callbacks: AiDrivenThreadEventCallbacks) {
        let event = this.pending.get(this.cursor + 1);
        while (event) {
            this.pending.delete(event.threadSequence);
            this.cursor = event.threadSequence;
            callbacks.onCursor(event.threadSequence);
            callbacks.onEvent(event);
            event = this.pending.get(this.cursor + 1);
        }
    }
}
