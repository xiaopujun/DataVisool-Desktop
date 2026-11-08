/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
/*
 * Copyright © 2023-2025 puyinzhen
 * All rights reserved.
 */

import {useEffect, useRef, useState} from "react";
import ReleaseNotesMarkdown from "@/comps/common-component/update-release-notes/ReleaseNotesMarkdown.tsx";

interface AiDrivenStreamingMessageProps {
    content: string;
    fallback: string;
    active: boolean;
    onContentChange: () => void;
}

/** 将事件批量文本平滑展开；Store 中仍保留完整原文。 */
const AiDrivenStreamingMessage = ({
    content,
    fallback,
    active,
    onContentChange,
}: AiDrivenStreamingMessageProps) => {
    // 首次挂载直接展示已有内容，避免历史回放重新播放打字动画。
    const [displayedText, setDisplayedText] = useState(content);
    const displayedRef = useRef(displayedText);
    const displayedCountRef = useRef(Array.from(content).length);
    const lastFrameAtRef = useRef(0);
    const characterCreditRef = useRef(1);
    const reducedMotionRef = useRef(window.matchMedia("(prefers-reduced-motion: reduce)").matches);

    displayedRef.current = displayedText;

    useEffect(() => {
        const target = Array.from(content);
        const current = displayedRef.current;
        if (!active || reducedMotionRef.current || !content.startsWith(current)) {
            displayedRef.current = content;
            displayedCountRef.current = target.length;
            setDisplayedText(content);
            return;
        }
        if (current === content) {
            lastFrameAtRef.current = 0;
            characterCreditRef.current = 1;
            return;
        }

        let frameId = 0;
        const revealNextFrame = (frameAt: number) => {
            const currentCount = displayedCountRef.current;
            const pendingCount = target.length - currentCount;
            if (pendingCount <= 0) {
                lastFrameAtRef.current = 0;
                characterCreditRef.current = 1;
                return;
            }

            const elapsed = lastFrameAtRef.current > 0
                ? Math.min(50, Math.max(0, frameAt - lastFrameAtRef.current))
                : 16;
            lastFrameAtRef.current = frameAt;
            characterCreditRef.current += elapsed * revealRate(pendingCount) / 1000;
            const revealCount = Math.min(pendingCount, Math.floor(characterCreditRef.current));
            if (revealCount > 0) {
                characterCreditRef.current -= revealCount;
                const nextText = target.slice(0, currentCount + revealCount).join("");
                displayedRef.current = nextText;
                displayedCountRef.current = currentCount + revealCount;
                setDisplayedText(nextText);
            }
            frameId = window.requestAnimationFrame(revealNextFrame);
        };
        frameId = window.requestAnimationFrame(revealNextFrame);
        return () => window.cancelAnimationFrame(frameId);
    }, [active, content]);

    useEffect(() => {
        onContentChange();
    }, [displayedText, onContentChange]);

    // 完成事件到达后直接使用最终快照，避免下一过程出现时文字仍停在半截。
    const renderedText = active ? displayedText : content;
    const revealing = active && displayedText !== content;
    return (
        <ReleaseNotesMarkdown content={renderedText}
                              fallback={fallback}
                              embedded
                              streaming={active || revealing}/>
    );
};

const revealRate = (pendingCount: number) => {
    return pendingCount > 160 ? 220
        : pendingCount > 80 ? 140
            : pendingCount > 32 ? 90
                : 60;
};

export default AiDrivenStreamingMessage;
