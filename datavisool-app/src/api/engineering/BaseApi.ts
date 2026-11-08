/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {URLUtil} from "@datavisool/sdk/utils";
import {IImageData} from "@/comps/builtin/base-image/BaseImageComponent.tsx";
import AuthFetchUtil from "@/util/AuthFetchUtil.ts";
import {IPage, IPageParam, IProject, IProjectInfoManager, IProjectMeta} from "@datavisool/sdk/types";
import {AbstractApi} from "@/api/engineering/AbstractApi.ts";
import ProjectUtil from "@/util/ProjectUtil.ts";
import ProjectDataCodecUtil from "@/util/ProjectDataCodecUtil.ts";

interface TemplateCoverPayload {
    fileName: string;
    dataUrl: string;
    mime?: string;
}

interface TemplateCreatePayload {
    name: string;
    description: string;
    templateJson: string;
    coverFile?: TemplateCoverPayload;
}

export class BaseApi extends AbstractApi {

    async createProject(project: IProjectInfoManager): Promise<string> {
        return await AuthFetchUtil.post<string>('/api/project/create', project);
    }

    async copyProject(id: string): Promise<string> {
        return await AuthFetchUtil.get<string>(`/api/project/copy/${id}`);
    }

    async deleteProject(id: string): Promise<boolean> {
        return await AuthFetchUtil.get<boolean>(`/api/project/del/${id}`);
    }

    async getProjectData(id: string): Promise<IProject | null> {
        const data = await AuthFetchUtil.get<{ id: string, name: string, dataJson: string } | null>(`/api/project/getProjectData/${id}`);
        if (data !== null) {
            const {id, name, dataJson} = data;
            const decoded = ProjectDataCodecUtil.decompressIfNeeded(dataJson);
            const projectData = ProjectUtil.deserializeProjectData(decoded);
            return {id, name, data: projectData};
        }
        return null;
    }

    async getProjectMeta(id: string): Promise<IProjectMeta | null> {
        const data = await AuthFetchUtil.get<{ id: string, name: string, pid: string | null, dataJson: string } | null>(`/api/project/getProjectMeta/${id}`);
        if (data !== null) {
            const {id, name, pid, dataJson} = data;
            const decoded = ProjectDataCodecUtil.decompressIfNeeded(dataJson);
            const projectData = ProjectUtil.deserializeProjectData(decoded);
            return {id, name, pid, data: projectData};
        }
        return null;
    }

    /**
     * 获取项目分页列表
     */
    public async getProjectInfoPageList(pageParam: IPageParam): Promise<IPage<IProjectInfoManager>> {
        const data = await AuthFetchUtil.post<IPage<IProjectInfoManager>>('/api/project/pageList', pageParam);
        return data || {records: [], total: 0, current: 1, size: 10};
    }

    public async updateProject(data: IProjectInfoManager): Promise<boolean> {
        return await AuthFetchUtil.post<boolean>('/api/project/update', data);
    }

    async getImageSourceList(): Promise<IImageData[]> {
        const data = await AuthFetchUtil.get<IImageData[]>(`/api/image/getUserUploadedImageList`);
        return data || [];
    }

    public async delImageSource(imageId: string): Promise<boolean> {
        return await AuthFetchUtil.get<boolean>(`/api/image/del/${imageId}`);
    }

    public async getVideoSourceList(name?: string): Promise<any[]> {
        const data = await AuthFetchUtil.post<IPage<any>>(`/api/video/page`, {
            current: 1,
            size: 100, // 获取足够多的数量，或者改为滚动加载
            searchValue: name
        });
        return data?.records || [];
    }

    public async delVideoSource(videoId: string): Promise<boolean> {
        return await AuthFetchUtil.get<boolean>(`/api/video/delete/${videoId}`);
    }

    public async uploadCover(file: File): Promise<boolean> {
        const {id} = URLUtil.parseUrlParams();
        const isTauri = typeof window !== 'undefined' && (('__TAURI_INTERNALS__' in window) || ('__TAURI__' in window));
        if (isTauri) {
            if (!id) return false;
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
            const coverUrl = await AuthFetchUtil.post<string>('/api/image/upload', {
                fileName: file.name,
                dataUrl: `data:${mime};base64,${base64}`,
                hash,
                name: file.name
            });

            if (!coverUrl) return false;
            return await AuthFetchUtil.post<boolean>('/api/project/update', {id, cover: coverUrl});
        }

        const formData = new FormData();
        formData.append('file', file);
        formData.append('id', id);
        return await AuthFetchUtil.post<boolean>('/api/project/cover', formData, {headers: {'Content-Type': 'multipart/form-data'}});
    }

    /**
     * 获取远程组件列表
     */
    public async getAllRemoteComponents(): Promise<{ id: string, url: string }[]> {
        const data = await AuthFetchUtil.get<{ id: string, url: string }[]>('/api/remote/component/list');
        return data || [];
    }

    public async uploadRemoteComponent(file: File, id?: string): Promise<boolean> {
        const dataUrl = await new Promise<string>((resolve, reject) => {
            const reader = new FileReader();
            reader.onload = () => resolve(String(reader.result || ''));
            reader.onerror = () => reject(new Error('读取文件失败'));
            reader.readAsDataURL(file);
        });
        return await AuthFetchUtil.post<boolean>(
            '/api/remote/component/upload',
            {
                id,
                fileName: file.name,
                dataUrl,
                mime: file.type || undefined
            },
            {
                loadingText: '正在上传服务器组件...'
            }
        );
    }

    public async createTemplate(template: FormData): Promise<boolean> {
        if (BaseApi.isTauri()) {
            const payload = await BaseApi.buildDesktopTemplatePayload(template);
            return await AuthFetchUtil.post<boolean>('/api/template/create', payload);
        }
        return await AuthFetchUtil.post<boolean>('/api/template/create', template, {headers: {'Content-Type': 'multipart/form-data'}});
    }

    public async getTemplateList(name?: string): Promise<{ id: string, name: string }[]> {
        return await AuthFetchUtil.post<{ id: string, name: string }[]>('/api/template/getAllTemplate', {name: name});
    }

    public async getTemplate(id: string): Promise<any> {
        return await AuthFetchUtil.get<any>(`/api/template/getTemplate/${id}`);
    }

    public async deleteTemplate(ids: string[]): Promise<boolean> {
        return await AuthFetchUtil.post<boolean>(`/api/template/batchDel`, ids);
    }

    private static isTauri(): boolean {
        return typeof window !== 'undefined' && (('__TAURI_INTERNALS__' in window) || ('__TAURI__' in window));
    }

    private static async buildDesktopTemplatePayload(formData: FormData): Promise<TemplateCreatePayload> {
        const payload: TemplateCreatePayload = {
            name: String(formData.get('name') || ''),
            description: String(formData.get('description') || ''),
            templateJson: String(formData.get('templateJson') || ''),
        };

        const coverFile = formData.get('coverFile');
        if (coverFile instanceof File) {
            payload.coverFile = {
                fileName: coverFile.name,
                dataUrl: await BaseApi.fileToDataUrl(coverFile),
                mime: coverFile.type || undefined,
            };
        }

        return payload;
    }

    private static fileToDataUrl(file: File): Promise<string> {
        return new Promise((resolve, reject) => {
            const reader = new FileReader();
            reader.onload = () => resolve(String(reader.result || ''));
            reader.onerror = () => reject(new Error('读取文件失败'));
            reader.readAsDataURL(file);
        });
    }

    public async createSubProject(subProject: any): Promise<string> {
        return await AuthFetchUtil.post<string>('/api/project/createSubProject', subProject);
    }

    public async getSubProjectList(id: string): Promise<{ id: string, name: string }[]> {
        const data = await AuthFetchUtil.get<{ id: string, name: string }[]>(`/api/project/getSubProjectList/${id}`);
        return data || [];
    }

    public async deleteSubProject(pid: string, subId: string): Promise<boolean> {
        return await AuthFetchUtil.post<boolean>(`/api/project/deleteSubProject/${pid}/${subId}`, {});
    }

}

const baseApi = new BaseApi();
export default baseApi;
