/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {ReactNode} from "react";
import TitleBar from "@/layouts/title-bar/TitleBar";
import '@/layouts/AppLayout.less';

interface AppLayoutProps {
    children: ReactNode;
}

export default function AppLayout({children}: AppLayoutProps) {
    return (
        <div className="app-layout-container">
            <TitleBar/>
            <div className="app-content-container">
                {children}
            </div>
        </div>
    );
}
