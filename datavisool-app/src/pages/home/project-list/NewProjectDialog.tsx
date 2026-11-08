/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import React, {MouseEvent, useEffect, useRef, useState} from 'react';
import "@/pages/home/project-list/NewProjectDialog.less";
import {ArrowRight, Edit} from '@icon-park/react';
import {tx} from "@/i18n/miniI18n.ts";
import {useLocaleVersion} from "@/i18n/useLocaleVersion.ts";
import newProjectVisual from './assets/new-project-visual.webp';
import {Button, Dialog, Input, NumberInput, TextArea} from "@datavisool/ui";

export interface INewProjectInfo {
    name: string;
    des?: string;
    width: number;
    height: number;
    pid?: string;
}

interface AddNewScreenDialogProps {
    onOk?: (data: INewProjectInfo) => void;
    onCancel?: () => void;
    visible?: boolean;
}

const createDefaultData = (): INewProjectInfo => ({
    name: tx('未命名项目'),
    des: '',
    width: 1920,
    height: 1080,
});

export const NewProjectDialog: React.FC<AddNewScreenDialogProps> = (props) => {
    const localeVersion = useLocaleVersion();
    const defaultNameRef = useRef(tx('未命名项目'));
    const [data, setData] = useState(createDefaultData);
    const [nameError, setNameError] = useState(false);

    useEffect(() => {
        const nextDefaultName = tx('未命名项目');
        setData((current) => (
            current.name === defaultNameRef.current
                ? {...current, name: nextDefaultName}
                : current
        ));
        defaultNameRef.current = nextDefaultName;
    }, [localeVersion]);

    const onOk = (e: MouseEvent<HTMLButtonElement>) => {
        e.preventDefault();
        const name = data.name.trim();
        if (!name) {
            setNameError(true);
            return;
        }
        const {onOk} = props;
        onOk && onOk({...data, name});
        setNameError(false);
        // Reset after a short delay to allow animation or parent close
        setTimeout(() => setData(createDefaultData()), 100);
    };

    const onCancel = () => {
        const {onCancel} = props;
        setNameError(false);
        onCancel && onCancel();
        setTimeout(() => setData(createDefaultData()), 100);
    };

    const {visible = false} = props;
    const projectNameLength = data.name.length;
    const projectDescLength = data.des?.length ?? 0;

    return (
        <Dialog
            open={visible}
            onClose={onCancel}
            mask
            className="new-project-dialog"
            bodyClassName="new-project-dialog-body"
            width="60vw"
            centered
            hideHeader
        >
            <div className="dialog-container">
                <div className="visual-panel">
                    <img className="visual-art" src={newProjectVisual} alt=""/>
                    <div className="visual-content">
                        <h2 className="visual-title">{tx('开始创作')} <span>✦</span></h2>
                        <p className="visual-desc">
                            {tx('配置您的画布尺寸与项目信息，开启新的可视化设计之旅。')}
                        </p>
                    </div>
                </div>

                <div className="form-panel">
                    <div className="panel-header">
                        <h3>{tx('新建项目')}</h3>
                        <p>{tx('基础信息')}</p>
                    </div>

                    <div className="form-content">
                        <div className="form-group">
                            <label>{tx('项目名称')}</label>
                            <Input
                                prefix={<Edit theme="outline" fill="#9a9a9a"/>}
                                suffix={<span className="field-count">{projectNameLength}/50</span>}
                                value={data.name}
                                onChange={(value) => {
                                    setData({...data, name: value});
                                    if (value.trim()) setNameError(false);
                                }}
                                controlClassName="datavisool-app-input"
                                placeholder={tx('请输入项目名称')}
                                status={nameError ? 'error' : 'default'}
                                required
                                maxLength={50}
                            />
                            {nameError && <span className="field-error" role="alert">{tx('项目名称不能为空')}</span>}
                        </div>

                        <div className="form-group">
                            <label>{tx('项目描述信息')} <span>{tx('（可选）')}</span></label>
                            <div className="textarea-wrap">
                                <TextArea
                                    value={data.des}
                                    onChange={(value) => setData({...data, des: value})}
                                    controlClassName="datavisool-app-input datavisool-app-textarea"
                                    placeholder={tx('项目简要描述...')}
                                    maxLength={200}
                                />
                                <span className="textarea-count">{projectDescLength}/200</span>
                            </div>
                        </div>

                        <div className="form-row">
                            <div className="form-group half">
                                <label>{tx('宽度')}（px）</label>
                                <div className="dimension-field">
                                    <NumberInput
                                        min={300}
                                        value={data.width}
                                        onChange={(value) => setData({...data, width: Number(value)})}
                                        controlClassName="datavisool-app-input-number"
                                    />
                                </div>
                            </div>
                            <div className="form-group half">
                                <label>{tx('高度')}（px）</label>
                                <div className="dimension-field">
                                    <NumberInput
                                        min={300}
                                        value={data.height}
                                        onChange={(value) => setData({...data, height: Number(value)})}
                                        controlClassName="datavisool-app-input-number"
                                    />
                                </div>
                            </div>
                        </div>
                    </div>

                    <div className="panel-actions">
                        <Button onClick={onCancel} className="btn-cancel">
                            {tx('取消')}
                        </Button>
                        <Button type="primary" onClick={onOk} className="btn-create">
                            {tx('创建项目')}
                            <ArrowRight theme="outline" size="24" fill="#ffffff"/>
                        </Button>
                    </div>
                </div>
            </div>
        </Dialog>
    );
};

export default NewProjectDialog;
