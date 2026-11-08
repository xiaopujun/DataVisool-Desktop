/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
import {runInAction} from 'mobx';

const saveMocks = vi.hoisted(() => ({
    updateProject: vi.fn(),
    showSaveError: vi.fn(),
}));

vi.mock('@/api/engineering/BaseApi.ts', () => ({
    default: {updateProject: saveMocks.updateProject},
}));

vi.mock('@datavisool/ui', () => ({
    globalMessage: {messageApi: {error: saveMocks.showSaveError}},
    globalModal: {},
    CheckBox: () => null,
    Input: () => null,
    TextArea: () => null,
}));
vi.mock('@/designer/operate-provider/EventOperateStore', () => ({default: {}}));
vi.mock('@/designer/operate-provider/undo-redo/HistoryOperator', () => ({historyOperator: {}}));
vi.mock('@/designer/operate-provider/undo-redo/HistoryRecordOperateProxy', () => ({default: {}}));
vi.mock('@/designer/operate-provider/undo-redo/core', () => ({default: {}}));
vi.mock('@/designer/store/RuntimeStore.ts', () => ({default: {}}));
vi.mock('@/designer/left/layer-list/LayerListStore.ts', () => ({default: {}}));
vi.mock('@/util/AuthFetchUtil.ts', () => ({default: {}}));
vi.mock('@/util/CoordinateUtil.ts', () => ({default: {}}));
vi.mock('@/designer/manager/ManagerStore.ts', () => ({default: {currentFinalManager: null}}));
vi.mock('@/designer/header/DesignerHeaderStore.ts', () => ({default: {}}));
vi.mock('@/designer/right/RightStore.ts', () => ({default: {}}));
vi.mock('@/designer/left/DesignerLeftStore.ts', () => ({default: {}}));
vi.mock('@/util/ExportRiskUtil.ts', () => ({default: {}}));
vi.mock('@/util/ExportSecretUtil.ts', () => ({default: {}, ExportSecretMode: {}}));
vi.mock('@/util/HybridCryptoUtil.ts', () => ({HybridCryptoUtil: {}}));
vi.mock('@/designer/left/layer-list/util/LayerUtil.ts', () => ({default: {}}));
vi.mock('@/designer/operate-provider/undo-redo/BPHistoryRecordOperateProxy', () => ({default: {}}));
vi.mock('@/designer/operate-provider/hot-key/DesktopImportProject.ts', () => ({importProjectFromDesktopZip: vi.fn()}));
vi.mock('@/designer/operate-provider/selection/SelectionCommands.ts', () => ({
    canManageSelectedObjects: vi.fn(() => false),
    deleteLayersByIds: vi.fn(),
    resolveSelectedCommandTargetIds: vi.fn(() => []),
}));

import ProjectUtil from '@/util/ProjectUtil.ts';
import ProjectDataCodecUtil from '@/util/ProjectDataCodecUtil.ts';
import {doSave, getSaveData} from '@/designer/operate-provider/hot-key/HotKeyImpl.ts';
import managerStore from '@/designer/manager/ManagerStore.ts';
import footerStore from '@/designer/footer/FooterStore.ts';
import type {IDesignerManager} from "@datavisool/sdk/types";

const createProjectData = (): IDesignerManager => ({
    canvasManager: {canvasConfig: {width: 1366, height: 768}} as any,
    themeManager: [{fontFamily: 'V1 Font'}] as any,
    layerManager: {
        elemConfigs: {
            image: {
                base: {id: 'image', type: 'BaseImage'},
                style: {localUrl: 'asset://localhost/C:/project/static/images/photo.png'},
            },
        },
        layerConfigs: {},
        layerHeader: [],
    } as any,
});

describe('single-artboard V1 persistence regression', () => {
    beforeEach(() => {
        saveMocks.updateProject.mockReset();
        saveMocks.showSaveError.mockReset();
        footerStore.resetSaveState();
        window.DATAVISOOL_ENV = {definitions: {}, remoteScripts: {}} as any;
    });

    afterEach(() => {
        runInAction(() => {
            managerStore.currentFinalManager = null;
        });
        delete (window as any).DATAVISOOL_ENV;
    });

    it('serializes the manager document as the original V1 shape when saving', () => {
        const data = createProjectData();
        const manager = {id: 'project-1', getData: vi.fn(() => data)};
        runInAction(() => {
            managerStore.currentFinalManager = manager as any;
        });

        const saveData = getSaveData();

        expect(saveData?.id).toBe('project-1');
        const stored = JSON.parse(ProjectDataCodecUtil.decompressIfNeeded(saveData!.dataJson));
        expect(stored).toEqual(data);
        expect(stored).not.toHaveProperty('version');
        expect(stored).not.toHaveProperty('artboards');
    });

    it('exports dependencies from one V1 manager and keeps its JSON shape', () => {
        const data = createProjectData();

        const dependency = ProjectUtil.calculateProjectDependencyByProject('project-1', 'V1 Project', data);

        expect(dependency.images).toEqual(['photo.png']);
        expect(dependency.fonts).toEqual(['V1 Font']);
        expect(JSON.parse(dependency.projectJson)).toEqual(data);
        expect(JSON.parse(dependency.projectJson)).not.toHaveProperty('artboards');
    });

    it('returns false and reports an error when the save request rejects', async () => {
        saveMocks.updateProject.mockRejectedValueOnce(new Error('network unavailable'));
        const manager = {id: 'project-1', getData: vi.fn(createProjectData)};
        runInAction(() => {
            managerStore.currentFinalManager = manager as any;
        });
        const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});

        await expect(doSave()).resolves.toBe(false);

        expect(saveMocks.updateProject).toHaveBeenCalledTimes(1);
        expect(saveMocks.showSaveError).toHaveBeenCalledWith('保存失败');
        expect(footerStore.saveState).toBe('failed');
        expect(footerStore.saving).toBe(false);
        consoleError.mockRestore();
    });
});
