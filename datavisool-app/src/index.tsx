/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {createRoot} from "react-dom/client";
import '@/global/configureComponentRendering';
import {getLocale, startDomTranslator, tx} from "@/i18n/miniI18n.ts";
import {useLocaleVersion} from "@/i18n/useLocaleVersion.ts";
import './style/tailwind.css';
import '@datavisool/ui/style.css';
import './index.less';
import './theme/default.less';
import {Profiler, Suspense} from "react";
import type {ProfilerOnRenderCallback, ReactNode} from "react";
import {GlobalMessage, GlobalModal, DataVisoolUIProvider} from "@datavisool/ui";
import {Loading} from "@datavisool/ui";
import {createBrowserRouter, RouterProvider} from "react-router-dom";
import routerConfig from "./router/router.tsx";
import {setFavicon} from "./util/LogoUtil.ts";
import logo from './images/datavisool-logo.webp';
import dayjs from "dayjs";
import 'dayjs/locale/zh-cn';
import AppLayout from "./layouts/AppLayout.tsx";
import AppErrorBoundary from "@/components/app-error/AppErrorBoundary.tsx";
import ErrorPage from "@/pages/result/Error.tsx";
import withRouteErrorElement from "@/router/withRouteErrorElement.tsx";
import {registerFrontendEffectListener} from "@/frontend-effect/FrontendEffectBridge.tsx";

if (import.meta.env.VITE_DESKTOP_E2E === '1') {
    void import('@wdio/tauri-plugin');
}

const router = createBrowserRouter(withRouteErrorElement(routerConfig, <ErrorPage homePath="/home"/>));

setFavicon(logo, logo)

startDomTranslator();
void registerFrontendEffectListener().catch((error) => {
    console.warn("register frontend effect listener failed", error);
});

const isPerfBaselineEnabled = () => new URLSearchParams(window.location.search).get('perfBaseline') === '1';

const recordReactCommit: ProfilerOnRenderCallback = (
    id,
    phase,
    actualDuration,
    baseDuration,
    startTime,
    commitTime
) => {
    const perf = (window as any).__DATAVISOOL_PERF_BASELINE__;
    if (!perf)
        return;
    perf.reactCommits = perf.reactCommits || [];
    perf.reactCommits.push({
        id,
        phase,
        actualDuration,
        baseDuration,
        startTime,
        commitTime,
    });
};

function Root() {
    useLocaleVersion();
    const locale = getLocale();
    dayjs.locale(locale === 'zh-CN' ? 'zh-cn' : 'en');

    const content: ReactNode = (
        <Suspense fallback={<Loading/>}>
            <AppErrorBoundary homePath="/home">
                <AppLayout>
                    <RouterProvider router={router}/>
                    <GlobalMessage/>
                    <GlobalModal/>
                </AppLayout>
            </AppErrorBoundary>
        </Suspense>
    );

    const appContent = isPerfBaselineEnabled()
        ? <Profiler id="AppRoot" onRender={recordReactCommit}>{content}</Profiler>
        : content;

    return (
        <DataVisoolUIProvider locale={locale} translate={tx}>
            {appContent}
        </DataVisoolUIProvider>
    );
}

createRoot(document.getElementById('root')!).render(<Root/>);
