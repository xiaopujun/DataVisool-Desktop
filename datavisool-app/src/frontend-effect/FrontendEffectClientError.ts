/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

/** 前端 effect 客户端错误。 */
export class FrontendEffectClientError extends Error {
    /** 稳定错误码。 */
    public readonly code: string;

    /** 创建带错误码的前端 effect 错误。 */
    constructor(code: string, message: string) {
        super(message);
        this.name = "FrontendEffectClientError";
        this.code = code;
    }
}
