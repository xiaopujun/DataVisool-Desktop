/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {ReactNode} from "react";
import "@/pages/home/page-container/PageContainer.less";

export interface PageContainerProps {
    title: string;
    tools: ReactNode[];
    content: ReactNode;
    className?: string;
}

export default function PageContainer(props: PageContainerProps) {
    const {title, tools, content, className} = props;
    return (
        <div className="home-page-container ">
            <div className={'page-container-header'}>
                <div className={'container-header-left'}>
                    <div className="container-header-title">{title}</div>
                </div>
                <div className={'container-header-right'}>
                    {tools.map((item, index) => (
                        <div key={index} className={'header-tool-item'}>{item}</div>
                    ))}
                </div>
            </div>
            <div className={`page-container-content ${className ?? ''}`}>
                {content}
            </div>
        </div>
    )
}
