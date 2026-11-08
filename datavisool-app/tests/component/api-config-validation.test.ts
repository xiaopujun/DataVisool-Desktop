import {describe, expect, it} from "vitest";
import {validateApiConfig} from "@/pages/home/api-management/ApiConfigValidation.ts";

describe("API configuration validation", () => {
    it.each([
        "https://api.example.com/v1/items?active=1",
        "http://127.0.0.1:8787/echo",
        "/api/commonApi/quickSearch?search=test",
        "/internal/data",
    ])("accepts supported URL %s", (url) => {
        expect(validateApiConfig({url})).toEqual({});
    });

    it.each([
        "",
        "   ",
        "not a url",
        "ftp://api.example.com/data",
        "//api.example.com/data",
        "http:api.example.com/data",
        "http:/api.example.com/data",
        "/\\api.example.com/data",
        "https://api.example.com/\u0001data",
    ]) (
        "rejects invalid URL %s",
        (url) => {
            expect(validateApiConfig({url})).toEqual({url: "请输入有效的请求 URL"});
        },
    );

    it.each([
        ["{\"Authorization\":\"Bearer token\"}", "{\"city\":\"shanghai\"}"],
        ["{Authorization: 'Bearer token'}", "{city: 'shanghai'}"],
        ["async () => ({Authorization: 'Bearer token'})", "async () => ({city: 'shanghai'})"],
        ["function getHeaders() { return {Authorization: 'Bearer token'}; }", "() => ({city: 'shanghai'})"],
        ["", "[\"first\", \"second\"]"],
    ])("accepts existing JSON and dynamic expressions", (headers, params) => {
        expect(validateApiConfig({url: "https://api.example.com/data", headers, params})).toEqual({});
    });

    it("rejects unsupported root values and malformed expressions", () => {
        expect(validateApiConfig({
            url: "https://api.example.com/data",
            headers: "{broken",
            params: "notAllowed",
        })).toEqual({
            headers: "请求头格式无效",
            params: "请求参数格式无效",
        });

        expect(validateApiConfig({
            url: "https://api.example.com/data",
            headers: "42",
            params: "'foo'",
        })).toEqual({
            headers: "请求头格式无效",
            params: "请求参数格式无效",
        });
    });

    it("checks function syntax without running it", () => {
        const marker = "__apiConfigValidationSideEffect";
        delete (globalThis as any)[marker];

        const errors = validateApiConfig({
            url: "https://api.example.com/data",
            headers: `() => { globalThis.${marker} = true; return {}; }`,
            params: "async () => ({city: 'shanghai'})",
        });

        expect(errors).toEqual({});
        expect((globalThis as any)[marker]).toBeUndefined();
    });

    it("does not accept parenthesized object literals that the resolver treats as unsupported values", () => {
        expect(validateApiConfig({
            url: "https://api.example.com/data",
            params: "({city: 'shanghai'})",
        })).toEqual({params: "请求参数格式无效"});
    });
});
