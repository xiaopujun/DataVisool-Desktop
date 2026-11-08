/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {useEffect, useState} from "react";
import type {MouseEvent} from "react";
import {observer} from "mobx-react";
import {Button, Dialog, Dropdown, Empty, Input, Pagination, Progress, Upload as DataVisoolUpload} from "@datavisool/ui";
import type {DropdownMenuItem, UploadFile} from "@datavisool/ui";
import {Copy, Delete, Edit, More, PlayOne, Search, Upload, VideoOne} from "@icon-park/react";
import videoResourceStore, {IVideoResource} from "@/pages/home/video-resource/VideoResourceStore";
import AuthFetchUtil from "@/util/AuthFetchUtil";
import {globalMessage} from "@datavisool/ui";
import {globalModal} from "@datavisool/ui";
import axios from "axios";
import AssetUrlUtil from "@/util/AssetUrlUtil.ts";
import styles from "@/pages/home/video-resource/VideoResourceManagement.module.less";
import fileIcon from "@/pages/home/video-resource/assets/video-file-icon.webp";
import thumbPurple from "@/pages/home/video-resource/assets/video-thumb-purple.webp";
import thumbCyan from "@/pages/home/video-resource/assets/video-thumb-cyan.webp";
import {tx} from "@/i18n/miniI18n.ts";
import {formatResourceSize} from "@/pages/home/common/formatResourceSize.ts";

const {Dragger} = DataVisoolUpload;

const getDisplayName = (item: IVideoResource) => item.name?.trim() || "未命名视频";

const getVideoType = (item: IVideoResource) => {
    const mimeType = item.type?.trim().toLowerCase();
    if (mimeType === "video/quicktime") return "MOV";
    if (mimeType?.includes("/")) return mimeType.split("/").pop()?.slice(0, 4).toUpperCase() || "VIDEO";

    const source = item.name || item.url || "";
    const extension = source.split("?")[0].split(".").pop();
    if (!extension || extension === source) return "MOV";
    return extension.slice(0, 4).toUpperCase();
};

const VideoResourceManagement = observer(() => {
    const isTauri = AssetUrlUtil.isTauri();
    const {pageData, modalVisible} = videoResourceStore;
    const {records, current, size, total} = pageData;
    const [previewVideo, setPreviewVideo] = useState<IVideoResource | null>(null);
    const [selectedIds, setSelectedIds] = useState<string[]>([]);
    const [fileList, setFileList] = useState<UploadFile[]>([]);
    const [uploadProgress, setUploadProgress] = useState(0);
    const [uploading, setUploading] = useState(false);

    useEffect(() => {
        videoResourceStore.init();
        return () => videoResourceStore.destroy();
    }, []);

    useEffect(() => {
        setSelectedIds((prev) => {
            const availableIds = new Set(records.map((item) => item.id).filter((id): id is string => Boolean(id)));
            return prev.filter((id) => availableIds.has(id));
        });
    }, [records]);

    const resolveVideoUrl = (url?: string) => {
        if (!url) return url;
        return AssetUrlUtil.toLoadableUrl(url);
    };

    const doSearch = (value?: string) => {
        videoResourceStore.searchValue = value?.trim() ?? "";
        videoResourceStore.pageData.current = 1;
        videoResourceStore.getList();
    };

    const toggleSelection = (id?: string) => {
        if (!id) return;
        setSelectedIds((prev) => (
            prev.includes(id)
                ? prev.filter((item) => item !== id)
                : [...prev, id]
        ));
    };

    const handleRename = (item: IVideoResource) => {
        let newName = getDisplayName(item);
        globalModal.modalApi?.confirm({
            title: tx("重命名视频"),
            content: (
                <Input
                    defaultValue={newName}
                    onChange={(value) => newName = value}
                    placeholder={tx("请输入新的视频名称")}
                />
            ),
            okText: tx("确定"),
            cancelText: tx("取消"),
            onOk: async () => {
                if (!newName.trim() || !item.id) return;
                await AuthFetchUtil.post("/api/video/update", {id: item.id, name: newName.trim()});
                videoResourceStore.getList();
                globalMessage.messageApi?.success(tx("重命名成功"));
            }
        });
    };

    const handleCopyUrl = (event: MouseEvent, url?: string) => {
        event.stopPropagation();
        if (!url || !navigator.clipboard) return;
        navigator.clipboard.writeText(resolveVideoUrl(url) || url).then(() => {
            globalMessage.messageApi?.success(tx("链接已复制"));
        });
    };

    const handleSingleDelete = (event: MouseEvent, id?: string) => {
        event.stopPropagation();
        if (!id) return;
        globalModal.modalApi?.confirm({
            title: tx("删除确认"),
            content: tx("确定要删除这个视频吗？删除后无法恢复，请谨慎操作。"),
            okText: tx("确定"),
            cancelText: tx("取消"),
            onOk: () => videoResourceStore.doBatchDelete([id])
        });
    };

    const handleBatchDelete = () => {
        if (selectedIds.length === 0) {
            globalMessage.messageApi?.warning(tx("请选择要删除的视频"));
            return;
        }

        globalModal.modalApi?.confirm({
            title: tx("删除确认"),
            content: tx("确定要删除选中的视频吗？删除后无法恢复，请谨慎操作。"),
            okText: tx("确定"),
            cancelText: tx("取消"),
            onOk: () => {
                videoResourceStore.doBatchDelete(selectedIds);
                setSelectedIds([]);
            },
        });
    };

    const uploadAction = (files: File[], id?: string, onProgress?: (percent: number) => void): Promise<boolean> => {
        return new Promise((resolve) => {
            if (isTauri) {
                const maxBytes = 200 * 1024 * 1024;
                const tooLarge = files.find((file) => file.size > maxBytes);
                if (tooLarge) {
                    globalMessage.messageApi?.error(tx(`文件过大：${tooLarge.name}，请控制在 200MB 以内`));
                    resolve(false);
                    return;
                }

                const arrayBufferToBase64 = (buffer: ArrayBuffer) => {
                    const bytes = new Uint8Array(buffer);
                    const chunkSize = 0x8000;
                    let binary = "";
                    for (let i = 0; i < bytes.length; i += chunkSize) {
                        binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize));
                    }
                    return btoa(binary);
                };

                onProgress?.(5);
                Promise.all(files.map(async (file) => {
                    const buf = await file.arrayBuffer();
                    const base64 = arrayBufferToBase64(buf);
                    const mime = file.type || "video/mp4";
                    return {
                        fileName: file.name,
                        mime,
                        dataUrl: `data:${mime};base64,${base64}`
                    };
                })).then(async (payloadFiles) => {
                    onProgress?.(85);
                    const res = await AuthFetchUtil.post("/api/video/upload", {files: payloadFiles, id});
                    if (res !== null) {
                        onProgress?.(100);
                        resolve(true);
                    } else {
                        resolve(false);
                    }
                }).catch((error) => {
                    globalMessage.messageApi?.error(tx("上传失败"));
                    console.error(error);
                    resolve(false);
                });
                return;
            }

            const formData = new FormData();
            files.forEach((file) => {
                formData.append("files", file);
            });

            axios.post("/api/video/upload", formData, {
                headers: {
                    "Content-Type": "multipart/form-data"
                },
                onUploadProgress: (progressEvent) => {
                    if (progressEvent.total && onProgress) {
                        onProgress((progressEvent.loaded / progressEvent.total) * 100);
                    }
                }
            }).then((response) => {
                if (response.status === 200) {
                    const res = response.data;
                    if (res.code === 200) {
                        resolve(true);
                    } else {
                        globalMessage.messageApi?.error(tx(res.msg || "上传失败"));
                        resolve(false);
                    }
                } else {
                    globalMessage.messageApi?.error(tx(`上传失败: ${response.statusText}`));
                    resolve(false);
                }
            }).catch((error) => {
                globalMessage.messageApi?.error(tx("网络错误，上传失败"));
                console.error(error);
                resolve(false);
            });
        });
    };

    const handleUpload = () => {
        if (fileList.length === 0) return;

        setUploading(true);
        setUploadProgress(0);
        const files = fileList.map((file) => file.originFileObj as File).filter(Boolean);
        uploadAction(files, videoResourceStore.currentItem.id, setUploadProgress).then((res) => {
            if (res) {
                globalMessage.messageApi?.success(tx("上传成功"));
                setFileList([]);
                videoResourceStore.setModalVisible(false);
                videoResourceStore.getList();
            }
        }).finally(() => {
            setUploading(false);
            setUploadProgress(0);
        });
    };

    const closeUploadModal = () => {
        if (uploading) return;
        videoResourceStore.setModalVisible(false);
        setFileList([]);
        setUploadProgress(0);
    };

    const renderVideoCard = (item: IVideoResource, index: number) => {
        const id = item.id;
        const isSelected = selectedIds.includes(id || "");
        const isCyan = index % 2 === 1;
        const menuItems: DropdownMenuItem[] = [
            {
                key: "rename",
                label: "重命名",
                icon: <Edit />,
                onClick: ({domEvent}) => {
                    domEvent.stopPropagation();
                    handleRename(item);
                }
            },
            {
                key: "copy",
                label: "复制链接",
                icon: <Copy />,
                onClick: ({domEvent}) => handleCopyUrl(domEvent as unknown as MouseEvent, item.url)
            },
            {
                key: "delete",
                label: "删除",
                icon: <Delete />,
                danger: true,
                onClick: ({domEvent}) => handleSingleDelete(domEvent as unknown as MouseEvent, id)
            }
        ];

        return (
            <article
                key={id || `${item.url}-${index}`}
                className={`${styles.videoCard} ${isCyan ? styles.cyanCard : styles.purpleCard} ${isSelected ? styles.selectedCard : ""}`}
                onClick={() => toggleSelection(id)}
            >
                <div className={styles.thumbnailWrap}>
                    <img
                        className={styles.thumbnailImage}
                        src={isCyan ? thumbCyan : thumbPurple}
                        alt=""
                    />
                    <div className={styles.videoTag}>{getVideoType(item)}</div>
                    <button type="button" className={styles.playControl} aria-label={tx("播放视频")}
                        onClick={(event) => { event.stopPropagation(); setPreviewVideo(item); }}>
                        <PlayOne theme="filled" size={38} fill="#ffffff" />
                    </button>
                </div>

                <footer className={styles.cardFooter}>
                    <img className={styles.fileIcon} src={fileIcon} alt="" />
                    <div className={styles.fileInfo}>
                        <h2 title={getDisplayName(item)}>{getDisplayName(item)}</h2>
                        <p>{formatResourceSize(item.size)}</p>
                    </div>
                    <Dropdown menu={{items: menuItems}} trigger={["click"]} placement="bottomRight">
                        <button
                            type="button"
                            className={styles.moreButton}
                            aria-label="更多操作"
                            onClick={(event) => event.stopPropagation()}
                        >
                            <More size={24} />
                        </button>
                    </Dropdown>
                </footer>
            </article>
        );
    };

    return (
        <div className={styles.videoResourcePage}>
            <header className={styles.pageHeader}>
                <div className={styles.headerTitleArea}>
                    <span className={styles.titleIcon} aria-hidden="true">
                        <VideoOne size={56} fill="#6e8dff" />
                    </span>
                    <div className={styles.titleText}>
                        <h1>视频资源管理</h1>
                        <p>集中管理和维护所有视频资源，高效组织与快速访问</p>
                    </div>
                </div>

                <div className={styles.headerTools}>
                    <Input
                        className={styles.searchInputField}
                        controlClassName={styles.searchInput}
                        placeholder="搜索视频名称"
                        prefix={<Search size={20} fill="#9EABC8" />}
                        onPressEnter={(event) => doSearch(event.currentTarget.value)}
                    />
                    <Button
                        className={styles.uploadButton}
                        type="primary"
                        icon={<Upload size={18} />}
                        onClick={() => videoResourceStore.setModalVisible(true)}
                    >
                        上传
                    </Button>
                    <Button
                        className={styles.deleteButton}
                        type="default"
                        icon={<Delete size={18} />}
                        onClick={handleBatchDelete}
                    >
                        删除
                    </Button>
                </div>
            </header>

            <main className={styles.pageBody}>
                <section className={styles.resourcePane}>
                    {records.length === 0 ? (
                        <div className={styles.emptyWrap}>
                            <Empty title={null} description="暂无视频资源">
                                <Button
                                    className={styles.uploadButton}
                                    type="primary"
                                    icon={<Upload size={16} />}
                                    onClick={() => videoResourceStore.setModalVisible(true)}
                                >
                                    上传
                                </Button>
                            </Empty>
                        </div>
                    ) : (
                        <div className={styles.videoGrid}>
                            {records.map((item, index) => renderVideoCard(item, index))}
                        </div>
                    )}

                    <div className={styles.paginationContainer}>
                        <Pagination
                            current={current}
                            pageSize={size}
                            total={total}
                            onChange={(page) => videoResourceStore.changeCurrentPage(page)}
                            showSizeChanger={false}
                            showTotal={(totalCount, range) => `第 ${range[0]}-${range[1]} 条，共 ${totalCount} 条`}
                            controlClassName={styles.pagination}
                        />
                    </div>
                </section>
            </main>

            <Dialog open={!!previewVideo} title={previewVideo ? getDisplayName(previewVideo) : "视频预览"}
                onClose={() => setPreviewVideo(null)} mask centered showClose width="min(800px, 90vw)">
                {previewVideo && <video controls autoPlay style={{width: "100%", maxHeight: "65vh"}} src={resolveVideoUrl(previewVideo.url)}/>}
            </Dialog>
            <Dialog
                open={modalVisible}
                title="上传视频"
                onClose={closeUploadModal}
                width={520}
                mask
                centered
                showClose
                className={styles.uploadModal}
            >
                <Dragger
                    multiple
                    maxCount={10}
                    accept="video/*"
                    fileList={fileList}
                    beforeUpload={(file: File) => {
                        if (!(file.type.toLowerCase().startsWith("video/") || (!file.type && /\.(mp4|mov|webm|m4v|avi|mkv|ogv)$/i.test(file.name)))) {
                            globalMessage.messageApi?.error(tx("请上传视频文件"));
                            return DataVisoolUpload.LIST_IGNORE;
                        }
                        setFileList((prev) => [...prev, {
                            uid: `${file.name}-${Date.now()}`,
                            name: file.name,
                            status: "done",
                            originFileObj: file,
                        }]);
                        return false;
                    }}
                    onRemove={(file) => {
                        setFileList((prev) => prev.filter((item) => item.uid !== file.uid));
                    }}
                    className={styles.uploadDragger}
                    disabled={uploading}
                >
                    <Upload fill="#7785A7" size={28} />
                    <p className={styles.uploadText}>点击或拖拽视频文件上传</p>
                </Dragger>
                {uploading && (
                    <div className={styles.progressWrap}>
                        <Progress percent={Math.round(uploadProgress)} status="active" />
                    </div>
                )}
                <div className={styles.uploadModalFooter}>
                    <Button
                        type="primary"
                        onClick={handleUpload}
                        className={styles.modalUploadButton}
                        loading={uploading}
                        disabled={fileList.length === 0}
                    >
                        {uploading ? "上传中..." : "上传"}
                    </Button>
                    <Button
                        onClick={closeUploadModal}
                        className={styles.modalCancelButton}
                        type="default"
                        disabled={uploading}
                    >
                        取消
                    </Button>
                </div>
            </Dialog>
        </div>
    );
});

export default VideoResourceManagement;
