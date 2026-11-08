/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {useState} from 'react';
import ThemeItem from "@/comps/common-component/theme-config/theme-item/ThemeItem";
import {observer} from "mobx-react";
import {IThemeItem, ThemeColors} from "@datavisool/sdk/types";

interface ThemeListProps {
    data?: IThemeItem[];
    onSelected?: (data: IThemeItem) => void;
    showOperator?: boolean;
    onDel?: (id: string) => void;
}

const ThemeList = observer((props: ThemeListProps) => {
    const {onSelected, showOperator, onDel} = props;
    const [activeId, setActiveId] = useState('');

    const _onDel = (id: string) => {
        const {themeConfig, updateThemeConfig} = window.DATAVISOOL_ENV.currentManager.themeManager!;
        const newThemes = themeConfig!.filter((item: IThemeItem) => item.id !== id);
        updateThemeConfig(newThemes);
        onDel && onDel(id);
    }


    const _onSelected = (data: IThemeItem) => {
        onSelected && onSelected(data);
        if (data.id !== activeId)
            setActiveId(data.id);
        else
            setActiveId('');
    }

    const {themeConfig} = window.DATAVISOOL_ENV.currentManager.themeManager!;
    const themeList = [];
    for (let i = 0; i < themeConfig!.length; i++) {
        themeList.push(<ThemeItem key={i} id={themeConfig![i].id} selected={themeConfig![i].id === activeId}
                                  name={themeConfig![i].name}
                                  showOperator={showOperator}
                                  onDel={_onDel}
                                  onSelected={_onSelected}
                                  colors={themeConfig![i].colors as ThemeColors}/>)
    }
    return (
        <div className={'datavisool-app-theme-list'} style={{
            width: '100%',
            display: 'flex',
            flexDirection: 'column',
            gap: 20
        }}>
            {themeList}
        </div>
    );
});

export default ThemeList;
