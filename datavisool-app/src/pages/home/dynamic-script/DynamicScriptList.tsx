/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {globalModal} from "@datavisool/ui";
import {observer} from "mobx-react";
import {ChangeEvent, DragEvent, useEffect, useRef, useState} from "react";
import {Button, Dialog, Empty, Input, Pagination, Select} from "@datavisool/ui";
import {Check, Close, Code, Delete, Down, Earth, Edit, LinkOne, Plus, Search, Switch} from "@icon-park/react";
import scriptManagementStore, {IDynamicScript} from "@/pages/home/dynamic-script/DynamicScriptManagementStore.ts";
import {globalMessage} from "@datavisool/ui";
import styles from "@/pages/home/dynamic-script/DynamicScriptList.module.less";
import waveBg from "@/pages/home/dynamic-script/assets/dynamic-script-wave.webp";
import {tx} from "@/i18n/miniI18n.ts";

const getScriptTitle = (record: IDynamicScript) => record.name?.trim() || "未命名脚本";
const getScriptUrl = (record: IDynamicScript) => record.url?.trim() || "暂无脚本地址";

const DynamicScriptList = observer((props: { projectId?: string, title?: string, variant?: "resource" }) => {
    const {projectId, title, variant} = props;
    const {
        setModalVisible,
        setCurrentItem,
        currentItem,
        modalVisible,
        fileList,
        setFileList,
        saveOrUpdateDynamicScript
    } = scriptManagementStore;
    const {records, current, size, total} = scriptManagementStore.pageData;
    const [selectedIds, setSelectedIds] = useState<string[]>([]);
    const fileInputRef = useRef<HTMLInputElement>(null);

    useEffect(() => {
        scriptManagementStore.init(projectId);
        return () => scriptManagementStore.destroy();
    }, [projectId]);

    useEffect(() => {
        setSelectedIds((prev) => {
            const availableIds = new Set(records.map((item) => item.id).filter((id): id is string => Boolean(id)));
            const retainedIds = prev.filter((id) => availableIds.has(id));
            if (retainedIds.length > 0) return retainedIds;
            const firstId = records[0]?.id;
            return firstId ? [firstId] : [];
        });
    }, [records]);

    const doSearch = (value?: string) => {
        scriptManagementStore.searchValue = value?.trim() ?? "";
        scriptManagementStore.pageData.current = 1;
        scriptManagementStore.getList();
    };

    const toggleSelection = (id: string) => {
        setSelectedIds((prev) => (
            prev.includes(id)
                ? prev.filter((item) => item !== id)
                : [...prev, id]
        ));
    };

    const handleBatchDelete = () => {
        if (selectedIds.length === 0) {
            globalMessage.messageApi?.warning(tx("请选择要删除的项目"));
            return;
        }

        globalModal.modalApi?.confirm({
            title: tx("删除脚本"),
            content: tx("确定删除所选脚本吗？此操作无法撤销。"),
            tone: 'danger',
            okText: tx("删除"),
            cancelText: tx("取消"),
            okType: 'danger',
            onOk: () => {
                scriptManagementStore.doBatchDelete(selectedIds);
                setSelectedIds([]);
            },
        });
    };

    const handleDelete = (id: string) => {
        globalModal.modalApi?.confirm({
            title: tx("删除脚本"),
            content: tx("确定删除这个脚本吗？此操作无法撤销。"),
            tone: 'danger',
            okText: tx("删除"),
            cancelText: tx("取消"),
            okType: 'danger',
            onOk: () => scriptManagementStore.doBatchDelete([id]),
        });
    };

    const handleEditName = (record: IDynamicScript) => {
        setCurrentItem(record);
        globalModal.modalApi?.confirm({
            title: tx("修改脚本名称"),
            content: <div className={styles.renameContent}>
                <Input
                    placeholder={tx("请输入脚本名称")}
                    defaultValue={record.name}
                    onChange={(value) => {
                        setCurrentItem({...record, name: value});
                    }}
                />
            </div>,
            okText: tx("确定"),
            cancelText: tx("取消"),
            onOk: () => {
                const name = scriptManagementStore.currentItem.name?.trim();
                if (!name) { globalMessage.messageApi?.error(tx("脚本名称不能为空")); return false; }
                setCurrentItem({...record, name});
                scriptManagementStore.updateItem();
            }
        });
    };

    const renderScriptIcon = (record: IDynamicScript) => (
        <div className={`${styles.scriptIcon} ${record.type === "REMOTE" ? styles.remoteIcon : styles.localIcon}`}>
            {record.type === "REMOTE" ? <Earth size={34}/> : <Code size={34}/>}
        </div>
    );

    const renderCard = (record: IDynamicScript) => {
        const id = record.id!;
        const isSelected = selectedIds.includes(id);
        const isRemote = record.type === "REMOTE";

        return (
            <article
                key={id}
                className={`${styles.scriptCard} ${isSelected ? styles.selectedCard : ""}`}
                onClick={() => toggleSelection(id)}
            >
                <div className={styles.cardHeader}>
                    {renderScriptIcon(record)}
                    <div className={styles.cardTitleBlock}>
                        <h2 title={getScriptTitle(record)}>{getScriptTitle(record)}</h2>
                        <p title={getScriptUrl(record)}>{getScriptUrl(record)}</p>
                    </div>
                    <button
                        className={`${styles.selectBox} ${isSelected ? styles.selectBoxActive : ""}`}
                        type="button"
                        aria-label={isSelected ? "取消选择" : "选择脚本"}
                        onClick={(event) => {
                            event.stopPropagation();
                            toggleSelection(id);
                        }}
                    >
                        {isSelected && <Check size={18}/>}
                    </button>
                </div>

                <div className={styles.tagRow}>
                    <span className={isRemote ? styles.remoteTag : styles.localTag}>
                        {isRemote ? "在线脚本" : "本地脚本"}
                    </span>
                </div>

                <div className={styles.cardDivider}/>

                <div
                    className={styles.cardActions}
                    onClick={(event) => event.stopPropagation()}
                >
                    <button type="button" onClick={() => handleEditName(record)}>
                        <Edit size={19}/>
                        编辑
                    </button>
                    <button
                        type="button"
                        onClick={() => {
                            setCurrentItem({id: record.id, type: record.type});
                            setModalVisible(true);
                        }}
                    >
                        <Switch size={19}/>
                        替换
                    </button>
                    <button
                        type="button"
                        className={styles.deleteAction}
                        onClick={() => handleDelete(id)}
                    >
                        <Delete size={19}/>
                        删除
                    </button>
                </div>
            </article>
        );
    };

    const handleScriptTypeChange = (value: string | number | Array<string | number>) => {
        const nextType = Array.isArray(value) ? value[0] : value;
        setCurrentItem({
            ...currentItem,
            type: String(nextType),
            url: String(nextType) === "REMOTE" ? currentItem.url : undefined,
            scriptFile: undefined
        });
        setFileList([]);
    };

    const setScriptFile = (file?: File) => {
        if (!file) return;
        const jsMimeTypes = ["text/javascript", "application/javascript", "application/x-javascript", ""];
        if (!file.name.toLowerCase().endsWith(".js") || !jsMimeTypes.includes(file.type)) {
            globalMessage.messageApi?.error(tx("请上传js文件"));
            return;
        }

        setCurrentItem({
            ...currentItem,
            name: file.name,
            scriptFile: file
        });
        setFileList([{
            name: file.name,
            uid: `${Date.now()}`
        }]);
    };

    const clearScriptFile = () => {
        setCurrentItem({
            ...currentItem,
            name: undefined,
            scriptFile: undefined
        });
        setFileList([]);
    };

    const handleFileChange = (event: ChangeEvent<HTMLInputElement>) => {
        setScriptFile(event.target.files?.[0]);
        event.target.value = "";
    };

    const handleFileDrop = (event: DragEvent<HTMLDivElement>) => {
        event.preventDefault();
        setScriptFile(event.dataTransfer.files?.[0]);
    };

    const renderEditModal = () => (
        <Dialog
            open={modalVisible}
            onClose={() => setModalVisible(false)}
            className={`${styles.dynamicScriptModal} ${variant === "resource" ? styles.resourceDynamicScriptModal : ""}`}
            bodyClassName={`${styles.dynamicScriptModalBody} ${variant === "resource" ? styles.resourceDynamicScriptModalBody : ""}`}
            mask
            maskClosable={false}
            centered
            hideHeader
            width="min(784px, calc(100vw - 180px))"
        >
            <div className={styles.modalHeader}>
                <span className={styles.modalHeaderIcon} aria-hidden="true">
                    <Code size={24}/>
                </span>
                <div className={styles.modalHeaderCopy}>
                    <div className={styles.modalTitle}>{currentItem.id ? "替换动态脚本" : "添加动态脚本"}</div>
                    <div className={styles.modalSubtitle}>
                        {currentItem.id ? "选择脚本类型并替换现有动态脚本" : "选择脚本类型并输入脚本链接或 CDN 地址"}
                    </div>
                </div>
            </div>

            <div className={styles.modalForm}>
                <div className={styles.modalField}>
                    <div className={styles.modalFieldLabel}>
                        <span>脚本类型</span>
                        <span className={styles.modalHelpIcon}>?</span>
                    </div>
                    <Select
                        disabled={!!currentItem.id}
                        options={[
                            {label: "在线脚本", value: "REMOTE"},
                            {label: "本地脚本", value: "LOCAL"}
                        ]}
                        value={currentItem.type}
                        onChange={handleScriptTypeChange}
                        className={styles.typeSelect}
                        controlClassName={styles.typeSelectControl}
                        panelClassName={`${styles.typeSelectPanel} ${variant === "resource" ? styles.resourceTypeSelectPanel : ""}`}
                        prefix={<Code size={22}/>}
                        suffixIcon={<Down size={18}/>}
                        block
                    />
                </div>

                {currentItem.type === "REMOTE" ? (
                    <div className={styles.modalField}>
                        <div className={styles.modalFieldLabel}>脚本链接或 CDN 地址</div>
                        <Input
                            value={currentItem.url}
                            onChange={(value) => {
                                const scriptName = value.split("/").pop();
                                setCurrentItem({
                                    ...currentItem,
                                    url: value,
                                    name: scriptName
                                });
                            }}
                            controlClassName={styles.urlInputControl}
                            prefix={<LinkOne size={22}/>}
                            allowClear
                            maxLength={500}
                            autoComplete="off"
                            className={styles.urlInput}
                            placeholder="请输入脚本链接或 CDN 地址"
                        />
                    </div>
                ) : (
                    <div className={styles.modalField}>
                        <div className={styles.modalFieldLabel}>本地脚本文件</div>
                        <div
                            className={styles.localUploadArea}
                            role="button"
                            tabIndex={0}
                            onClick={() => fileInputRef.current?.click()}
                            onDragOver={(event) => event.preventDefault()}
                            onDrop={handleFileDrop}
                            onKeyDown={(event) => {
                                if (event.key === "Enter" || event.key === " ") {
                                    event.preventDefault();
                                    fileInputRef.current?.click();
                                }
                            }}
                        >
                            <input
                                ref={fileInputRef}
                                className={styles.fileInput}
                                type="file"
                                accept=".js,text/javascript,application/javascript,application/x-javascript"
                                onChange={handleFileChange}
                            />
                            <Code size={22}/>
                            <span>点击或拖拽上传 JavaScript 文件</span>
                        </div>
                        {fileList.length > 0 ? (
                            <div className={styles.selectedFileList}>
                                {fileList.map((file) => (
                                    <div className={styles.selectedFileItem} key={file.uid}>
                                        <Code size={18}/>
                                        <span className={styles.selectedFileName} title={file.name}>{file.name}</span>
                                        <button type="button" onClick={clearScriptFile} aria-label="移除已选择文件">
                                            <Close size={16}/>
                                        </button>
                                    </div>
                                ))}
                            </div>
                        ) : null}
                    </div>
                )}
            </div>

            <div className={styles.dynamicScriptModalFooter}>
                <Button
                    key="cancel"
                    type="default"
                    className={styles.modalCancelButton}
                    onClick={() => setModalVisible(false)}
                >
                    取消
                </Button>
                <Button
                    key="ok"
                    type="primary"
                    className={styles.modalSubmitButton}
                    onClick={() => saveOrUpdateDynamicScript()}
                >
                    {currentItem.id ? "替换" : "添加"}
                </Button>
            </div>
        </Dialog>
    );

    return (
        <div className={`${styles.dynamicScriptPage} ${variant === "resource" ? styles.resourceManagementPage : ""}`}>
            <img className={styles.waveBackground} src={waveBg} alt=""/>

            <header className={styles.pageHeader}>
                <div className={styles.headerTitleArea}>
                    <span className={styles.titleIcon} aria-hidden="true">
                        <Code size={56} fill="#6e8dff"/>
                    </span>
                    <div className={styles.titleText}>
                        <h1>{title || "动态脚本管理"}</h1>
                        <p>集中管理和维护所有动态脚本，灵活配置与高效执行</p>
                    </div>
                </div>

                <div className={styles.headerTools}>
                    <Input
                        className={styles.searchInputField}
                        controlClassName={styles.searchInput}
                        placeholder="搜索动态脚本"
                        prefix={<Search size={19} fill="#9aaad0"/>}
                        onPressEnter={(event) => doSearch(event.currentTarget.value)}
                    />
                    <Button
                        className={styles.createButton}
                        type="primary"
                        icon={<Plus size={22}/>}
                        onClick={() => setModalVisible(true)}
                    >
                        创建
                    </Button>
                    <Button
                        className={styles.batchDeleteButton}
                        type="default"
                        icon={<Delete size={18}/>}
                        onClick={handleBatchDelete}
                    >
                        删除
                    </Button>
                </div>
            </header>

            <main className={styles.pageBody}>
                {records.length === 0 ? (
                    <div className={styles.emptyWrap}>
                        <Empty title={null} description="暂无动态脚本">
                            <Button
                                className={styles.createButton}
                                type="primary"
                                icon={<Plus size={18}/>}
                                onClick={() => setModalVisible(true)}
                            >
                                创建
                            </Button>
                        </Empty>
                    </div>
                ) : (
                    <div className={styles.scriptGrid}>
                        {records.map(renderCard)}
                    </div>
                )}

                <div className={styles.paginationContainer}>
                    <Pagination
                        current={current}
                        pageSize={size}
                        total={total}
                        onChange={(page) => scriptManagementStore.changeCurrentPage(page)}
                        showSizeChanger={false}
                        showTotal={(totalCount, range) => `第 ${range[0]}-${range[1]} 条，共 ${totalCount} 条`}
                        controlClassName={styles.pagination}
                    />
                </div>
            </main>

            {renderEditModal()}
        </div>
    );
});

export default DynamicScriptList;
