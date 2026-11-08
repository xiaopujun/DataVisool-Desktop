/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
import {act, cleanup, fireEvent, render, screen, waitFor} from '@testing-library/react';
import {globalMessage} from '@datavisool/ui';
import {open} from '@tauri-apps/plugin-dialog';
import FetchUtil from '@/util/FetchUtil.ts';
import Setting from '@/pages/home/setting/Setting.tsx';

vi.mock('@tauri-apps/plugin-dialog', () => ({open: vi.fn()}));
vi.mock('@/util/FetchUtil.ts', () => ({default: {get: vi.fn(), post: vi.fn()}}));

const oldPath = '/private/tmp/settings-old';
const newPath = '/private/tmp/settings-new';

async function ready() {
    const choose = await screen.findByRole('button', {name: '选择目录'});
    await waitFor(() => expect(choose).toBeEnabled());
    return choose;
}

describe('settings configuration errors', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        Object.defineProperty(window, '__TAURI__', {value: {}, configurable: true});
        window.datavisoolI18n?.setLocale('zh-CN');
        vi.mocked(FetchUtil.get).mockResolvedValue({resourceStorageDir: oldPath});
        vi.spyOn(globalMessage, 'success').mockImplementation(() => {});
        vi.spyOn(globalMessage, 'error').mockImplementation(() => {});
    });

    afterEach(() => {
        cleanup();
        vi.restoreAllMocks();
        delete (window as any).__TAURI__;
    });

    it('keeps a failed read distinct from the default and retries', async () => {
        vi.mocked(FetchUtil.get).mockResolvedValueOnce(null).mockResolvedValueOnce({resourceStorageDir: oldPath});
        render(<Setting/>);
        expect(await screen.findByRole('alert')).toHaveTextContent('读取配置失败');
        expect(screen.getByPlaceholderText('读取配置失败')).toHaveValue('');
        expect(screen.queryByPlaceholderText('默认使用 AppData（留空）')).not.toBeInTheDocument();
        expect(screen.getByRole('button', {name: '选择目录'})).toBeDisabled();
        fireEvent.click(screen.getByRole('button', {name: '重试'}));
        await screen.findByDisplayValue(oldPath);
        expect(screen.queryByRole('alert')).not.toBeInTheDocument();
        expect(await ready()).toBeEnabled();
    });

    it('reports a rejected read and allows a successful retry', async () => {
        vi.mocked(FetchUtil.get).mockRejectedValueOnce(new Error('read failed'))
            .mockResolvedValueOnce({resourceStorageDir: oldPath});
        render(<Setting/>);
        expect(await screen.findByRole('alert')).toHaveTextContent('读取配置失败');
        fireEvent.click(screen.getByRole('button', {name: '重试'}));
        await screen.findByDisplayValue(oldPath);
        expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    });

    it('ignores an older read after switching between sections', async () => {
        let finishFirst!: (value: {resourceStorageDir: string}) => void;
        vi.mocked(FetchUtil.get).mockReturnValueOnce(new Promise(resolve => { finishFirst = resolve; }))
            .mockResolvedValueOnce({resourceStorageDir: newPath});
        render(<Setting/>);
        await waitFor(() => expect(FetchUtil.get).toHaveBeenCalledTimes(1));
        fireEvent.click(screen.getByRole('menuitem', {name: '关于'}));
        fireEvent.click(screen.getByRole('menuitem', {name: '通用'}));
        await screen.findByDisplayValue(newPath);
        await act(async () => finishFirst({resourceStorageDir: oldPath}));
        expect(screen.getByDisplayValue(newPath)).toBeInTheDocument();
    });

    it('keeps the previously saved directory when an update returns failure', async () => {
        vi.mocked(open).mockResolvedValue(newPath);
        vi.mocked(FetchUtil.post).mockResolvedValue(null);
        render(<Setting/>);
        fireEvent.click(await ready());
        await waitFor(() => expect(globalMessage.error).toHaveBeenCalledWith('保存失败'));
        expect(globalMessage.success).not.toHaveBeenCalled();
        expect(await screen.findByDisplayValue(oldPath)).toBeInTheDocument();
        expect(screen.queryByDisplayValue(newPath)).not.toBeInTheDocument();
    });

    it('does not clear the saved directory when restoring the default fails', async () => {
        vi.mocked(FetchUtil.post).mockResolvedValue(null);
        render(<Setting/>);
        await ready();
        fireEvent.click(screen.getByRole('button', {name: '清空并恢复默认'}));
        await waitFor(() => expect(globalMessage.error).toHaveBeenCalledWith('保存失败'));
        expect(await screen.findByDisplayValue(oldPath)).toBeInTheDocument();
        expect(globalMessage.success).not.toHaveBeenCalled();
    });

    it('keeps the saved path when the save request rejects', async () => {
        vi.mocked(open).mockResolvedValue(newPath);
        vi.mocked(FetchUtil.post).mockRejectedValue(new Error('save failed'));
        render(<Setting/>);
        fireEvent.click(await ready());
        await waitFor(() => expect(globalMessage.error).toHaveBeenCalledWith('保存失败'));
        expect(await screen.findByDisplayValue(oldPath)).toBeInTheDocument();
        expect(globalMessage.success).not.toHaveBeenCalled();
    });

    it('saves and restores the default when the backend confirms success', async () => {
        let stored: string | null = oldPath;
        vi.mocked(FetchUtil.get).mockImplementation(async () => ({resourceStorageDir: stored}));
        vi.mocked(FetchUtil.post).mockImplementation(async (_url, body: any) => {
            stored = body.resourceStorageDir;
            return {};
        });
        vi.mocked(open).mockResolvedValue(newPath);
        render(<Setting/>);
        fireEvent.click(await ready());
        await screen.findByDisplayValue(newPath);
        await ready();
        fireEvent.click(screen.getByRole('button', {name: '清空并恢复默认'}));
        await waitFor(() => expect(screen.queryByRole('button', {name: '清空并恢复默认'})).not.toBeInTheDocument());
        expect(screen.getByPlaceholderText('默认使用 AppData（留空）')).toHaveValue('');
        expect(stored).toBeNull();
    });

    it('leaves the setting unchanged when the directory picker is cancelled', async () => {
        vi.mocked(open).mockResolvedValue(null);
        render(<Setting/>);
        fireEvent.click(await ready());
        await waitFor(() => expect(open).toHaveBeenCalled());
        expect(FetchUtil.post).not.toHaveBeenCalled();
        expect(screen.getByDisplayValue(oldPath)).toBeInTheDocument();
    });

    it('continues to switch languages and sections', async () => {
        render(<Setting/>);
        await ready();
        fireEvent.click(screen.getByRole('combobox'));
        fireEvent.click(screen.getByRole('option', {name: 'English'}));
        await waitFor(() => expect(screen.getByRole('combobox')).toHaveTextContent('English'));
        fireEvent.click(screen.getByRole('menuitem', {name: 'About'}));
        expect(await screen.findByText(/^(版本|Version)\s*[:：]/i)).toBeInTheDocument();
        expect(screen.queryByText(/发布日期|release date|publish date/i)).not.toBeInTheDocument();
    });
});
