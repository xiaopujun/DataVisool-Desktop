/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {describe, expect, test} from "vitest";
import {ObjectUtil} from "@datavisool/sdk/utils";

describe("ObjectUtil.merge", () => {
    test("recursively merges non-empty plain objects", () => {
        const original = {
            style: {
                color: "#FFFFFF",
                axis: {visible: true, label: "月份"},
            },
        };

        const result = ObjectUtil.merge(original, {
            style: {axis: {label: "日期"}},
        });

        expect(result).toBe(original);
        expect(result).toEqual({
            style: {
                color: "#FFFFFF",
                axis: {visible: true, label: "日期"},
            },
        });
    });

    test("uses an empty plain object to clear the previous value", () => {
        const original = {style: {axis: {visible: true}}};

        const result = ObjectUtil.merge(original, {style: {axis: {}}});

        expect(result.style.axis).toEqual({});
    });

    test("replaces an array with a plain object", () => {
        const original = {data: [{year: "1月", value: 1}]};
        const nextData = {
            sourceType: "static",
            staticData: {value: [{year: "1月", value: 2}]},
            value: [{year: "1月", value: 2}],
        };

        const result = ObjectUtil.merge(original, {data: nextData});

        expect(Array.isArray(result.data)).toBe(false);
        expect(result.data).toEqual(nextData);
        expect(JSON.parse(JSON.stringify(result.data))).toEqual(nextData);
    });

    test("replaces arrays and other mismatched value types", () => {
        const original = {
            arrayValue: [1, 2],
            objectValue: {enabled: true},
            scalarValue: "old",
        };

        const result = ObjectUtil.merge(original, {
            arrayValue: [3],
            objectValue: ["new"],
            scalarValue: {enabled: false},
        });

        expect(result).toEqual({
            arrayValue: [3],
            objectValue: ["new"],
            scalarValue: {enabled: false},
        });
    });

    test("replaces non-plain object instances", () => {
        class ConfigValue {
            constructor(public readonly value: number) {}
        }
        const original = {custom: new ConfigValue(1)};
        const next = new ConfigValue(2);

        const result = ObjectUtil.merge(original, {custom: next});

        expect(result.custom).toBe(next);
    });
});
