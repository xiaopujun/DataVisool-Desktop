/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {parse} from "acorn";

export interface ApiConfigValidationInput {
    url?: unknown;
    headers?: unknown;
    params?: unknown;
}

export interface ApiConfigValidationErrors {
    url?: string;
    headers?: string;
    params?: string;
}

type DynamicValueField = "headers" | "params";

const validationMessages = {
    url: "请输入有效的请求 URL",
    headers: "请求头格式无效",
    params: "请求参数格式无效",
} satisfies Record<keyof ApiConfigValidationErrors, string>;

function isValidUrl(value: unknown): value is string {
    if (typeof value !== "string") {
        return false;
    }

    if (/[\u0000-\u001F\u007F]/.test(value) || value.includes("\\")) {
        return false;
    }

    const url = value.trim();
    if (!url) {
        return false;
    }

    if (url.startsWith("/") && !url.startsWith("//")) {
        try {
            new URL(url, "https://datavisool.invalid");
            return true;
        } catch {
            return false;
        }
    }

    if (!/^https?:\/\//i.test(url)) {
        return false;
    }

    try {
        const parsedUrl = new URL(url);
        return (parsedUrl.protocol === "http:" || parsedUrl.protocol === "https:")
            && Boolean(parsedUrl.hostname);
    } catch {
        return false;
    }
}

function isJsonValue(value: string, field: DynamicValueField): boolean {
    let parsedValue: unknown;
    try {
        parsedValue = JSON.parse(value);
    } catch {
        return false;
    }

    if (parsedValue === null || typeof parsedValue !== "object") {
        return false;
    }

    return field === "params" || !Array.isArray(parsedValue);
}

function isSupportedExpression(value: string): boolean {
    try {
        const program = parse(`(${value})`, {
            ecmaVersion: "latest",
            sourceType: "script",
        });
        const statement = program.body[0];
        if (program.body.length !== 1 || statement?.type !== "ExpressionStatement") {
            return false;
        }

        const expression = statement.expression;
        return (expression.type === "ObjectExpression" && value.startsWith("{"))
            || expression.type === "ArrowFunctionExpression"
            || (expression.type === "FunctionExpression" && !expression.generator);
    } catch {
        return false;
    }
}

function isValidDynamicValue(value: unknown, field: DynamicValueField): boolean {
    if (value == null || value === "") {
        return true;
    }

    if (typeof value === "object") {
        return field === "params" || !Array.isArray(value);
    }

    if (typeof value !== "string") {
        return false;
    }

    const source = value.trim();
    if (!source) {
        return true;
    }

    if (isJsonValue(source, field)) {
        return true;
    }

    return isSupportedExpression(source);
}

export function validateApiConfig(config: ApiConfigValidationInput): ApiConfigValidationErrors {
    const errors: ApiConfigValidationErrors = {};

    if (!isValidUrl(config.url)) {
        errors.url = validationMessages.url;
    }
    if (!isValidDynamicValue(config.headers, "headers")) {
        errors.headers = validationMessages.headers;
    }
    if (!isValidDynamicValue(config.params, "params")) {
        errors.params = validationMessages.params;
    }

    return errors;
}
