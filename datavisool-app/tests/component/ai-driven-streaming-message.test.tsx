/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {act, render, screen} from "@testing-library/react";
import {afterEach, beforeEach, describe, expect, test, vi} from "vitest";
import AiDrivenStreamingMessage from "@/designer/ai-driven/AiDrivenStreamingMessage.tsx";

describe("AI driven streaming message", () => {
    let callbacks: FrameRequestCallback[];
    let nextFrameId: number;

    beforeEach(() => {
        callbacks = [];
        nextFrameId = 0;
        vi.stubGlobal("requestAnimationFrame", vi.fn((callback: FrameRequestCallback) => {
            callbacks.push(callback);
            return ++nextFrameId;
        }));
        vi.stubGlobal("cancelAnimationFrame", vi.fn());
    });

    afterEach(() => vi.unstubAllGlobals());

    test("reveals a newly received batch over multiple frames", () => {
        const {rerender} = render(
            <AiDrivenStreamingMessage content=""
                                      fallback="正在生成回复…"
                                      active
                                      onContentChange={vi.fn()}/>
        );
        rerender(
            <AiDrivenStreamingMessage content="这是一段需要平滑展示的流式文本"
                                      fallback="正在生成回复…"
                                      active
                                      onContentChange={vi.fn()}/>
        );

        expect(screen.queryByText("这是一段需要平滑展示的流式文本")).not.toBeInTheDocument();
        runFrame(callbacks, 16);
        expect(document.body).toHaveTextContent("这");
        expect(screen.queryByText("这是一段需要平滑展示的流式文本")).not.toBeInTheDocument();

        for (let frame = 2; frame <= 40; frame += 1)
            runFrame(callbacks, frame * 16);
        expect(screen.getByText("这是一段需要平滑展示的流式文本")).toBeInTheDocument();
        expect(document.querySelector("[aria-hidden='true']")).toBeInTheDocument();
    });

    test("shows restored content immediately on first mount", () => {
        render(
            <AiDrivenStreamingMessage content="历史消息直接展示"
                                      fallback="正在生成回复…"
                                      active={false}
                                      onContentChange={vi.fn()}/>
        );

        expect(screen.getByText("历史消息直接展示")).toBeInTheDocument();
        expect(callbacks).toHaveLength(0);
        expect(document.querySelector("[aria-hidden='true']")).not.toBeInTheDocument();
    });

    test("shows the complete snapshot immediately when streaming finishes", () => {
        const content = "这是一段尚未播放完成但已经结束的流式文本";
        const {rerender} = render(
            <AiDrivenStreamingMessage content=""
                                      fallback="正在生成回复…"
                                      active
                                      onContentChange={vi.fn()}/>
        );
        rerender(
            <AiDrivenStreamingMessage content={content}
                                      fallback="正在生成回复…"
                                      active
                                      onContentChange={vi.fn()}/>
        );
        runFrame(callbacks, 16);
        expect(screen.queryByText(content)).not.toBeInTheDocument();

        rerender(
            <AiDrivenStreamingMessage content={content}
                                      fallback="正在生成回复…"
                                      active={false}
                                      onContentChange={vi.fn()}/>
        );

        expect(screen.getByText(content)).toBeInTheDocument();
        expect(document.querySelector("[aria-hidden='true']")).not.toBeInTheDocument();
        expect(cancelAnimationFrame).toHaveBeenCalled();
    });
});

const runFrame = (callbacks: FrameRequestCallback[], timestamp: number) => {
    const callback = callbacks.shift();
    if (!callback)
        return;
    act(() => callback(timestamp));
};
