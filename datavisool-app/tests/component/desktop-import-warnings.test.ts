/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {describe, expect, it, vi} from 'vitest';
import {globalModal} from '@datavisool/ui';
import {importProjectFromDesktopZip} from '@/designer/operate-provider/hot-key/DesktopImportProject.ts';

describe('desktop import warnings', () => {
    it('waits for the user to acknowledge missing references before reloading', async () => {
        let acknowledge!: () => void;
        const reloadPage = vi.fn();
        const showWarnings = vi.fn(() => new Promise<void>(resolve => { acknowledge = resolve; }));
        const result = importProjectFromDesktopZip('/tmp/project.zip', '1', {
            requestSecretPassphrase: async () => '',
            postImportRequest: async () => ({warnings: ['项目 1：公共 API：layerManager']}),
            showWarnings, reloadPage,
        });
        await vi.waitFor(() => expect(showWarnings).toHaveBeenCalledOnce());
        expect(reloadPage).not.toHaveBeenCalled();
        acknowledge();
        await expect(result).resolves.toBe(true);
        expect(reloadPage).toHaveBeenCalledOnce();
    });

    it('shows a non-dismissible confirmation containing the missing reference', async () => {
        let options: any;
        const confirm = vi.spyOn(globalModal.modalApi, 'confirm').mockImplementation(config => { options = config; return 'test'; });
        const reloadPage = vi.fn();
        const result = importProjectFromDesktopZip('project.zip', '1', {
            requestSecretPassphrase: async () => '', postImportRequest: async () => ({warnings: ['项目 1：公共 API：x']}), reloadPage,
        });
        await vi.waitFor(() => expect(confirm).toHaveBeenCalledOnce());
        expect(options.showCancel).toBe(false);
        expect(options.maskClosable).toBe(false);
        expect(options.content.props.children).toContain('项目 1：公共 API：x');
        expect(reloadPage).not.toHaveBeenCalled();
        options.onOk();
        await expect(result).resolves.toBe(true);
        expect(reloadPage).toHaveBeenCalledOnce();
        confirm.mockRestore();
    });

    it.each([true, {warnings: []}])('accepts successful imports without warnings', async response => {
        const reloadPage = vi.fn();
        await expect(importProjectFromDesktopZip('project.zip', '1', {
            requestSecretPassphrase: async () => '', postImportRequest: async () => response,
            showSuccess: vi.fn(), reloadPage,
        })).resolves.toBe(true);
        expect(reloadPage).toHaveBeenCalledOnce();
    });

    it.each([null, false])('does not reload after unsuccessful imports', async response => {
        const reloadPage = vi.fn();
        await expect(importProjectFromDesktopZip('project.zip', '1', {
            requestSecretPassphrase: async () => '', postImportRequest: async () => response, reloadPage,
        })).resolves.toBe(false);
        expect(reloadPage).not.toHaveBeenCalled();
    });
});
