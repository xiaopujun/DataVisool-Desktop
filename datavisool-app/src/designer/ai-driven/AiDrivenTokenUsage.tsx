/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {createPortal} from "react-dom";
import {forwardRef, useEffect, useId, useLayoutEffect, useRef, useState} from "react";
import {tx} from "@/i18n/miniI18n.ts";
import {positionAiDrivenTokenDetails, type IAiDrivenTokenDetailsPosition} from "./AiDrivenTokenUsageLayout.ts";
import type {IAiDrivenTokenTotals, IAiDrivenTokenUsage} from "./types/AiDrivenTaskSubmitTypes.ts";
import "./AiDrivenTokenUsage.less";

const formatCount = (value?: number | null) => value === undefined || value === null
    ? "—"
    : value.toLocaleString("en-US");

/** 顶部只展示本轮和会话总量，明细支持悬停、点击和键盘访问。 */
const AiDrivenTokenUsage = ({usage}: {usage: IAiDrivenTokenUsage}) => {
    const rootRef = useRef<HTMLDivElement>(null);
    const triggerRef = useRef<HTMLButtonElement>(null);
    const detailsRef = useRef<HTMLElement>(null);
    const hoverCloseTimerRef = useRef<number | null>(null);
    const [hovered, setHovered] = useState(false);
    const [focused, setFocused] = useState(false);
    const [pinned, setPinned] = useState(false);
    const [dismissed, setDismissed] = useState(false);
    const [position, setPosition] = useState<IAiDrivenTokenDetailsPosition | null>(null);
    const detailsId = useId();
    const open = !dismissed && (hovered || focused || pinned);

    useLayoutEffect(() => {
        if (!open) {
            setPosition(null);
            return;
        }
        const updatePosition = () => {
            const bounds = triggerRef.current?.getBoundingClientRect();
            const panelBounds = detailsRef.current?.getBoundingClientRect();
            if (!bounds || !panelBounds)
                return;
            const next = positionAiDrivenTokenDetails(
                bounds,
                panelBounds,
                {width: window.innerWidth, height: window.innerHeight},
            );
            setPosition((current) => current?.top === next.top && current.left === next.left ? current : next);
        };
        updatePosition();
        window.addEventListener("resize", updatePosition);
        window.addEventListener("scroll", updatePosition, true);
        return () => {
            window.removeEventListener("resize", updatePosition);
            window.removeEventListener("scroll", updatePosition, true);
        };
    }, [open, usage]);

    useEffect(() => {
        if (!open)
            return;
        const closeOnEscape = (event: KeyboardEvent) => {
            if (event.key !== "Escape")
                return;
            event.stopPropagation();
            setPinned(false);
            setDismissed(true);
            setFocused(false);
            triggerRef.current?.blur();
        };
        const closeOnOutsidePointer = (event: PointerEvent) => {
            const target = event.target as Node | null;
            if (rootRef.current?.contains(target) || detailsRef.current?.contains(target))
                return;
            setPinned(false);
            setDismissed(true);
            setHovered(false);
            setFocused(false);
            triggerRef.current?.blur();
        };
        document.addEventListener("keydown", closeOnEscape, true);
        document.addEventListener("pointerdown", closeOnOutsidePointer, true);
        return () => {
            document.removeEventListener("keydown", closeOnEscape, true);
            document.removeEventListener("pointerdown", closeOnOutsidePointer, true);
        };
    }, [open]);

    useEffect(() => () => {
        if (hoverCloseTimerRef.current !== null)
            window.clearTimeout(hoverCloseTimerRef.current);
    }, []);

    const handleMouseEnter = () => {
        if (hoverCloseTimerRef.current !== null)
            window.clearTimeout(hoverCloseTimerRef.current);
        hoverCloseTimerRef.current = null;
        setDismissed(false);
        setHovered(true);
    };

    const handleMouseLeave = () => {
        if (hoverCloseTimerRef.current !== null)
            window.clearTimeout(hoverCloseTimerRef.current);
        hoverCloseTimerRef.current = window.setTimeout(() => {
            hoverCloseTimerRef.current = null;
            setHovered(false);
        }, 120);
    };

    const activate = () => {
        if (pinned) {
            setPinned(false);
            setDismissed(true);
            setHovered(false);
            setFocused(false);
            triggerRef.current?.blur();
            return;
        }
        setDismissed(false);
        setPinned(true);
    };

    return (
        <div ref={rootRef}
             className="ai-driven-token-usage-wrap"
             onMouseEnter={handleMouseEnter}
             onMouseLeave={handleMouseLeave}>
            <button ref={triggerRef}
                    type="button"
                    className="ai-driven-run-token-usage"
                    onClick={activate}
                    onFocus={() => { setDismissed(false); setFocused(true); }}
                    onBlur={(event) => {
                        if (!rootRef.current?.contains(event.relatedTarget as Node | null))
                            setFocused(false);
                    }}
                    aria-label={tx("查看 Token 消耗明细")}
                    aria-haspopup="dialog"
                    aria-expanded={open}
                    aria-controls={detailsId}>
                <span className="is-total">
                    <em>{tx("本轮")}</em>
                    <b>{formatCount(usage.totalTokens)}</b>
                </span>
                <span className="ai-driven-run-token-divider" aria-hidden="true"/>
                <span>
                    <em>{tx("会话累计")}</em>
                    <b>{formatCount(usage.threadUsage?.totalTokens)}</b>
                </span>
            </button>
            {open && createPortal(
                <TokenDetails ref={detailsRef}
                              id={detailsId}
                              usage={usage}
                              position={position ?? {top: 12, left: 12}}
                              visible={position !== null}
                              onMouseEnter={handleMouseEnter}
                              onMouseLeave={handleMouseLeave}/>,
                document.body,
            )}
        </div>
    );
};

const TokenDetails = forwardRef<HTMLElement, {
    id: string;
    usage: IAiDrivenTokenUsage;
    position: IAiDrivenTokenDetailsPosition;
    visible: boolean;
    onMouseEnter: () => void;
    onMouseLeave: () => void;
}>(({id, usage, position, visible, onMouseEnter, onMouseLeave}, ref) => {
    const rows: Array<[string, keyof IAiDrivenTokenTotals]> = [
        ["输入", "inputTokens"], ["输出", "outputTokens"], ["总计", "totalTokens"],
    ];
    for (const [label, key] of [["缓存命中", "cachedInputTokens"], ["缓存新建", "cacheCreationInputTokens"]] as const) {
        if (usage[key] !== undefined || usage.threadUsage?.[key] !== undefined)
            rows.push([label, key]);
    }
    const metrics: Array<[string, string]> = [];
    if (usage.compactionCount !== undefined && usage.compactionCount > 0)
        metrics.push(["压缩次数", formatCount(usage.compactionCount)]);
    for (const [label, key] of [
        ["剩余 Token", "remainingTotalTokens"], ["剩余模型调用", "remainingModelCalls"],
        ["剩余工具调用", "remainingToolCalls"], ["剩余写入调用", "remainingWriteCalls"],
        ["剩余前端交互", "remainingFrontendInteractions"],
    ] as const) {
        const value = usage[key];
        if (value !== undefined && value !== null)
            metrics.push([label, formatCount(value)]);
    }
    if (usage.remainingDurationMs !== undefined && usage.remainingDurationMs !== null) {
        const seconds = Math.max(0, Math.floor(usage.remainingDurationMs / 1000));
        metrics.push(["剩余时长", `${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`]);
    }
    return (
        <section id={id}
                 ref={ref}
                 role="dialog"
                 aria-label={tx("Token 消耗明细")}
                 className="ai-driven-token-details"
                 style={{top: position.top, left: position.left, visibility: visible ? "visible" : "hidden"}}
                 onMouseEnter={onMouseEnter}
                 onMouseLeave={onMouseLeave}>
            <strong className="ai-driven-token-details-title">{tx("Token 消耗明细")}</strong>
            <table>
                <thead>
                <tr><th scope="col">{tx("消耗项")}</th><th scope="col">{tx("本轮")}</th><th scope="col">{tx("会话累计")}</th></tr>
                </thead>
                <tbody>
                {rows.map(([label, key]) => (
                    <tr key={key} className={key === "totalTokens" ? "is-total" : undefined}>
                        <th scope="row">{tx(label)}</th>
                        <td>{formatCount(usage[key])}</td>
                        <td>{formatCount(usage.threadUsage?.[key])}</td>
                    </tr>
                ))}
                </tbody>
            </table>
            {metrics.length > 0 && <dl className="ai-driven-token-context">
                {metrics.map(([label, value]) => (
                    <div key={label}><dt>{tx(label)}</dt><dd>{value}</dd></div>
                ))}
            </dl>}
            <p>{tx("会话累计包含本轮消耗，缓存是输入的子项。")}</p>
        </section>
    );
});

export default AiDrivenTokenUsage;
