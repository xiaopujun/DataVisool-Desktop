/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, describe, expect, test, vi} from "vitest";

const deleteMocks = vi.hoisted(() => ({deleteLayersByIds: vi.fn()}));
vi.mock("@/designer/operate-provider/selection/SelectionCommands.ts", () => deleteMocks);

import {runDeleteCanvasComponentsTool} from "@/frontend-effect/tools/DeleteCanvasComponentsTool.ts";

const approvalTicket = {approvalId: "test-approval", nonce: "test-nonce"};

describe("deleteCanvasComponents tool", () => {
    afterEach(() => {
        deleteMocks.deleteLayersByIds.mockReset();
        delete (window as any).DATAVISOOL_ENV;
    });

    test("deletes exactly the targets protected by layer snapshots", async () => {
        const layers: Record<string, any> = {
            A: {id: "A", type: "BaseText", x: 10, y: 20},
            B: {id: "B", type: "BaseText", x: 30, y: 40},
        };
        deleteMocks.deleteLayersByIds.mockImplementation((ids: string[]) => ids.forEach((id) => delete layers[id]));
        (window as any).DATAVISOOL_ENV = {currentManager: {layerManager: {layerConfigs: layers}}};

        const result = await runDeleteCanvasComponentsTool({
            approvalTicket,
            operationId: "delete-test-1",
            operationFingerprint: "sha256:delete-1",
            targets: [{componentId: "A", componentType: "BaseText"}],
            expectedSnapshots: [{componentId: "A", componentType: "BaseText", layer: structuredClone(layers.A)}],
        });

        expect(result.failed).toEqual([]);
        expect(result.deletedComponentIds).toEqual(["A"]);
        expect(layers.B).toBeDefined();
        expect(deleteMocks.deleteLayersByIds).toHaveBeenCalledWith(["A"]);
    });

    test("rejects the whole deletion when the layer changed after inspection", async () => {
        const layers: Record<string, any> = {A: {id: "A", type: "BaseText", x: 99}};
        (window as any).DATAVISOOL_ENV = {currentManager: {layerManager: {layerConfigs: layers}}};

        const result = await runDeleteCanvasComponentsTool({
            approvalTicket,
            operationId: "delete-test-2",
            operationFingerprint: "sha256:delete-2",
            targets: [{componentId: "A", componentType: "BaseText"}],
            expectedSnapshots: [{componentId: "A", componentType: "BaseText", layer: {id: "A", type: "BaseText", x: 10}}],
        });

        expect(result.deletedComponentIds).toEqual([]);
        expect(result.failed[0]?.code).toBe("component_revision_conflict");
        expect(deleteMocks.deleteLayersByIds).not.toHaveBeenCalled();
    });

    test("rejects deletion without an approval ticket", async () => {
        await expect(runDeleteCanvasComponentsTool({
            operationId: "delete-test-missing-ticket",
            operationFingerprint: "sha256:delete-missing-ticket",
            targets: [{componentId: "A", componentType: "BaseText"}],
            expectedSnapshots: [{componentId: "A", componentType: "BaseText", layer: {id: "A", type: "BaseText"}}],
        })).rejects.toMatchObject({code: "approval_ticket_missing"});
        expect(deleteMocks.deleteLayersByIds).not.toHaveBeenCalled();
    });
});
