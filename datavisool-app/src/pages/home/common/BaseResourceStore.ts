/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {action, makeObservable, observable, runInAction} from "mobx";
import {IPage} from "@datavisool/sdk/types";
import AuthFetchUtil from "@/util/AuthFetchUtil.ts";
import {globalMessage} from "@datavisool/ui";
import {tx} from "@/i18n/miniI18n.ts";

export interface IBaseResource {
    id?: string;
    [key: string]: any;
}

export abstract class BaseResourceStore<T extends IBaseResource> {
    constructor() {
        makeObservable(this, {
            modalVisible: observable,
            pageData: observable,
            currentItem: observable,
            setPageData: action.bound,
            setCurrentItem: action.bound,
            changeCurrentPage: action.bound,
            setModalVisible: action.bound,
            destroy: action.bound,
            getList: action.bound,
            doBatchDelete: action.bound,
            updateItem: action.bound,
            saveItem: action.bound,
            copyItem: action.bound,
            fetchAndEdit: action.bound,
            init: action.bound
        })
    }

    modalVisible = false;
    searchValue: string | null = null;
    pageData: IPage<T> = {
        records: [],
        total: 0,
        size: 10,
        current: 1
    };
    currentItem: Partial<T> = {};
    private listRequestVersion = 0;

    // Abstract properties/methods to be implemented by subclasses
    abstract getListApiUrl: string;
    abstract deleteApiUrl: string;
    createApiUrl?: string;
    updateApiUrl?: string;

    setModalVisible(visible: boolean) {
        this.modalVisible = visible
        if (!visible) {
            this.currentItem = {};
        }
    }

    setPageData(pageData: IPage<T>) {
        this.pageData = pageData;
    }

    setCurrentItem(item: Partial<T>) {
        this.currentItem = item;
    }

    changeCurrentPage(page: number) {
        this.pageData.current = page;
        this.getList();
    }

    /**
     * Optional hook to process records after fetching
     */
    protected processRecords(records: T[]): void {
        records.forEach((item) => {
            if (item.id && !item.key) {
                (item as any).key = item.id;
            }
        });
    }

    getList(extraParams: Record<string, any> = {}) {
        const version = ++this.listRequestVersion;
        const params = {
            current: this.pageData.current,
            size: this.pageData.size,
            searchValue: this.searchValue,
            ...extraParams
        };
        const loadPage = (current: number): Promise<void> => (
            AuthFetchUtil.post<IPage<T>>(this.getListApiUrl, {...params, current}).then(res => {
                if (version !== this.listRequestVersion || res === null) return;
                const lastPage = Math.max(1, Math.ceil(res.total / res.size));
                if (res.current > lastPage) return loadPage(lastPage);
                runInAction(() => {
                    if (res.records) {
                        this.processRecords(res.records as T[]);
                    }
                    this.setPageData({
                        records: res.records,
                        total: res.total,
                        size: res.size,
                        current: res.current
                    });
                })
            })
        );
        void loadPage(params.current);
    }

    doBatchDelete(ids: string[]) {
        if (ids.length === 0)
            return;
        AuthFetchUtil.post(this.deleteApiUrl, ids).then((res) => {
            if (res !== null) {
                globalMessage.messageApi?.success(tx('删除成功'));
                this.getList();
            }
        })
    }

    updateItem() {
        this.saveItem();
    }

    saveItem() {
        const isUpdate = !!this.currentItem.id;
        const url = isUpdate ? this.updateApiUrl : this.createApiUrl;

        if (!url) {
            console.error("No API URL defined for save operation (create/update)");
            return;
        }

        AuthFetchUtil.post(url, this.currentItem).then(res => {
            if (res !== null) {
                runInAction(() => {
                    this.setModalVisible(false);
                    this.currentItem = {};
                    this.getList();
                    globalMessage.messageApi?.success(tx("操作成功"));
                })
            }
        })
    }

    copyItem(url: string) {
        AuthFetchUtil.get(url).then(res => {
            if (res !== null) {
                globalMessage.messageApi?.success(tx("复制成功"));
                this.getList();
            }
        });
    }

    fetchAndEdit(url: string) {
        AuthFetchUtil.get(url).then(res => {
            if (res !== null) {
                runInAction(() => {
                    this.setCurrentItem(res || {});
                    this.setModalVisible(true);
                });
            }
        });
    }

    init() {
        this.getList();
    }

    destroy() {
        this.listRequestVersion++;
        this.searchValue = null;
        this.pageData = {
            records: [],
            total: 0,
            size: 10,
            current: 1
        };
        this.currentItem = {};
    }
}
