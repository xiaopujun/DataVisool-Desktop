/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {action, makeObservable, observable, override} from "mobx";
import {globalMessage} from "@datavisool/ui";
import AuthFetchUtil from "@/util/AuthFetchUtil.ts";
import {EncryptResult, HybridCryptoUtil} from "@/util/HybridCryptoUtil.ts";
import {BaseResourceStore, IBaseResource} from "@/pages/home/common/BaseResourceStore.ts";
import {tx} from "@/i18n/miniI18n.ts";

export interface IReleaseProjectItem extends IBaseResource {
    id?: string;
    key?: string;
    name?: string;
    state?: boolean;
    createTime?: string;
    updateTime?: string;
    description?: string;
    des?: string;
    expirationDate?: number | null;
    encrypt?: boolean;
    password?: string | EncryptResult;
}

export class ReleaseStore extends BaseResourceStore<IReleaseProjectItem> {
    sortDirection: "asc" | "desc" = "desc";

    constructor() {
        super();
        makeObservable(this, {
            sortDirection: observable,
            setSortDirection: action.bound,
            getList: override,
            updateReleaseState: action,
            updateItem: override,
        });
    }

    getListApiUrl = '/api/release/pageList';
    deleteApiUrl = '/api/release/batchDel';

    protected processRecords(records: IReleaseProjectItem[]) {
        records.forEach((item) => item.key = item.id);
    }

    setSortDirection(direction: "asc" | "desc") {
        this.sortDirection = direction;
    }

    getList(extraParams: Record<string, any> = {}) {
        super.getList({...extraParams, sortDirection: this.sortDirection});
    }

    async updateItem() {
        const {id, name, expirationDate, encrypt, password} = this.currentItem;
        if (!id) return;
        try {
            const encoded = typeof password === "string" && password
                ? await HybridCryptoUtil.encryptRequest(password)
                : undefined;
            const res = await AuthFetchUtil.post<boolean | null>('/api/release/update', {
                id, name, expirationDate: expirationDate ?? null, encrypt,
                ...(encoded ? {password: encoded} : {}),
            });
            if (res === true) {
                this.setModalVisible(false);
                this.getList();
                globalMessage.messageApi?.success(tx("操作成功"));
            } else if (res === false) {
                globalMessage.messageApi?.error(tx("发布记录不存在或已删除"));
            }
        } catch (error) {
            console.error("Failed to update release", error);
            globalMessage.messageApi?.error(tx("更新失败，请稍后重试"));
        }
    }

    updateReleaseState = (id: string, state: boolean) => {
        AuthFetchUtil.post('/api/release/update', {id, state: state ? 1 : 0}).then(res => {
            res !== null && globalMessage.messageApi?.success(tx('操作成功'));
        });
    };
}

const releaseStore = new ReleaseStore();
export default releaseStore;
