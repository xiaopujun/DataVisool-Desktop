/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import React, {useState} from "react";
import BaseImageController from "./BaseImageController";
import {ConfigType} from "@/designer/right/ConfigContent";
import {FieldChangeData, IImageData, DatavisoolGUI, type Control} from "@datavisool/ui";
import {URLUtil} from "@datavisool/sdk/utils";
import AuthFetchUtil from "@/util/AuthFetchUtil.ts";

export const BaseImageStyleConfig: React.FC<ConfigType<BaseImageController>> = ({controller}) => {
    const {style} = controller.getConfig()!;
    const {type, onLineUrl, localUrl, opacity} = style!;

    const [, setCount] = useState(0);

    const onFieldChange = (fieldChangeData: FieldChangeData) => {
        const {id, data, dataFragment, reRender} = fieldChangeData;
        if (id === 'localUrl') {
            const {hash, url} = data as Record<string, any>;
            controller.update({style: {localUrl: url, hash}});
        } else {
            controller.update(dataFragment);
        }
        if (reRender)
            setCount(count => count + 1);
    }

    const schema: Control = {
        key: 'style',
        type: 'grid',
        children: [
            {
                key: 'type',
                type: 'radio',
                label: '来源',
                value: type,
                reRender: true,
                config: {
                    options: [
                        {label: '本地', value: 'local'},
                        {label: '在线', value: 'online'},
                    ],
                }
            },
            {
                key: 'onLineUrl',
                rules: "{type} === 'online'",
                type: 'input',
                label: '链接',
                value: onLineUrl,
            },
            {
                id: 'localUrl',
                key: 'localUrl',
                rules: "{type} === 'local'",
                type: 'image-upload',
                label: '上传',
                value: localUrl,
                config: {
                    accept: 'image/*',
                    uploadHandle: async (file: File): Promise<IImageData | boolean> => {
                        const isTauri = typeof window !== 'undefined' && (('__TAURI_INTERNALS__' in window) || ('__TAURI__' in window));
                        if (isTauri) {
                            const buf = await file.arrayBuffer();
                            const bytes = new Uint8Array(buf);
                            const chunkSize = 0x8000;
                            let binary = '';
                            for (let i = 0; i < bytes.length; i += chunkSize) {
                                binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize));
                            }
                            const base64 = btoa(binary);

                            const digest = await crypto.subtle.digest('SHA-256', buf);
                            const hash = Array.from(new Uint8Array(digest)).map(b => b.toString(16).padStart(2, '0')).join('');

                            const mime = file.type || 'application/octet-stream';
                            const dataUrl = `data:${mime};base64,${base64}`;
                            const res = await AuthFetchUtil.post<string>('/api/image/upload', {
                                fileName: file.name,
                                dataUrl,
                                hash,
                                name: file.name
                            });
                            return res !== null ? {url: res, hash} : false;
                        }
                        const {id} = URLUtil.parseUrlParams();
                        const formData = new FormData();
                        formData.append('file', file);
                        formData.append('projectId', id);
                        formData.append('type', "1");
                        const res = await AuthFetchUtil.post<string>('/api/image/upload', formData, {headers: {'Content-Type': 'multipart/form-data'}});
                        return res !== null ? {url: res} : false;
                    }
                }
            },
            {
                key: 'opacity',
                type: 'number-input',
                label: '透明度',
                value: opacity,
                config: {
                    min: 0,
                    max: 1,
                    step: 0.1
                }
            }
        ]
    }

    return (
        <div style={{padding: 10}}><DatavisoolGUI schema={schema} onFieldChange={onFieldChange}/></div>
    )
}
