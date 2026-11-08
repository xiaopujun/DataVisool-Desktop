/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, describe, expect, test, vi} from "vitest";

const modalMocks = vi.hoisted(() => ({
    confirm: vi.fn(() => "preview-interaction-modal"),
    destroy: vi.fn(),
}));

vi.mock("@datavisool/ui", () => ({
    globalModal: {modalApi: modalMocks},
}));

vi.mock("@/designer/ai-driven/AiLayerDraftRuntime.ts", () => ({
    aiLayerDraftRuntime: {
        activeDraft: {
            draftId: "draft-1",
            layerCount: 3,
        },
    },
}));

import {
    normalizeComponentSelectionPayload,
    runRequestComponentSelectionTool,
    runShowPreviewTool,
} from "@/frontend-effect/tools/PreviewInteractionTools.ts";
import aiDrivenProgressTerminalStore from "@/designer/ai-driven/AiDrivenProgressTerminalStore.ts";

describe("preview interaction tools", () => {
    afterEach(() => {
        aiDrivenProgressTerminalStore.setInlineHostAvailable(false);
        aiDrivenProgressTerminalStore.reset();
    });

    test("preview cancellation resolves confirmed=false without accepting the draft", async () => {
        const result = runShowPreviewTool({draftId: "draft-1", message: "请确认"});
        const options = modalMocks.confirm.mock.calls.at(-1)?.[0] as {onCancel: () => void};

        options.onCancel();

        await expect(result).resolves.toEqual({confirmed: false, draftId: "draft-1"});
    });

    test("component selection is bounded to candidates and supplies the minimum defaults", async () => {
        const result = runRequestComponentSelectionTool({
            message: "请选择组件",
            candidates: [
                {componentType: "BaseText", displayName: "文本"},
                {componentType: "BaseImage", displayName: "图片"},
            ],
            minSelected: 1,
            maxSelected: 1,
        });
        const options = modalMocks.confirm.mock.calls.at(-1)?.[0] as {onOk: () => Promise<boolean>};

        await options.onOk();

        await expect(result).resolves.toEqual({
            confirmed: true,
            selectedComponentTypes: ["BaseText"],
        });
    });

    test("duplicate component types are rejected before opening the modal", () => {
        expect(() => normalizeComponentSelectionPayload({
            message: "请选择组件",
            candidates: [
                {componentType: "BaseText", displayName: "文本"},
                {componentType: "BaseText", displayName: "另一个文本"},
            ],
        })).toThrowError(/候选组件类型重复/);
    });

    test("abort closes a pending preview confirmation", async () => {
        const controller = new AbortController();
        const result = runShowPreviewTool({draftId: "draft-1"}, controller.signal);

        controller.abort("task_cancelled");

        await expect(result).rejects.toMatchObject({code: "effect_cancelled"});
        expect(modalMocks.destroy).toHaveBeenCalledWith("preview-interaction-modal");
    });

    test("uses the task timeline when its inline interaction host is available", async () => {
        aiDrivenProgressTerminalStore.startRun("request-a");
        aiDrivenProgressTerminalStore.setInlineHostAvailable(true);
        const result = runShowPreviewTool({draftId: "draft-1", message: "检查预览"});

        expect(aiDrivenProgressTerminalStore.pendingInlineInteraction).toEqual(expect.objectContaining({
            kind: "preview",
            draftId: "draft-1",
            layerCount: 3,
        }));
        aiDrivenProgressTerminalStore.completeInlineInteraction({
            action: "submit",
            data: {confirmed: true, draftId: "draft-1"},
        });

        await expect(result).resolves.toEqual({confirmed: true, draftId: "draft-1"});
    });
});
