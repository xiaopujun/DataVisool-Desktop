/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {globalMessage} from "@datavisool/ui";
import type {HttpResponse} from "@/util/FetchUtil.ts";
export interface ResponseHandlerOptions {
    suppressErrorMessage?: boolean | ((response: HttpResponse) => boolean);
}

const shouldSuppressErrorMessage = (
    response: HttpResponse,
    suppressErrorMessage: ResponseHandlerOptions["suppressErrorMessage"],
): boolean => {
    if (typeof suppressErrorMessage === "function") {
        return suppressErrorMessage(response);
    }
    return suppressErrorMessage === true;
};

const responseHandler = (response: HttpResponse, options: ResponseHandlerOptions = {}): unknown => {
    const {code, msg} = response;
    if (code == 200) {
        return response.data;
    } else if (code === 499) {
        return null;
    } else {
        if (!shouldSuppressErrorMessage(response, options.suppressErrorMessage)) {
            globalMessage?.messageApi?.error(msg)
        }
        return null;
    }
}

export default responseHandler;
