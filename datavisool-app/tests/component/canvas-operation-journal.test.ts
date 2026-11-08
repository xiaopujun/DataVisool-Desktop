/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {describe, expect, test, vi} from "vitest";
import {
    runGetCanvasOperationStatusTool,
    runIdempotentCanvasOperation,
} from "@/frontend-effect/CanvasOperationJournal.ts";

describe("canvas operation journal", () => {
    test("reuses an applied receipt without executing twice", async () => {
        const execute = vi.fn(() => ({applied: ["c1"]}));
        const payload = {operationId: "run-applied:call-1", operationFingerprint: "sha256:same"};

        const first = await runIdempotentCanvasOperation(payload, execute);
        const second = await runIdempotentCanvasOperation(payload, execute);

        expect(second).toEqual(first);
        expect(execute).toHaveBeenCalledTimes(1);
        expect(JSON.parse(localStorage.getItem(`datavisool:canvas-operation:${encodeURIComponent(payload.operationId)}`)!)).toMatchObject({
            operationId: payload.operationId, status: "applied",
        });
        expect(runGetCanvasOperationStatusTool({operationId: payload.operationId})).toMatchObject({
            status: "applied",
            result: first,
        });
    });

    test("rejects reusing an operation id with different arguments", async () => {
        const operationId = "run-conflict:call-1";
        await runIdempotentCanvasOperation(
            {operationId, operationFingerprint: "sha256:first"},
            () => ({ok: true}),
        );

        await expect(runIdempotentCanvasOperation(
            {operationId, operationFingerprint: "sha256:second"},
            () => ({ok: true}),
        )).rejects.toMatchObject({code: "operation_fingerprint_conflict"});
    });

    test("does not retry an action that threw after entering the write boundary", async () => {
        const execute = vi.fn(() => {
            throw new Error("injected failure");
        });
        const payload = {operationId: "run-failed:call-1", operationFingerprint: "sha256:failed"};

        await expect(runIdempotentCanvasOperation(payload, execute)).rejects.toThrow("injected failure");
        await expect(runIdempotentCanvasOperation(payload, execute)).rejects.toMatchObject({code: "frontend_effect_failed"});
        expect(execute).toHaveBeenCalledTimes(1);
    });

    test("fails closed when a persisted receipt is corrupted", async () => {
        const operationId = "run-corrupted:call-1";
        const execute = vi.fn(() => ({applied: ["c1"]}));
        localStorage.setItem(`datavisool:canvas-operation:${encodeURIComponent(operationId)}`, "{broken");

        await expect(runIdempotentCanvasOperation(
            {operationId, operationFingerprint: "sha256:corrupted"},
            execute,
        )).rejects.toMatchObject({code: "operation_receipt_corrupted"});
        expect(execute).not.toHaveBeenCalled();
    });

    test("does not read an old-prefix operation receipt", async () => {
        const operationId = "run-old-prefix:call-1";
        localStorage.setItem(`lcp:canvas-operation:${encodeURIComponent(operationId)}`, JSON.stringify({
            operationId, operationFingerprint: "sha256:old", status: "applied", result: {old: true},
        }));
        const execute = vi.fn(() => ({current: true}));
        await expect(runIdempotentCanvasOperation({operationId, operationFingerprint: "sha256:new"}, execute)).resolves.toEqual({current: true});
        expect(execute).toHaveBeenCalledTimes(1);
    });
});
