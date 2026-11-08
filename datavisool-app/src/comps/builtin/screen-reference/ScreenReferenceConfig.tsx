/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import React from 'react';
import {ScreenReferenceController} from "@/comps/builtin/screen-reference/ScreenReferenceController.ts";
import {ConfigType} from "@/designer/right/ConfigContent";
import {URLUtil} from "@datavisool/sdk/utils";
import AuthFetchUtil from "@/util/AuthFetchUtil.ts";
import {FieldChangeData, globalMessage, DatavisoolGUI, type Control} from "@datavisool/ui";

export const ScreenReferenceConfig: React.FC<ConfigType<ScreenReferenceController>> = ({controller}) => {

    const config = controller.getConfig()?.style;
    const {id: projectId} = URLUtil.parseUrlParams();

    const onFieldChange = (fieldChangeData: FieldChangeData) => {
        const {dataFragment, data} = fieldChangeData;
        if (data === projectId) {
            globalMessage?.messageApi?.error("大屏ID不能引用自身");
            return;
        }
        controller.update(dataFragment);
    }

    const schema: Control = {
        type: "grid",
        key: "style",
        children: [
            {
                key: "screenId",
                type: "select",
                label: "大屏ID",
                value: config?.screenId,
                config: {
                    showSearch: true,
                    handleSearch: async (search: string) => {
                        const response = await AuthFetchUtil.get<any>(`/api/project/quickSearch?current=${config?.screenId ?? ""}&exclude=${projectId}&search=${search}`)
                        if (response !== null)
                            return response?.map((item: any) => ({label: item.name, value: item.id}));
                        else
                            return [];
                    }
                }
            }
        ]
    }


    return (
        <div style={{padding: 10}}><DatavisoolGUI schema={schema} onFieldChange={onFieldChange}/></div>
    )
}
