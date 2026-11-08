/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
export interface IAiDrivenTokenDetailsAnchor {
    top: number;
    bottom: number;
    right: number;
}

export interface IAiDrivenTokenDetailsSize {
    width: number;
    height: number;
}

export interface IAiDrivenTokenDetailsViewport {
    width: number;
    height: number;
}

export interface IAiDrivenTokenDetailsPosition {
    top: number;
    left: number;
}

const VIEWPORT_GUTTER = 12;
const ANCHOR_GAP = 8;

/** Places the measured details panel inside the viewport, preferring the trigger's right edge and below it. */
export const positionAiDrivenTokenDetails = (
    anchor: IAiDrivenTokenDetailsAnchor,
    panel: IAiDrivenTokenDetailsSize,
    viewport: IAiDrivenTokenDetailsViewport,
): IAiDrivenTokenDetailsPosition => {
    const maxLeft = Math.max(VIEWPORT_GUTTER, viewport.width - VIEWPORT_GUTTER - panel.width);
    const left = clamp(anchor.right - panel.width, VIEWPORT_GUTTER, maxLeft);
    const belowTop = anchor.bottom + ANCHOR_GAP;
    const aboveTop = anchor.top - ANCHOR_GAP - panel.height;
    const maxTop = Math.max(VIEWPORT_GUTTER, viewport.height - VIEWPORT_GUTTER - panel.height);
    const top = belowTop + panel.height <= viewport.height - VIEWPORT_GUTTER
        ? belowTop
        : aboveTop >= VIEWPORT_GUTTER
            ? aboveTop
            : clamp(belowTop, VIEWPORT_GUTTER, maxTop);
    return {top, left};
};

const clamp = (value: number, min: number, max: number) => Math.min(Math.max(value, min), max);
