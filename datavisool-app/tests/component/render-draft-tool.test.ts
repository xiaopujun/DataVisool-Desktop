/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, describe, expect, test, vi} from "vitest";
import {DesignerMode, DesignerScene} from "@datavisool/sdk/types";

const runtimeMocks = vi.hoisted(() => {
    const managerStore = {currentManager: null as any};

    class MockDesignerManager {
        public scene: DesignerScene;
        public layerManager: any = {};

        constructor(scene: DesignerScene) {
            this.scene = scene;
        }

        public init(data: any): void {
            this.layerManager = {
                ...(data.layerManager ?? {}),
                controllers: {},
                getData: () => {
                    const {layerConfigs = {}, elemConfigs = {}, controllers = {}} = this.layerManager;
                    return {
                        layerHeader: this.layerManager.layerHeader,
                        layerTail: this.layerManager.layerTail,
                        layerConfigs,
                        elemConfigs: Object.keys(layerConfigs).reduce((configs, id) => {
                            const config = controllers[id]?.getConfig?.() ?? elemConfigs[id];
                            if (config != null)
                                configs[id] = config;
                            return configs;
                        }, {} as Record<string, any>),
                    };
                },
            };
        }

        public destroy(): void {
            this.layerManager = {};
        }
    }

    return {managerStore, MockDesignerManager};
});

const historyMocks = vi.hoisted(() => ({
    doUseTemplate: vi.fn((input: any) => {
        const sourceIds = Object.keys(input.layers);
        return {
            appliedLayerIds: sourceIds.map((_, index) => `real-layer-${index + 1}`),
            layerIdMap: Object.fromEntries(sourceIds.map((id, index) => [id, `real-layer-${index + 1}`])),
        };
    }),
}));

vi.mock("@/designer/manager/ManagerStore.ts", () => ({
    default: runtimeMocks.managerStore,
}));

vi.mock("@/designer/manager/DesignerManager.ts", () => ({
    default: runtimeMocks.MockDesignerManager,
}));

vi.mock("@/designer/operate-provider/undo-redo/HistoryRecordOperateProxy.ts", () => ({
    default: historyMocks,
}));

import managerStore from "@/designer/manager/ManagerStore.ts";
import {aiLayerDraftRuntime} from "@/designer/ai-driven/AiLayerDraftRuntime.ts";
import {runRenderDraftTool} from "@/frontend-effect/tools/RenderDraftTool.ts";

const currentManager = {
    scene: DesignerScene.DEFAULT,
    getData: () => ({}),
    canvasManager: {canvasConfig: {width: 1920, height: 1080}},
};

const approvalTicket = {approvalId: "test-approval", nonce: "test-nonce"};

describe("renderDraft tool", () => {
    afterEach(() => {
        currentManager.canvasManager.canvasConfig = {width: 1920, height: 1080};
        aiLayerDraftRuntime.clear();
        managerStore.currentManager = null;
        delete (window as any).DATAVISOOL_ENV;
        historyMocks.doUseTemplate.mockClear();
        vi.restoreAllMocks();
    });

    test("rejects acceptance when the canvas size changed after preview", () => {
        managerStore.currentManager = currentManager as any;
        (window as any).DATAVISOOL_ENV = {
            currentManager,
            mode: DesignerMode.EDIT,
            definitions: {
                BaseColorBlock: {
                    getInitConfig: () => ({base: {}, style: {}}),
                },
            },
        };
        const rendered = runRenderDraftTool({
            components: [{id: "A", type: "BaseColorBlock", x: 0, y: 0, width: 100, height: 100}],
        });
        if (rendered.status !== "draft-rendered")
            throw new Error("expected rendered draft");

        currentManager.canvasManager.canvasConfig = {width: 1280, height: 720};

        expect(() => runRenderDraftTool({action: "accept", draftId: rendered.draftId, approvalTicket}))
            .toThrow("页面草稿未能完整写入画布");
        expect(historyMocks.doUseTemplate).not.toHaveBeenCalled();
    });

    test("creates a temporary manager with component defaults and config patches", () => {
        managerStore.currentManager = currentManager as any;
        (window as any).DATAVISOOL_ENV = {
            currentManager,
            mode: DesignerMode.EDIT,
            definitions: {
                BaseColorBlock: {
                    getInitConfig: () => ({
                        base: {
                            name: "基础色块",
                            type: "BaseColorBlock",
                        },
                        style: {
                            background: "#009DFF33",
                            borderWidth: 0,
                        },
                        data: {
                            sourceType: "static",
                            value: "默认内容",
                        },
                    }),
                },
            },
        };

        const result = runRenderDraftTool({
            components: [
                {
                    id: "header",
                    type: "BaseColorBlock",
                    name: "顶部区域",
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 80,
                    config: {
                        style: {
                            background: "#FF000033",
                        },
                        data: {
                            sourceType: "static",
                            staticData: {value: "顶部内容"},
                        },
                    },
                },
                {
                    id: "content",
                    type: "BaseColorBlock",
                    x: 0,
                    y: 80,
                    width: 1920,
                    height: 1000,
                },
            ],
        });

        expect(result.status).toBe("draft-rendered");
        expect(result.layerCount).toBe(2);

        const layerManager = aiLayerDraftRuntime.activeDraft?.batches[0].manager.layerManager;
        const layers = Object.values(layerManager?.layerConfigs ?? {});
        const headerLayer = layers.find((layer) => layer.name === "顶部区域");
        const headerConfig = layerManager?.elemConfigs?.[headerLayer?.id ?? ""];

        expect(layers).toHaveLength(2);
        expect(headerLayer).toMatchObject({
            type: "BaseColorBlock",
            x: 0,
            y: 0,
            width: 1920,
            height: 80,
        });
        expect(headerConfig).toMatchObject({
            base: {
                id: headerLayer?.id,
                name: "顶部区域",
                type: "BaseColorBlock",
            },
            style: {
                background: "#FF000033",
                borderWidth: 0,
            },
            data: {
                sourceType: "static",
                value: "顶部内容",
                staticData: {value: "顶部内容"},
            },
        });
    });

    test("replaces and destroys the previous temporary manager", () => {
        managerStore.currentManager = currentManager as any;
        (window as any).DATAVISOOL_ENV = {
            currentManager,
            mode: DesignerMode.EDIT,
            definitions: {
                BaseColorBlock: {
                    getInitConfig: () => ({base: {}, style: {}}),
                },
            },
        };
        const component = {
            type: "BaseColorBlock",
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };

        runRenderDraftTool({components: [component]});
        const previousManager = aiLayerDraftRuntime.activeDraft!.batches[0].manager;
        const previousDestroy = vi.spyOn(previousManager, "destroy");

        runRenderDraftTool({components: [component]});

        expect(previousDestroy).toHaveBeenCalledOnce();
        expect(aiLayerDraftRuntime.activeDraft?.batches[0].manager).not.toBe(previousManager);
    });

    test("incrementally adds and updates components by id", () => {
        managerStore.currentManager = currentManager as any;
        (window as any).DATAVISOOL_ENV = {
            currentManager,
            mode: DesignerMode.EDIT,
            definitions: {
                BaseColorBlock: {
                    getInitConfig: () => ({base: {}, style: {}}),
                },
            },
        };
        const initialResult = runRenderDraftTool({
            components: [{
                id: "header",
                type: "BaseColorBlock",
                name: "顶部区域",
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            }],
        });
        if (initialResult.status !== "draft-rendered")
            throw new Error("expected rendered draft");
        const draftId = initialResult.draftId;
        const initialManager = aiLayerDraftRuntime.activeDraft!.batches[0].manager;
        const initialDestroy = vi.spyOn(initialManager, "destroy");

        const appendResult = runRenderDraftTool({
            incremental: true,
            components: [{
                id: "content",
                type: "BaseColorBlock",
                name: "内容区域",
                x: 0,
                y: 100,
                width: 100,
                height: 200,
            }],
        });

        expect(appendResult).toMatchObject({
            status: "draft-rendered",
            draftId,
            layerCount: 2,
        });
        expect(initialDestroy).toHaveBeenCalledOnce();
        expect(aiLayerDraftRuntime.activeDraft?.revision).toBe(2);
        expect(aiLayerDraftRuntime.activeDraft?.batches.map((batch) => batch.sequence)).toEqual([1, 2]);

        const appendedManager = aiLayerDraftRuntime.activeDraft!.batches[1].manager;
        const appendedDestroy = vi.spyOn(appendedManager, "destroy");
        const updateResult = runRenderDraftTool({
            incremental: true,
            components: [{
                id: "content",
                type: "BaseColorBlock",
                name: "内容区域",
                x: 200,
                y: 100,
                width: 100,
                height: 200,
            }],
        });
        const latestBatch = aiLayerDraftRuntime.activeDraft?.batches[1];
        const layers = Object.values(latestBatch?.manager.layerManager.layerConfigs ?? {});
        const contentLayer = layers.find((layer) => layer.name === "内容区域");

        expect(updateResult).toMatchObject({
            status: "draft-rendered",
            draftId,
            layerCount: 2,
        });
        expect(appendedDestroy).toHaveBeenCalledOnce();
        expect(aiLayerDraftRuntime.activeDraft?.revision).toBe(3);
        expect(aiLayerDraftRuntime.activeDraft?.batches.map((batch) => batch.sequence)).toEqual([1, 3]);
        expect(layers).toHaveLength(1);
        expect(contentLayer?.x).toBe(200);
    });

    test("accepts or discards one batch without affecting the others", () => {
        managerStore.currentManager = currentManager as any;
        (window as any).DATAVISOOL_ENV = {
            currentManager,
            mode: DesignerMode.EDIT,
            definitions: {
                BaseColorBlock: {
                    getInitConfig: () => ({base: {}, style: {}}),
                },
            },
        };
        const renderBatch = (id: string, x: number, incremental: boolean) => runRenderDraftTool({
            incremental,
            components: [{
                id,
                type: "BaseColorBlock",
                name: id,
                x,
                y: 0,
                width: 100,
                height: 100,
            }],
        });

        renderBatch("A", 0, false);
        renderBatch("B", 100, true);
        renderBatch("C", 200, true);
        const batches = aiLayerDraftRuntime.activeDraft!.batches;
        const acceptedManager = batches[1].manager;
        const acceptedDestroy = vi.spyOn(acceptedManager, "destroy");

        expect(batches.map((batch) => batch.sequence)).toEqual([1, 2, 3]);
        expect(aiLayerDraftRuntime.acceptBatch(batches[1].batchId)).toEqual(["real-layer-1"]);
        expect(historyMocks.doUseTemplate).toHaveBeenCalledOnce();
        expect(Object.values(historyMocks.doUseTemplate.mock.calls[0][0].layers)[0]).toMatchObject({
            name: "B",
            x: 100,
        });
        expect(acceptedDestroy).toHaveBeenCalledOnce();
        expect(aiLayerDraftRuntime.activeDraft?.batches.map((batch) => batch.sequence)).toEqual([1, 3]);
        expect(aiLayerDraftRuntime.activeDraft?.layerCount).toBe(2);

        expect(aiLayerDraftRuntime.discardBatch(batches[0].batchId)).toBe(true);
        expect(historyMocks.doUseTemplate).toHaveBeenCalledOnce();
        expect(aiLayerDraftRuntime.activeDraft?.batches.map((batch) => batch.sequence)).toEqual([3]);
        expect(aiLayerDraftRuntime.discardBatch(batches[2].batchId)).toBe(true);
        expect(aiLayerDraftRuntime.activeDraft).toBeNull();
    });

    test("accepts the exact confirmed draft without rendering it again", () => {
        managerStore.currentManager = currentManager as any;
        (window as any).DATAVISOOL_ENV = {
            currentManager,
            mode: DesignerMode.EDIT,
            definitions: {
                BaseColorBlock: {
                    getInitConfig: () => ({base: {}, style: {}}),
                },
            },
        };
        const rendered = runRenderDraftTool({
            interactive: false,
            components: [{
                id: "content",
                type: "BaseColorBlock",
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            }],
        });
        if (rendered.status !== "draft-rendered")
            throw new Error("expected rendered draft");

        expect(aiLayerDraftRuntime.activeDraft?.interactive).toBe(false);
        expect(runRenderDraftTool({action: "accept", draftId: rendered.draftId, approvalTicket})).toMatchObject({
            status: "draft-accepted",
            draftId: rendered.draftId,
            layerCount: 1,
            componentBindings: {content: "real-layer-1"},
        });
        expect(historyMocks.doUseTemplate).toHaveBeenCalledOnce();
        expect(aiLayerDraftRuntime.activeDraft).toBeNull();
        expect(() => runRenderDraftTool({action: "accept", draftId: rendered.draftId, approvalTicket}))
            .toThrow("草稿不存在或已发生变化");
    });

    test("rejects a tampered applied id that does not match the host reservation", () => {
        managerStore.currentManager = currentManager as any;
        (window as any).DATAVISOOL_ENV = {
            currentManager,
            mode: DesignerMode.EDIT,
            definitions: {BaseColorBlock: {getInitConfig: () => ({base: {}, style: {}})}},
        };
        const rendered = runRenderDraftTool({
            components: [{id: "content", type: "BaseColorBlock", x: 0, y: 0, width: 100, height: 100}],
        });
        if (rendered.status !== "draft-rendered")
            throw new Error("expected rendered draft");

        expect(() => runRenderDraftTool({
            action: "accept",
            draftId: rendered.draftId,
            approvalTicket,
            reservedComponentIds: {content: "host-reserved-id"},
        })).toThrow("页面草稿未能完整写入画布");
        expect(aiLayerDraftRuntime.activeDraft?.draftId).toBe(rendered.draftId);
    });

    test("rejects a reserved id that already exists on the formal canvas", () => {
        const layerManager = {
            layerConfigs: {existing: {id: "existing", type: "BaseColorBlock"}},
            getData: vi.fn(() => ({layerConfigs: {existing: {id: "existing"}}, elemConfigs: {}})),
        };
        const targetManager = {...currentManager, layerManager};
        managerStore.currentManager = targetManager as any;
        (window as any).DATAVISOOL_ENV = {
            currentManager: targetManager,
            mode: DesignerMode.EDIT,
            definitions: {BaseColorBlock: {getInitConfig: () => ({base: {}, style: {}})}},
        };
        const rendered = runRenderDraftTool({
            components: [{id: "content", type: "BaseColorBlock", x: 0, y: 0, width: 100, height: 100}],
        });
        if (rendered.status !== "draft-rendered")
            throw new Error("expected rendered draft");

        expect(() => runRenderDraftTool({
            action: "accept",
            draftId: rendered.draftId,
            approvalTicket,
            reservedComponentIds: {content: "existing"},
        })).toThrow("页面草稿未能完整写入画布");
        expect(historyMocks.doUseTemplate).not.toHaveBeenCalled();
    });

    test("same operation id accepts a draft only once", async () => {
        managerStore.currentManager = currentManager as any;
        (window as any).DATAVISOOL_ENV = {
            currentManager,
            mode: DesignerMode.EDIT,
            definitions: {BaseColorBlock: {getInitConfig: () => ({base: {}, style: {}})}},
        };
        const rendered = runRenderDraftTool({
            components: [{id: "content", type: "BaseColorBlock", x: 0, y: 0, width: 100, height: 100}],
        });
        if (rendered instanceof Promise || rendered.status !== "draft-rendered")
            throw new Error("expected rendered draft");
        const payload = {
            action: "accept" as const,
            draftId: rendered.draftId,
            operationId: "run-create:call-1",
            operationFingerprint: "sha256:create",
            approvalTicket,
        };

        const first = await runRenderDraftTool(payload);
        const second = await runRenderDraftTool(payload);

        expect(second).toEqual(first);
        expect(historyMocks.doUseTemplate).toHaveBeenCalledOnce();
    });

    test("accepts every incremental batch in one history operation", () => {
        managerStore.currentManager = currentManager as any;
        (window as any).DATAVISOOL_ENV = {
            currentManager,
            mode: DesignerMode.EDIT,
            definitions: {
                BaseColorBlock: {
                    getInitConfig: () => ({base: {}, style: {}}),
                },
            },
        };
        historyMocks.doUseTemplate.mockImplementationOnce((input: any) => ({
            appliedLayerIds: Object.keys(input.layers).map((id) => `real-${id}`),
            layerIdMap: Object.fromEntries(Object.keys(input.layers).map((id) => [id, `real-${id}`])),
        }));
        const first = runRenderDraftTool({
            components: [{id: "A", type: "BaseColorBlock", x: 0, y: 0, width: 100, height: 100}],
        });
        if (first.status !== "draft-rendered")
            throw new Error("expected rendered draft");
        runRenderDraftTool({
            incremental: true,
            components: [{id: "B", type: "BaseColorBlock", x: 100, y: 0, width: 100, height: 100}],
        });

        expect(runRenderDraftTool({action: "accept", draftId: first.draftId, approvalTicket})).toMatchObject({
            status: "draft-accepted",
            layerCount: 2,
        });
        expect(historyMocks.doUseTemplate).toHaveBeenCalledOnce();
        expect(Object.values(historyMocks.doUseTemplate.mock.calls[0][0].layers)).toHaveLength(2);
        expect(aiLayerDraftRuntime.activeDraft).toBeNull();
    });

    test("restores the formal layer snapshot when draft acceptance fails", () => {
        const originalLayerData = {
            layerHeader: "existing",
            layerTail: "existing",
            layerConfigs: {existing: {id: "existing", type: "BaseColorBlock"}},
            elemConfigs: {existing: {base: {id: "existing", type: "BaseColorBlock"}}},
        };
        const layerManager = {
            getData: vi.fn(() => structuredClone(originalLayerData)),
            init: vi.fn(),
        };
        const transactionalManager = {...currentManager, layerManager};
        managerStore.currentManager = transactionalManager as any;
        (window as any).DATAVISOOL_ENV = {
            currentManager: transactionalManager,
            mode: DesignerMode.EDIT,
            definitions: {
                BaseColorBlock: {
                    getInitConfig: () => ({base: {}, style: {}}),
                },
            },
        };
        historyMocks.doUseTemplate.mockImplementationOnce(() => {
            throw new Error("injected publish failure");
        });
        const rendered = runRenderDraftTool({
            components: [{id: "A", type: "BaseColorBlock", x: 0, y: 0, width: 100, height: 100}],
        });
        if (rendered.status !== "draft-rendered")
            throw new Error("expected rendered draft");

        expect(() => runRenderDraftTool({action: "accept", draftId: rendered.draftId, approvalTicket}))
            .toThrow("injected publish failure");
        expect(layerManager.init).toHaveBeenCalledWith(originalLayerData);
        expect(aiLayerDraftRuntime.activeDraft?.draftId).toBe(rendered.draftId);
    });

    test("keeps controller config when accepting a rendered draft batch", () => {
        managerStore.currentManager = currentManager as any;
        (window as any).DATAVISOOL_ENV = {
            currentManager,
            mode: DesignerMode.EDIT,
            definitions: {
                BaseColorBlock: {
                    getInitConfig: () => ({
                        base: {},
                        style: {background: "#009DFF33"},
                        data: {sourceType: "static", value: "默认内容"},
                    }),
                },
            },
        };

        runRenderDraftTool({
            components: [{
                id: "metric",
                type: "BaseColorBlock",
                x: 0,
                y: 0,
                width: 100,
                height: 100,
                config: {
                    style: {background: "#FF000033"},
                    data: {sourceType: "static", staticData: {value: "补丁内容"}},
                },
            }],
        });
        const batch = aiLayerDraftRuntime.activeDraft!.batches[0];
        const layerManager = batch.manager.layerManager;
        const layerId = Object.keys(layerManager.layerConfigs)[0];
        const renderedConfig = layerManager.elemConfigs[layerId];

        layerManager.controllers[layerId] = {getConfig: () => renderedConfig};
        delete layerManager.elemConfigs[layerId];

        aiLayerDraftRuntime.acceptBatch(batch.batchId);

        expect(historyMocks.doUseTemplate).toHaveBeenCalledWith({
            layers: expect.any(Object),
            configs: {
                [layerId]: expect.objectContaining({
                    style: expect.objectContaining({background: "#FF000033"}),
                    data: {
                        sourceType: "static",
                        staticData: {value: "补丁内容"},
                        value: "补丁内容",
                    },
                }),
            },
        });
    });

    test("keeps the draft session when all visible batches are resolved before the next increment", () => {
        managerStore.currentManager = currentManager as any;
        (window as any).DATAVISOOL_ENV = {
            currentManager,
            mode: DesignerMode.EDIT,
            definitions: {
                BaseColorBlock: {
                    getInitConfig: () => ({base: {}, style: {}}),
                },
            },
        };
        const first = runRenderDraftTool({
            components: [{id: "A", type: "BaseColorBlock", x: 0, y: 0, width: 100, height: 100}],
        });
        if (first.status !== "draft-rendered")
            throw new Error("expected rendered draft");
        const firstBatch = aiLayerDraftRuntime.activeDraft!.batches[0];

        aiLayerDraftRuntime.discardBatch(firstBatch.batchId);
        expect(aiLayerDraftRuntime.activeDraft).toBeNull();

        const second = runRenderDraftTool({
            incremental: true,
            components: [{id: "B", type: "BaseColorBlock", x: 100, y: 0, width: 100, height: 100}],
        });

        expect(second).toMatchObject({
            status: "draft-rendered",
            draftId: first.draftId,
            layerCount: 1,
        });
        expect(aiLayerDraftRuntime.activeDraft?.batches[0].sequence).toBe(2);
    });

    test("requires boolean incremental mode and component ids", () => {
        managerStore.currentManager = currentManager as any;
        (window as any).DATAVISOOL_ENV = {
            currentManager,
            mode: DesignerMode.EDIT,
            definitions: {
                BaseColorBlock: {
                    getInitConfig: () => ({base: {}, style: {}}),
                },
            },
        };
        const component = {
            type: "BaseColorBlock",
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };

        expect(() => runRenderDraftTool({incremental: true, components: [component]}))
            .toThrow("增量渲染的每个草稿组件都必须提供 id");
        expect(() => runRenderDraftTool({incremental: "yes", components: [{...component, id: "content"}]}))
            .toThrow("incremental 必须是布尔值");
    });

    test("clears the active draft and rejects invalid component sizes", () => {
        managerStore.currentManager = currentManager as any;
        (window as any).DATAVISOOL_ENV = {
            currentManager,
            mode: DesignerMode.EDIT,
            definitions: {
                BaseColorBlock: {
                    getInitConfig: () => ({base: {}, style: {}}),
                },
            },
        };

        expect(() => runRenderDraftTool({
            components: [{
                type: "BaseColorBlock",
                x: 0,
                y: 0,
                width: 0,
                height: 100,
            }],
        })).toThrow("width 必须大于 0");

        runRenderDraftTool({
            components: [{
                type: "BaseColorBlock",
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            }],
        });
        const manager = aiLayerDraftRuntime.activeDraft!.batches[0].manager;
        const destroy = vi.spyOn(manager, "destroy");

        expect(runRenderDraftTool({action: "clear"})).toEqual({
            status: "draft-cleared",
            layerCount: 0,
        });
        expect(destroy).toHaveBeenCalledOnce();
        expect(aiLayerDraftRuntime.activeDraft).toBeNull();
    });
});
