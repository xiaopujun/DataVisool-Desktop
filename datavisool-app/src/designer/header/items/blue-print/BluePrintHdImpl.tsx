/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import ReactDOM from "react-dom";
import {lazy, ReactElement, Suspense} from "react";
import {Loading} from "@datavisool/ui";

const BluePrint = lazy(() => import("@/designer/blueprint/BluePrint.tsx"));

export default function BluePrintHdImpl() {
    return ReactDOM.createPortal(
        <div className="blueprint-portal-host" style={{
            position: 'fixed',
            inset: 'var(--datavisool-app-titlebar-height, 45px) 0 0 0',
            zIndex: 1000,
            isolation: 'isolate',
            backgroundColor: '#0A1020',
            overflow: 'hidden'
        }}>
            <Suspense fallback={<Loading/>}>
                <BluePrint/>
            </Suspense>
        </div>, document.body)! as ReactElement;
}
