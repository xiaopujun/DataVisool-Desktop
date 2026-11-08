/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {cleanup, fireEvent, render, screen, waitFor} from "@testing-library/react";
import {afterEach, beforeEach, describe, expect, test, vi} from "vitest";

const storeMocks = vi.hoisted(() => ({
    setTargetIds: vi.fn(),
    focusDesignerCanvas: vi.fn(),
    setAddRecordCompId: vi.fn(),
}));

const rightMocks = vi.hoisted(() => ({activeConfig: vi.fn()}));

vi.mock("@/designer/operate-provider/EventOperateStore.ts", () => ({
    default: storeMocks,
}));

vi.mock("@/designer/right/RightStore.ts", () => ({default: rightMocks}));

vi.mock("@/designer/loader/DesignerLoader.ts", () => ({
    default: class DesignerLoaderMock {
        static definitionMap = {};
    },
}));

vi.mock("@/designer/operate-provider/undo-redo/HistoryRecordOperateProxy.ts", () => ({
    default: {doAdd: vi.fn()},
}));

import layerListStore from "@/designer/left/layer-list/LayerListStore.ts";
import {SearchLayer} from "@/designer/left/layer-list/search-layer/SearchLayer.tsx";

describe("SearchLayer management selection", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        (window as any).DATAVISOOL_ENV = {
            currentManager: {
                layerManager: {
                    layerConfigs: {
                        group: {id: "group", name: "父分组", type: "group", hide: true, lock: true, order: 1},
                        child: {id: "child", name: "目标图层", type: "rect", pid: "group", order: 2},
                    },
                },
                canvasManager: {canvasConfig: {width: 800, height: 600}},
            },
        };
        layerListStore.setSearchLayer(true);
    });

    afterEach(() => {
        cleanup();
        layerListStore.setSearchLayer(false);
        delete (window as any).DATAVISOOL_ENV;
    });

    test("shows effective ancestor state and locates a hidden locked layer through the search source", async () => {
        render(<SearchLayer/>);

        fireEvent.change(screen.getByPlaceholderText("请输入图层名、组件名或组件 Key"), {
            target: {value: "目标"},
        });

        expect(await screen.findByText("目标图层")).toBeInTheDocument();
        expect(screen.getByText("已隐藏")).toBeInTheDocument();
        expect(screen.getByText("已锁定")).toBeInTheDocument();

        fireEvent.click(screen.getByRole("button", {name: /目标图层/}));

        await waitFor(() => {
            expect(storeMocks.setTargetIds).toHaveBeenCalledWith(["child"], "search");
        });
        expect(rightMocks.activeConfig).toHaveBeenCalledWith("child", "rect");
    });
});
