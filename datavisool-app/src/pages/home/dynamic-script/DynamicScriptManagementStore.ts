/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {action, makeObservable, observable, override} from "mobx";
import {globalMessage} from "@datavisool/ui";
import AuthFetchUtil from "@/util/AuthFetchUtil.ts";
import {BaseResourceStore} from "@/pages/home/common/BaseResourceStore.ts";
import {tx} from "@/i18n/miniI18n.ts";

export interface IDynamicScript {
    id?: string;
    key?: string;
    name?: string;
    type?: string;
    url?: string;
    projectId?: string;
    scriptFile?: File;
}

export class DynamicScriptManagementStore extends BaseResourceStore<IDynamicScript> {
    constructor() {
        super();
        makeObservable(this, {
            fileList: observable,
            setFileList: action.bound,
            saveOrUpdateDynamicScript: action.bound,
            setModalVisible: override,
            init: override,
            getList: override
        });
        // Set default currentItem state
        this.currentItem = { type: 'REMOTE' };
    }

    getListApiUrl = '/api/dynamicScript/pageList';
    deleteApiUrl = '/api/dynamicScript/batchDel';
    updateApiUrl = '/api/dynamicScript/update';

    projectId: string | undefined = undefined;

    // 文件列表
    fileList: { name: string, uid: string }[] = [];

    setFileList(fileList: { name: string, uid: string }[]) {
        this.fileList = fileList;
    }

    setModalVisible(visible: boolean) {
        this.modalVisible = visible;
        if (!visible) {
            this.currentItem = { type: 'REMOTE', projectId: this.projectId };
            this.fileList = [];
        }
    }

    init(projectId?: string) {
        if (projectId) {
            this.projectId = projectId;
        }
        if (!this.currentItem.projectId) {
            this.currentItem = { ...this.currentItem, projectId: this.projectId };
        }
        this.getList();
    }

    getList() {
        super.getList({ projectId: this.projectId });
    }

    saveOrUpdateDynamicScript = async () => {
        const {name, type, url, id, scriptFile} = this.currentItem;
        const projectId = this.currentItem.projectId ?? this.projectId;
        if (type === 'LOCAL' && !scriptFile) {
            globalMessage.messageApi?.error(tx('请选择脚本文件'));
            return;
        }
        if (type === 'REMOTE' && !url) {
            globalMessage.messageApi?.error(tx('请输入远程脚本地址'));
            return;
        }

        if (type === 'REMOTE') {
            try {
                const parsed = new URL(url!.trim(), 'https://localhost/');
                if (!['http:', 'https:'].includes(parsed.protocol)) throw new Error('invalid protocol');
            } catch {
                globalMessage.messageApi?.error(tx('请输入有效的脚本地址'));
                return false;
            }
        }

        const isTauri =
            typeof window !== 'undefined' && (('__TAURI_INTERNALS__' in window) || ('__TAURI__' in window));

        if (isTauri) {
            const arrayBufferToBase64 = (buffer: ArrayBuffer) => {
                const bytes = new Uint8Array(buffer);
                const chunkSize = 0x8000;
                let binary = '';
                for (let i = 0; i < bytes.length; i += chunkSize) {
                    binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize));
                }
                return btoa(binary);
            };

            const payload: any = {
                id,
                name: name ?? '',
                type,
                url: url ?? '',
                projectId
            };

            if (type === 'LOCAL' && scriptFile) {
                const buf = await scriptFile.arrayBuffer();
                const base64 = arrayBufferToBase64(buf);
                payload.scriptFileName = scriptFile.name;
                payload.scriptDataUrl = `data:text/javascript;base64,${base64}`;
                payload.url = '';
            }

            const res = await AuthFetchUtil.post('/api/dynamicScript/saveOrUpdate', payload);
            if (res !== null) {
                globalMessage.messageApi?.success(tx('操作成功'));
                this.setModalVisible(false);
                this.getList();
                return true;
            }
            return false;
        }

        const formData = new FormData();
        if (scriptFile) formData.append('scriptFile', scriptFile);
        formData.append('name', name ?? '');
        formData.append('type', type!);
        url && formData.append('url', url ?? '');
        projectId && formData.append('projectId', projectId ?? '');
        id && formData.append('id', id ?? '');

        const res = await AuthFetchUtil.post('/api/dynamicScript/saveOrUpdate', formData, {
            headers: {'Content-Type': 'multipart/form-data'}
        });
        if (res !== null) {
            globalMessage.messageApi?.success(tx('操作成功'));
            this.setModalVisible(false);
            this.getList();
            return true;
        }
        return false;
    }
}

const scriptManagementStore = new DynamicScriptManagementStore();
export default scriptManagementStore;
