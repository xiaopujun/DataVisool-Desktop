/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {Suspense} from "react";
import {render, screen, waitFor} from "@testing-library/react";
import {createMemoryRouter, Outlet, RouterProvider} from "react-router-dom";
import {describe, expect, test, vi} from "vitest";

vi.mock("@/pages/home/Home", () => ({default: () => <Outlet />}));
vi.mock("@/pages/home/console/ConsoleDashboard.tsx", () => ({
    default: () => <div>控制台已加载</div>,
}));

import routes from "@/router/router.tsx";

describe("desktop startup without login", () => {
    test.each(["/", "/home"])("opens the console from %s", async (path) => {
        const router = createMemoryRouter(routes, {initialEntries: [path]});
        render(<Suspense fallback={null}><RouterProvider router={router} /></Suspense>);
        expect(await screen.findByText("控制台已加载")).toBeTruthy();
        await waitFor(() => expect(router.state.location.pathname).toBe("/home/console"));
        router.dispose();
    });
});
