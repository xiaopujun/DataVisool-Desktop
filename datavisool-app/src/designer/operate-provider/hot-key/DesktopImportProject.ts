/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import AuthFetchUtil from "@/util/AuthFetchUtil.ts";
import {globalModal} from "@datavisool/ui";
import {createElement} from "react";
import {Input} from "@datavisool/ui";
import {globalMessage} from "@datavisool/ui";

type ConfirmApi = {
    confirm: (options: any) => unknown;
};

type DesktopImportResult = boolean | {warnings: string[]};

interface DesktopImportProjectDeps {
    modalApi?: ConfirmApi | null;
    requestSecretPassphrase?: () => Promise<string | null>;
    postImportRequest?: (url: string, body: any, config?: any) => Promise<DesktopImportResult | null>;
    showWarnings?: (warnings: string[]) => Promise<void>;
    showSuccess?: (options: { content: string, duration: number }) => Promise<unknown> | unknown;
    reloadPage?: () => void;
}

export const requestImportSecretPassphrase = async (modalApi: ConfirmApi | null | undefined = globalModal.modalApi) => {
    if (!modalApi)
        return '';

    return await new Promise<string | null>((resolve) => {
        let settled = false;
        let secretPassphrase = '';
        const finish = (result: string | null) => {
            if (settled)
                return;
            settled = true;
            resolve(result);
        };

        modalApi.confirm({
            title: '导入项目',
            okText: '开始导入',
            cancelText: '取消',
            icon: null,
            content: createElement(() => {
                return createElement('div', {style: {marginTop: '16px'}}, [
                    createElement('label', {
                        key: 'secretPassphrase',
                        style: {display: 'grid', gap: '8px', marginBottom: '12px'}
                    }, [
                        createElement('span', {key: 'label'}, '导入口令（选填）'),
                        createElement(Input, {
                        key: 'input',
                        type: 'password',
                        placeholder: '整包加密或历史字段加密包请填写导入口令',
                        autoComplete: 'new-password',
                        onChange: (value: string) => {
                            secretPassphrase = value;
                        },
                    })]),
                    createElement('div', {
                        key: 'hint',
                        style: {color: 'rgba(255,255,255,0.65)', lineHeight: '22px'}
                    }, '明文包可留空；整包加密或历史字段加密包需要填写正确的导入口令。')
                ]);
            }),
            onOk: async () => {
                finish(secretPassphrase.trim());
                return true;
            },
            onCancel: () => finish(null),
            afterClose: () => finish(null),
        });
    });
}

export const importProjectFromDesktopZip = async (
    zipPath: string,
    projectId: string,
    deps: DesktopImportProjectDeps = {}
) => {
    const requestSecretPassphrase = deps.requestSecretPassphrase
        ?? (() => requestImportSecretPassphrase(deps.modalApi));
    const secretPassphrase = await requestSecretPassphrase();
    if (secretPassphrase === null)
        return false;

    const postImportRequest = deps.postImportRequest
        ?? ((url: string, body: any, config?: any) => AuthFetchUtil.post<DesktopImportResult>(url, body, config));
    const response = await postImportRequest(
        '/api/import/importProjectFromPath',
        {projectId, path: zipPath, secretPassphrase},
        {loadingText: '正在导入项目数据...'}
    );
    if (response === null || response === false)
        return false;

    const warnings = typeof response === 'object' && Array.isArray(response.warnings) ? response.warnings : [];
    if (warnings.length > 0) {
        const showWarnings = deps.showWarnings ?? showImportWarnings;
        await showWarnings(warnings);
        (deps.reloadPage ?? (() => location.reload()))();
        return true;
    }

    const showSuccess = deps.showSuccess
        ?? ((options: { content: string, duration: number }) => globalMessage.messageApi?.success(options.content, options.duration));
    await showSuccess?.({content: '项目数据导入成功, 将重新加载页面！', duration: 1});
    (deps.reloadPage ?? (() => location.reload()))();
    return true;
}

/** 导入成功后等待用户阅读缺失引用，再重新加载项目。 */
const showImportWarnings = async (warnings: string[]) => {
    const message = `项目已导入，但以下数据源引用无法恢复，请重新选择：\n${warnings.join('\n')}`;
    const modal = globalModal.modalApi;
    if (!modal) {
        window.alert(message);
        return;
    }
    await new Promise<void>((resolve) => {
        modal.confirm({
            title: '数据源需要重新选择',
            content: createElement('div', {style: {whiteSpace: 'pre-wrap', maxHeight: '50vh', overflow: 'auto'}}, message),
            okText: '知道了',
            showCancel: false,
            maskClosable: false,
            closable: false,
            keyboard: false,
            onOk: () => resolve(),
        });
    });
};
