/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import React, {Component} from 'react';
import './ThemeItem.less';
import {Close} from "@icon-park/react";
import {IThemeItem, ThemeColors} from "@datavisool/sdk/types";
import ColorPreviewItem from "./ColorPreviewItem.tsx";


interface ThemeItemProps extends ThemeColors {
    //用于标识。 该主题是否被选中被选中的主题边框颜色与其他主题不同。
    selected?: boolean;
    name?: string;
    colors: ThemeColors;
    id?: string;
    itemStyle?: React.CSSProperties;
    showOperator?: boolean;
    onDel?: (id: string) => void;
    onSelected?: (data: IThemeItem) => void;
}

class ThemeItem extends Component<ThemeItemProps> {

    onSelected = () => {
        const {onSelected, id = '', name = '', colors} = this.props;
        onSelected && onSelected({id, name, colors});
    }

    render() {
        const {colors, selected = false, itemStyle, name, id, showOperator = false, onDel} = this.props;
        return (
            <div id={id} className={`datavisool-app-theme-item ${selected ? 'datavisool-app-theme-item-active' : ''}`} style={itemStyle}
                 onClick={this.onSelected}>
                <div className={'datavisool-app-theme-item-header'}>
                    <div className={'datavisool-app-theme-item-title'}>{name}</div>
                    {showOperator && <div className={'datavisool-app-theme-item-operators'}>
                        <div className={'operator-item'} onClick={(event) => {
                            event.stopPropagation();
                            onDel && onDel(id || '')
                        }}>
                            <Close size={16}/>
                        </div>
                    </div>}
                </div>
                <div className={'datavisool-app-theme-item-body'}>
                    {
                        Object.keys(this.props.colors).map((key, index) => {
                            return <ColorPreviewItem key={index} color={(colors[key as keyof ThemeColors]) || ''}/>
                        })
                    }
                </div>
            </div>
        );
    }
}

export default ThemeItem;