/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {beforeEach, describe, expect, test, vi} from "vitest";

const rightStoreMocks = vi.hoisted(() => ({
    activeConfig: vi.fn(),
}));

vi.mock("@/designer/event/ContextMenuStore.ts", () => ({default: {}}));
vi.mock("@/designer/left/layer-list/util/LayerUtil.ts", () => ({default: {}}));
vi.mock("@/designer/right/RightStore.ts", () => ({default: rightStoreMocks}));

import eventOperateStore from "@/designer/operate-provider/EventOperateStore.ts";

describe("EventOperateStore", () => {
    beforeEach(() => {
        eventOperateStore.reset();
        vi.clearAllMocks();
    });

    test("resets the canvas scale before a new editor session", () => {
        eventOperateStore.setScale(0.05);
        eventOperateStore.setRatio(1 / 1.2);

        eventOperateStore.reset();

        expect(eventOperateStore.scale).toBe(1);
        expect(eventOperateStore.ratio).toBe(1);
    });
});
