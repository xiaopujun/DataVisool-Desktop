/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {beforeEach, describe, expect, test, vi} from "vitest";

const requests = vi.hoisted(() => ({get: vi.fn(), post: vi.fn()}));
vi.mock("@/util/FetchUtil.ts", () => ({default: requests}));

import AuthFetchUtil from "@/util/AuthFetchUtil.ts";

describe("requests without an app login", () => {
    beforeEach(() => vi.clearAllMocks());

    test("forwards local requests without adding a login token", async () => {
        await AuthFetchUtil.get("/api/project/list");
        expect(requests.get).toHaveBeenCalledWith("/api/project/list", undefined);
    });

    test("preserves explicitly supplied authorization", async () => {
        const options = {headers: {Authorization: "Bearer external-service-token"}};
        await AuthFetchUtil.post("/api/operation", {value: 1}, options);
        expect(requests.post).toHaveBeenCalledWith("/api/operation", {value: 1}, options);
    });
});
