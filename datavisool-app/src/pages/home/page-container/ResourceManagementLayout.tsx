/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {observer} from "mobx-react";
import {BaseResourceStore, IBaseResource} from "@/pages/home/common/BaseResourceStore.ts";
import PageContainer from "@/pages/home/page-container/PageContainer.tsx";
import {Button, Dialog, Input, Pagination, Progress, Upload as DataVisoolUpload, type UploadFile} from "@datavisool/ui";
import {Delete, Search, Upload as UploadIcon} from "@icon-park/react";
import React, {KeyboardEvent, useEffect, useState} from "react";
import {globalMessage} from "@datavisool/ui";
import {globalModal} from "@datavisool/ui";
import styles from "@/pages/home/page-container/ResourceManagementLayout.module.less";
import {tx} from "@/i18n/miniI18n.ts";

const {Dragger} = DataVisoolUpload;

interface ResourceManagementLayoutProps<T extends IBaseResource> {
    store: BaseResourceStore<T>;
    title: string;
    searchPlaceholder: string;
    renderCard?: (item: T, isSelected: boolean, toggleSelection: (id: string) => void) => React.ReactNode;
    renderList?: (data: T[], selectedIds: string[], setSelectedIds: React.Dispatch<React.SetStateAction<string[]>>) => React.ReactNode;
    uploadConfig?: {
        accept?: string;
        maxCount?: number;
        validateFile?: (file: File) => boolean;
        uploadAction: (files: File[], id?: string, onProgress?: (percent: number) => void) => Promise<boolean>;
        tipText?: string;
        modalTitle: string;
    };
    deleteTip?: string;
    renderEditComponent?: () => React.ReactNode;
    extraHeaderTools?: React.ReactNode[];
    createButtonText?: string;
    createButtonIcon?: React.ReactNode;
    showCreateButton?: boolean;
}

function ResourceManagementLayout<T extends IBaseResource>(props: ResourceManagementLayoutProps<T>) {
    const {
        store,
        title,
        searchPlaceholder,
        renderCard,
        renderList,
        uploadConfig,
        deleteTip,
        renderEditComponent,
        extraHeaderTools,
        createButtonText = '上传',
        createButtonIcon = <UploadIcon style={{marginRight: 3}}/>,
        showCreateButton = true
    } = props;
    const {pageData, modalVisible} = store;
    const {records, current, size, total} = pageData;

    const [selectedIds, setSelectedIds] = useState<string[]>([]);
    const [fileList, setFileList] = useState<UploadFile[]>([]);
    const [uploadProgress, setUploadProgress] = useState<number>(0);
    const [uploading, setUploading] = useState<boolean>(false);

    useEffect(() => {
        store.init();
        return () => store.destroy();
    }, [store]);

    const doSearch = (value?: string) => {
        store.searchValue = value ?? '';
        store.getList();
    }

    const onKeyDown = (event: React.KeyboardEvent) => {
        if (event.key !== 'Enter') return;
        doSearch((event.target as HTMLInputElement).value);
    }

    const onClose = () => {
        if (uploading) return;
        store.setModalVisible(false);
        setFileList([]);
        setUploadProgress(0);
    }

    const toggleSelection = (id: string) => {
        if (selectedIds.includes(id)) {
            setSelectedIds(selectedIds.filter(item => item !== id));
        } else {
            setSelectedIds([...selectedIds, id]);
        }
    }

    const handleDelete = () => {
        if (selectedIds.length === 0) {
            globalMessage.messageApi?.warning(tx('请选择要删除的项目'));
            return;
        }
        globalModal.modalApi?.confirm({
            title: tx('删除确认'),
            content: tx(deleteTip || '确定删除选中的项目吗？删除后无法恢复，请谨慎操作！'),
            onOk: () => {
                store.doBatchDelete(selectedIds);
                setSelectedIds([]);
            },
            okText: tx('确定'),
            cancelText: tx('取消'),
        });
    }

    const handleUpload = () => {
        if (fileList.length > 0 && uploadConfig) {
            setUploading(true);
            setUploadProgress(0);
            const files = fileList.map(f => f.originFileObj as File).filter(Boolean);
            uploadConfig.uploadAction(files, store.currentItem.id, (percent) => {
                setUploadProgress(percent);
            }).then(res => {
                if (res) {
                    globalMessage.messageApi?.success(tx('操作成功'));
                    setFileList([]);
                    store.setModalVisible(false);
                    store.getList();
                }
            }).finally(() => {
                setUploading(false);
                setUploadProgress(0);
            });
        }
    }

    return (
        <PageContainer
            title={title}
            tools={[
                <Input
                    placeholder={searchPlaceholder}
                    prefix={<Search color={'#A0A0B2'} size={12}/>}
                    className={styles.searchInput}
                    onKeyDown={onKeyDown}
                    onPressEnter={(e: KeyboardEvent) => doSearch((e.target as HTMLInputElement).value)}
                />,
                ...(extraHeaderTools || []),
                showCreateButton && (
                    <Button
                        type={"primary"}
                        onClick={() => store.setModalVisible(true)}
                        className={styles.uploadButton}
                    >
                        {createButtonIcon}
                        {createButtonText}
                    </Button>
                ),
                <Button
                    variant="destructive"
                    onClick={handleDelete}
                    className={styles.deleteButton}
                >
                    <Delete size={12}/>
                    {'删除'}
                </Button>
            ]}
            content={
                <>
                    {renderList ? renderList(records, selectedIds, setSelectedIds) : (
                        <div className={styles.resourceGrid}>
                            {records.map((record: T) => (
                                <React.Fragment key={record.id}>
                                    {renderCard && renderCard(record, selectedIds.includes(record.id!), toggleSelection)}
                                </React.Fragment>
                            ))}
                        </div>
                    )}

                    <div className={styles.paginationContainer}>
                        <Pagination
                            current={current}
                            pageSize={size}
                            total={total}
                            onChange={(page) => store.changeCurrentPage(page)}
                            showSizeChanger={false}
                            showTotal={(total, range) => `第 ${range[0]}-${range[1]} 条，共 ${total} 条`}
                            className={styles.pagination}
                        />
                    </div>

                    {renderEditComponent ? renderEditComponent() : (
                        uploadConfig && (
                            <Dialog
                                open={modalVisible}
                                title={uploadConfig.modalTitle}
                                onClose={onClose}
                                width={520}
                                mask
                                centered
                                showClose
                                className={styles.uploadModal}
                            >
                                <Dragger
                                    multiple={true}
                                    accept={uploadConfig.accept}
                                    maxCount={uploadConfig.maxCount || 10}
                                    fileList={fileList}
                                    beforeUpload={(file: File) => {
                                        if (uploadConfig.validateFile && !uploadConfig.validateFile(file)) {
                                            return DataVisoolUpload.LIST_IGNORE;
                                        }
                                        setFileList(prev => [...prev, {
                                            uid: file.name + Date.now(),
                                            name: file.name,
                                            status: 'done',
                                            originFileObj: file,
                                        }]);
                                        return false;
                                    }}
                                    onRemove={(file) => {
                                        setFileList(prev => prev.filter(item => item.uid !== file.uid));
                                    }}
                                    className={styles.uploadDragger}
                                    disabled={uploading}
                                >
                                    <UploadIcon fill={'#717171'} size={20}/>
                                    <p className={styles.uploadText}>{uploadConfig.tipText || '点击或拖拽文件上传'}</p>
                                </Dragger>
                                {uploading && (
                                    <div style={{marginTop: 16}}>
                                        <Progress percent={Math.round(uploadProgress)} status="active" />
                                    </div>
                                )}
                                <div className={styles.uploadModalFooter}>
                                    <Button
                                        type={"primary"}
                                        onClick={handleUpload}
                                        className={styles.modalUploadButton}
                                        loading={uploading}
                                        disabled={fileList.length === 0}
                                    >
                                        {uploading ? '上传中...' : createButtonText}
                                    </Button>
                                    <Button
                                        type="default"
                                        onClick={onClose}
                                        className={styles.modalCancelButton}
                                        disabled={uploading}
                                    >
                                        {'取消'}
                                    </Button>
                                </div>
                            </Dialog>
                        )
                    )}
                </>
            }
        />
    );
}

const ResourceManagementLayoutView = observer(ResourceManagementLayout);
export default ResourceManagementLayoutView;
