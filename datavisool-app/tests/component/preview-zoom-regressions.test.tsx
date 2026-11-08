/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {act, cleanup, fireEvent, render, screen} from '@testing-library/react';
import {afterEach, beforeEach, describe, expect, test, vi} from 'vitest';

const mocks = vi.hoisted(() => ({
    doSave: vi.fn(),
    managerStore: {
        currentManager: {id: 'main-project', name: '主项目', scene: 'default'},
        currentFinalManager: {id: 'main-project'},
    },
    footerStore: {saveState: 'saved', latestSaveTime: ''},
    navigate: vi.fn(),
    windowOpen: vi.fn(),
    previewWindow: {closed: false, close: vi.fn(), location: {replace: vi.fn()}},
    webviewWindows: [] as Array<{label: string; url: string}>,
    viewportStore: {scale: 1},
    setScale: vi.fn(),
    zoomBy: vi.fn(),
    fitArtboard: vi.fn(),
}));

vi.mock('@/designer/operate-provider/hot-key/HotKeyImpl', () => ({
    createLoopContainer: vi.fn(),
    doSave: mocks.doSave,
    exportProject: vi.fn(),
    importProject: vi.fn(),
    returnPrevProject: vi.fn(),
    throttleSave: vi.fn(),
}));
vi.mock('@/designer/manager/ManagerStore.ts', () => ({default: mocks.managerStore}));
vi.mock('@/designer/footer/FooterStore.ts', () => ({default: mocks.footerStore}));
vi.mock('@/designer/operate-provider/EventOperateStore', () => ({default: {setTargetIds: vi.fn()}}));
vi.mock('@/designer/header/DesignerHeaderStore.ts', () => ({
    default: {currentKey: null, setCurrentKey: vi.fn()},
}));
vi.mock('react-router-dom', () => ({useNavigate: () => mocks.navigate}));
vi.mock('@tauri-apps/api/webviewWindow', () => ({
    WebviewWindow: class {
        constructor(label: string, options: {url: string}) {
            mocks.webviewWindows.push({label, url: options.url});
        }
    },
}));
vi.mock('@/util/WebviewUserAgentUtil.ts', () => ({withDesktopWebviewUserAgent: (options: unknown) => options}));
vi.mock('@icon-park/react', () => ({
    AfferentFour: () => null,
    DashboardOne: () => null,
    Down: () => null,
    EfferentFour: () => null,
    FullScreen: () => null,
    HardDiskOne: () => null,
    Keyboard: () => null,
    MemoryOne: () => null,
    MoreOne: () => null,
    Pic: () => null,
    PreviewOpen: () => null,
    Repair: () => null,
    Return: () => null,
    Send: () => null,
    ZoomIn: () => null,
    ZoomOut: () => null,
}));
vi.mock('@/designer/header/items/project/ProjectHdItemImpl.tsx', () => ({default: () => null}));
vi.mock('@/designer/header/items/canvas/CanvasHdConfigImpl.tsx', () => ({default: () => null}));
vi.mock('@/designer/header/items/theme/ThemeHdItemImpl.tsx', () => ({default: () => null}));
vi.mock('@/designer/header/items/blue-print/BluePrintHdImpl.tsx', () => ({default: () => null}));
vi.mock('@/designer/header/items/release/ReleaseHdConfigImpl.tsx', () => ({default: () => null}));
vi.mock('@/designer/header/items/global-filter/GlobalFilterConfigImpl.tsx', () => ({default: () => null}));
vi.mock('@/designer/workspace/toolbar/WorkspacePopover.tsx', () => ({default: () => null}));
vi.mock('@/designer/workspace/toolbar/WorkspaceToolbar.tsx', () => ({default: () => null}));
vi.mock('@/designer/workspace/viewport/ViewportStore.ts', () => ({
    default: Object.assign(mocks.viewportStore, {
        setScale: mocks.setScale,
        zoomBy: mocks.zoomBy,
        fitArtboard: mocks.fitArtboard,
    }),
}));

import Header from '@/designer/header/DesignerHeader.tsx';
import WorkspaceZoomControls from '@/designer/workspace/toolbar/WorkspaceZoomControls.tsx';

describe('preview and zoom input regression fixes', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        mocks.managerStore.currentManager = {id: 'main-project', name: '主项目', scene: 'default'};
        mocks.managerStore.currentFinalManager = {id: 'main-project'};
        mocks.footerStore.saveState = 'saved';
        mocks.footerStore.latestSaveTime = '';
        mocks.viewportStore.scale = 1;
        mocks.webviewWindows.length = 0;
        mocks.previewWindow.closed = false;
        mocks.windowOpen.mockReturnValue(mocks.previewWindow);
        vi.spyOn(window, 'open').mockImplementation(mocks.windowOpen);
        delete (window as Window & {__TAURI__?: unknown}).__TAURI__;
        delete (window as Window & {__TAURI_INTERNALS__?: unknown}).__TAURI_INTERNALS__;
    });

    afterEach(() => {
        cleanup();
        vi.restoreAllMocks();
    });

    test.each(['browser', 'tauri'] as const)(
        'opens the clicked project in the %s preview after delayed save despite a project switch',
        async (platform) => {
            if (platform === 'tauri')
                Object.defineProperty(window, '__TAURI__', {configurable: true, value: true});

            let resolveSave!: (saved: boolean) => void;
            mocks.doSave.mockReturnValue(new Promise<boolean>((resolve) => { resolveSave = resolve; }));
            render(<Header/>);

            fireEvent.click(screen.getByRole('button', {name: '预览项目'}));
            expect(mocks.doSave).toHaveBeenCalledOnce();
            if (platform === 'browser') {
                expect(mocks.windowOpen).toHaveBeenCalledWith('about:blank', '_blank');
                expect(mocks.windowOpen.mock.invocationCallOrder[0]).toBeLessThan(mocks.doSave.mock.invocationCallOrder[0]);
            } else {
                expect(mocks.windowOpen).not.toHaveBeenCalled();
            }
            expect(mocks.previewWindow.location.replace).not.toHaveBeenCalled();
            expect(mocks.webviewWindows).toHaveLength(0);
            mocks.managerStore.currentFinalManager = {id: 'child-project'};

            await act(async () => {
                resolveSave(true);
                await Promise.resolve();
            });

            if (platform === 'browser') {
                expect(mocks.windowOpen).toHaveBeenCalledOnce();
                expect(mocks.previewWindow.location.replace).toHaveBeenCalledWith('/preview?id=main-project');
                expect(mocks.previewWindow.close).not.toHaveBeenCalled();
                expect(mocks.webviewWindows).toHaveLength(0);
            } else {
                expect(mocks.windowOpen).not.toHaveBeenCalled();
                expect(mocks.webviewWindows).toEqual([{
                    label: 'preview-main-project',
                    url: '/preview?id=main-project',
                }]);
            }
        },
    );

    test.each(['browser', 'tauri'] as const)('does not open %s preview when saving fails', async (platform) => {
        if (platform === 'tauri')
            Object.defineProperty(window, '__TAURI__', {configurable: true, value: true});
        mocks.doSave.mockResolvedValue(false);
        render(<Header/>);

        fireEvent.click(screen.getByRole('button', {name: '预览项目'}));
        await act(async () => { await Promise.resolve(); });
        expect(mocks.doSave).toHaveBeenCalledOnce();
        if (platform === 'browser') {
            expect(mocks.windowOpen).toHaveBeenCalledWith('about:blank', '_blank');
            expect(mocks.previewWindow.close).toHaveBeenCalledOnce();
        } else {
            expect(mocks.windowOpen).not.toHaveBeenCalled();
        }
        expect(mocks.previewWindow.location.replace).not.toHaveBeenCalled();
        expect(mocks.webviewWindows).toHaveLength(0);
    });

    test('does not reopen a browser preview closed while saving', async () => {
        let resolveSave!: (saved: boolean) => void;
        mocks.doSave.mockReturnValue(new Promise<boolean>((resolve) => { resolveSave = resolve; }));
        render(<Header/>);

        fireEvent.click(screen.getByRole('button', {name: '预览项目'}));
        mocks.previewWindow.closed = true;
        await act(async () => { resolveSave(true); });

        expect(mocks.windowOpen).toHaveBeenCalledOnce();
        expect(mocks.previewWindow.location.replace).not.toHaveBeenCalled();
        expect(mocks.webviewWindows).toHaveLength(0);
    });

    test('does not retry opening a blocked browser preview after saving', async () => {
        mocks.windowOpen.mockReturnValue(null);
        mocks.doSave.mockResolvedValue(true);
        render(<Header/>);

        fireEvent.click(screen.getByRole('button', {name: '预览项目'}));
        await act(async () => { await Promise.resolve(); });

        expect(mocks.windowOpen).toHaveBeenCalledOnce();
        expect(mocks.previewWindow.location.replace).not.toHaveBeenCalled();
        expect(mocks.webviewWindows).toHaveLength(0);
    });

    test('Escape cancels the zoom draft to the latest viewport scale and leaves later edits committable', () => {
        const {rerender} = render(<WorkspaceZoomControls/>);
        const input = screen.getByRole('textbox', {name: '缩放百分比'});

        act(() => input.focus());
        fireEvent.change(input, {target: {value: '175'}});
        mocks.viewportStore.scale = 1.4;
        rerender(<WorkspaceZoomControls/>);
        fireEvent.keyDown(input, {key: 'Escape'});
        expect(mocks.setScale).not.toHaveBeenCalled();
        expect(input).toHaveValue('140%');

        act(() => input.focus());
        fireEvent.change(input, {target: {value: '125'}});
        fireEvent.blur(input);
        expect(mocks.setScale).toHaveBeenCalledOnce();
        expect(mocks.setScale).toHaveBeenCalledWith(1.25);
    });

    test('Enter and ordinary blur commit the zoom draft', () => {
        render(<WorkspaceZoomControls/>);
        const input = screen.getByRole('textbox', {name: '缩放百分比'});

        act(() => input.focus());
        fireEvent.change(input, {target: {value: '150'}});
        fireEvent.keyDown(input, {key: 'Enter'});
        expect(mocks.setScale).toHaveBeenLastCalledWith(1.5);

        act(() => input.focus());
        fireEvent.change(input, {target: {value: '80'}});
        fireEvent.blur(input);
        expect(mocks.setScale).toHaveBeenLastCalledWith(0.8);
        expect(mocks.setScale).toHaveBeenCalledTimes(2);
    });
});
