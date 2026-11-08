/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, describe, expect, test, vi} from "vitest";
import {runApplyComponentPatchTool} from "@/frontend-effect/tools/ApplyComponentPatchTool.ts";
import {ObjectUtil} from "@datavisool/sdk/utils";

const approvalTicket = {approvalId: "test-approval", nonce: "test-nonce"};

describe("applyComponentPatch tool", () => {
    afterEach(() => {
        delete (window as any).DATAVISOOL_ENV;
    });

    test("updates only the existing target without adding layers", async () => {
        const config: any = {
            base: {id: "image-real", type: "BaseImage"},
            style: {onLineUrl: "old", opacity: 0.8},
            data: {sourceType: "static", staticData: {value: "old"}, value: "old"},
        };
        const update = vi.fn((patch: any) => {
            config.style = {...config.style, ...patch.style};
            config.data = {...config.data, ...patch.data};
        });
        const layerConfigs = {
            "image-real": {id: "image-real", type: "BaseImage"},
            "text-real": {id: "text-real", type: "BaseText"},
        };
        (window as any).DATAVISOOL_ENV = {
            currentManager: {
                layerManager: {
                    layerConfigs,
                    controllers: {"image-real": {getConfig: () => config, update}},
                },
            },
        };

        const result = await runApplyComponentPatchTool({
            approvalTicket,
            patches: [{
                componentId: "image-real",
                componentType: "BaseImage",
                stylePatch: {style: {onLineUrl: "green"}},
                dataPatch: {sourceType: "static", staticData: {value: "green"}},
            }],
            expectedSnapshots: [{
                componentId: "image-real",
                componentType: "BaseImage",
                style: {onLineUrl: "old", opacity: 0.8},
                data: {sourceType: "static", staticData: {value: "old"}, value: "old"},
            }],
        });

        expect(Object.keys(layerConfigs)).toHaveLength(2);
        expect(update).toHaveBeenCalledWith({
            style: {onLineUrl: "green", opacity: 0.8},
            data: {
                value: "green",
                sourceType: "static",
                staticData: {value: "green"},
            },
        }, {reRender: true});
        expect(result).toMatchObject({
            applied: [{componentId: "image-real", componentType: "BaseImage"}],
            failed: [],
        });
    });

    test("replaces malformed array data with the normalized data config", async () => {
        const oldData = [{year: "1月", value: 1, category: "旧系列"}];
        const nextValue = [
            {year: "1月", value: 10, category: "系列一"},
            {year: "1月", value: 20, category: "系列二"},
        ];
        const config: any = {
            base: {id: "chart-real", type: "AntdMultiLine"},
            style: {
                data: oldData,
                xField: "year",
                yField: "value",
                seriesField: "category",
            },
            data: oldData,
        };
        const update = vi.fn((patch: any) => {
            ObjectUtil.merge(config, patch);
            if (Array.isArray(config.data?.value))
                config.style.data = config.data.value;
        });
        (window as any).DATAVISOOL_ENV = {
            currentManager: {
                layerManager: {
                    layerConfigs: {"chart-real": {id: "chart-real", type: "AntdMultiLine"}},
                    controllers: {"chart-real": {getConfig: () => config, update}},
                },
            },
        };

        const result = await runApplyComponentPatchTool({
            approvalTicket,
            patches: [{
                componentId: "chart-real",
                componentType: "AntdMultiLine",
                dataPatch: {sourceType: "static", staticData: {value: nextValue}},
            }],
            expectedSnapshots: [{
                componentId: "chart-real",
                componentType: "AntdMultiLine",
                data: structuredClone(oldData),
            }],
        });

        expect(result.failed).toEqual([]);
        expect(Array.isArray(config.data)).toBe(false);
        expect(config.data).toEqual({
            sourceType: "static",
            staticData: {value: nextValue},
            value: nextValue,
        });
        expect(config.style.data).toEqual(nextValue);
    });

    test("same operation id applies a patch only once", async () => {
        const config: any = {base: {id: "text-real", type: "BaseText"}, style: {color: "red"}};
        const update = vi.fn((patch: any) => { config.style = patch.style; });
        (window as any).DATAVISOOL_ENV = {
            currentManager: {
                layerManager: {
                    layerConfigs: {"text-real": {id: "text-real", type: "BaseText"}},
                    controllers: {"text-real": {getConfig: () => config, update}},
                },
            },
        };
        const payload = {
            operationId: "run-patch:call-1",
            operationFingerprint: "sha256:patch",
            patches: [{componentId: "text-real", componentType: "BaseText", stylePatch: {style: {color: "green"}}}],
            expectedSnapshots: [{componentId: "text-real", componentType: "BaseText", style: {color: "red"}}],
        };

        const first = await runApplyComponentPatchTool(payload);
        const second = await runApplyComponentPatchTool(payload);

        expect(first.failed).toEqual([]);
        expect(second).toEqual(first);
        expect(update).toHaveBeenCalledTimes(1);
    });

    test("rejects the whole batch when a target baseline is stale", async () => {
        const configs: Record<string, any> = {
            A: {base: {type: "BaseText"}, style: {color: "red"}},
            B: {base: {type: "BaseText"}, style: {color: "blue"}},
        };
        const updates = {A: vi.fn(), B: vi.fn()};
        (window as any).DATAVISOOL_ENV = {
            currentManager: {
                layerManager: {
                    layerConfigs: {A: {id: "A", type: "BaseText"}, B: {id: "B", type: "BaseText"}},
                    controllers: {
                        A: {getConfig: () => configs.A, update: updates.A},
                        B: {getConfig: () => configs.B, update: updates.B},
                    },
                },
            },
        };

        const result = await runApplyComponentPatchTool({
            patches: [
                {componentId: "A", componentType: "BaseText", stylePatch: {style: {color: "green"}}},
                {componentId: "B", componentType: "BaseText", stylePatch: {style: {color: "green"}}},
            ],
            expectedSnapshots: [
                {componentId: "A", componentType: "BaseText", style: {color: "red"}},
                {componentId: "B", componentType: "BaseText", style: {color: "stale"}},
            ],
        });

        expect(updates.A).not.toHaveBeenCalled();
        expect(updates.B).not.toHaveBeenCalled();
        expect(result.applied).toEqual([]);
        expect(result.failed).toHaveLength(2);
    });

    test("rolls back every target when one update fails", async () => {
        const original = {
            layerConfigs: {A: {id: "A", type: "BaseText"}, B: {id: "B", type: "BaseText"}},
            elemConfigs: {},
        };
        const configs: Record<string, any> = {
            A: {base: {type: "BaseText"}, style: {color: "red"}},
            B: {base: {type: "BaseText"}, style: {color: "blue"}},
        };
        const init = vi.fn();
        const layerManager = {
            layerConfigs: original.layerConfigs,
            controllers: {
                A: {
                    getConfig: () => configs.A,
                    update: (patch: any) => { configs.A.style = patch.style; },
                },
                B: {
                    getConfig: () => configs.B,
                    update: () => { throw new Error("injected failure"); },
                },
            },
            getData: () => structuredClone(original),
            init,
        };
        (window as any).DATAVISOOL_ENV = {currentManager: {layerManager}};

        const result = await runApplyComponentPatchTool({
            patches: [
                {componentId: "A", componentType: "BaseText", stylePatch: {style: {color: "green"}}},
                {componentId: "B", componentType: "BaseText", stylePatch: {style: {color: "green"}}},
            ],
            expectedSnapshots: [
                {componentId: "A", componentType: "BaseText", style: {color: "red"}},
                {componentId: "B", componentType: "BaseText", style: {color: "blue"}},
            ],
        });

        expect(init).toHaveBeenCalledWith(original);
        expect(configs.A.style).toEqual({color: "red"});
        expect(result.applied).toEqual([]);
        expect(result.failed.every((failure) => failure.code === "batch_apply_rolled_back")).toBe(true);
    });

    test("applies only whitelisted layer and filter fields", async () => {
        const layer: any = {id: "A", type: "BaseText", x: 10, y: 20, width: 100, height: 40, name: "旧名称"};
        const config: any = {
            base: {type: "BaseText"},
            filter: {enable: false, blur: 0, brightness: 1, contrast: 1, opacity: 1, saturate: 1, hueRotate: 0},
        };
        const updateLayer = vi.fn((items: any[]) => Object.assign(layer, items[0]));
        const update = vi.fn((patch: any) => Object.assign(config, patch));
        const updateFilter = vi.fn((filter: any) => { config.filter = filter; });
        (window as any).DATAVISOOL_ENV = {
            currentManager: {layerManager: {
                layerConfigs: {A: layer}, updateLayer,
                controllers: {A: {getConfig: () => config, update, updateFilter}},
            }},
        };

        const result = await runApplyComponentPatchTool({
            approvalTicket,
            patches: [{
                componentId: "A", componentType: "BaseText",
                layerPatch: {x: 30, width: 160, name: "新名称"},
                filterPatch: {enable: true, opacity: 0.5},
            }],
            expectedSnapshots: [{
                componentId: "A", componentType: "BaseText",
                layer: structuredClone(layer), filter: structuredClone(config.filter),
            }],
        });

        expect(result.failed).toEqual([]);
        expect(layer).toMatchObject({x: 30, y: 20, width: 160, height: 40, name: "新名称"});
        expect(config.filter).toMatchObject({enable: true, opacity: 0.5, brightness: 1});
        expect(updateLayer).toHaveBeenCalledTimes(1);
        expect(updateFilter).toHaveBeenCalledTimes(1);
    });

    test("does not report a layer conflict when IPC only removes undefined fields", async () => {
        const layer: any = {
            id: "A",
            type: "BaseText",
            x: 10,
            y: 20,
            width: 100,
            height: 40,
            pid: undefined,
            childHeader: undefined,
            childTail: undefined,
            children: undefined,
            prev: undefined,
            next: undefined,
        };
        const config = {base: {type: "BaseText"}};
        const updateLayer = vi.fn((items: any[]) => Object.assign(layer, items[0]));
        (window as any).DATAVISOOL_ENV = {
            currentManager: {layerManager: {
                layerConfigs: {A: layer},
                updateLayer,
                controllers: {A: {getConfig: () => config, update: vi.fn()}},
            }},
        };

        const result = await runApplyComponentPatchTool({
            approvalTicket,
            patches: [{componentId: "A", componentType: "BaseText", layerPatch: {x: 30}}],
            expectedSnapshots: [{
                componentId: "A",
                componentType: "BaseText",
                layer: JSON.parse(JSON.stringify(layer)),
            }],
        });

        expect(result.failed).toEqual([]);
        expect(updateLayer).toHaveBeenCalledTimes(1);
        expect(layer.x).toBe(30);
    });

    test("still reports a layer conflict when a coordinate really changed", async () => {
        const layer = {id: "A", type: "BaseText", x: 11, y: 20, width: 100, height: 40};
        const updateLayer = vi.fn();
        (window as any).DATAVISOOL_ENV = {
            currentManager: {layerManager: {
                layerConfigs: {A: layer},
                updateLayer,
                controllers: {A: {getConfig: () => ({base: {type: "BaseText"}}), update: vi.fn()}},
            }},
        };

        const result = await runApplyComponentPatchTool({
            approvalTicket,
            patches: [{componentId: "A", componentType: "BaseText", layerPatch: {x: 30}}],
            expectedSnapshots: [{
                componentId: "A",
                componentType: "BaseText",
                layer: {id: "A", type: "BaseText", x: 10, y: 20, width: 100, height: 40},
            }],
        });

        expect(updateLayer).not.toHaveBeenCalled();
        expect(result.applied).toEqual([]);
        expect(result.failed[0]).toMatchObject({code: "component_revision_conflict"});
    });

    test("validates field mappings against the complete runtime data", async () => {
        const config = {
            base: {id: "chart-real", type: "AntdMultiLine"},
            style: {xField: "time", yField: "amount", seriesField: "channel"},
            data: {
                sourceType: "static",
                staticData: {value: []},
                value: [
                    {time: "Q1", amount: 1, channel: "A"},
                    {time: "Q2", amount: 2, channel: "A"},
                    {time: "Q3", amount: 3, channel: "A"},
                    {time: "Q4", amount: 4},
                ],
            },
        };
        const update = vi.fn();
        (window as any).DATAVISOOL_ENV = {
            currentManager: {
                layerManager: {
                    layerConfigs: {"chart-real": {id: "chart-real", type: "AntdMultiLine"}},
                    controllers: {"chart-real": {getConfig: () => config, update}},
                },
            },
        };

        const result = await runApplyComponentPatchTool({
            approvalTicket,
            patches: [{
                componentId: "chart-real",
                componentType: "AntdMultiLine",
                stylePatch: {style: {seriesField: "channel"}},
            }],
            expectedSnapshots: [{
                componentId: "chart-real",
                componentType: "AntdMultiLine",
                style: structuredClone(config.style),
            }],
        });

        expect(update).not.toHaveBeenCalled();
        expect(result.applied).toEqual([]);
        expect(result.failed[0]).toMatchObject({
            code: "component.data.field_mapping_missing_property",
        });
    });

    test("rejects a high-risk patch without an approval ticket", async () => {
        const update = vi.fn();
        (window as any).DATAVISOOL_ENV = {
            currentManager: {
                layerManager: {
                    layerConfigs: {A: {id: "A", type: "BaseText"}},
                    controllers: {A: {getConfig: () => ({base: {type: "BaseText"}}), update}},
                },
            },
        };

        await expect(runApplyComponentPatchTool({
            patches: [{componentId: "A", componentType: "BaseText", layerPatch: {x: 30}}],
            expectedSnapshots: [{componentId: "A", componentType: "BaseText", layer: {id: "A", type: "BaseText"}}],
        })).rejects.toMatchObject({code: "approval_ticket_missing"});
        expect(update).not.toHaveBeenCalled();
    });
});
