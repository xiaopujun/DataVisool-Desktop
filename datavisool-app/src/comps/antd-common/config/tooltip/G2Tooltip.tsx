/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {FieldChangeData, DatavisoolGUI, type Control} from "@datavisool/ui";
// import {useState} from "react";
import AbstractDesignerController from "@/framework/core/AbstractDesignerController.ts";

/**
 * G2图表悬浮标签配置
 * @param props
 * @constructor
 */
export const G2Tooltip = (props: { controller: AbstractDesignerController }) => {
    const {controller} = props;
    const {legend} = controller.getConfig().style;
    // const [count, setCount] = useState(0);

    const onFieldChange = (_fieldChangeData: FieldChangeData) => {
        void _fieldChangeData;
        // const {id, data, dataFragment, reRender} = fieldChangeData;
        //
        // if (reRender)
        //     setCount(count + 1);
    }

    const schema: Control = {
        key: 'style',
        type: 'accordion',
        label: '悬浮标签',
        children: [
            {
                key: 'legend',
                type: 'grid',
                children: [
                    {
                        id: 'legendSwitch',
                        type: 'switch',
                        label: '显示',
                        reRender: true,
                        value: !!legend
                    },
                    {
                        rules: `"${!!legend}" === "true"`,
                        children: []
                    }
                ]
            }
        ]
    }

    return (
        <DatavisoolGUI schema={schema} onFieldChange={onFieldChange}/>
    );
};
