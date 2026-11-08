/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {cleanup, fireEvent, render, screen, within} from '@testing-library/react';
import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
import {spy as mobxSpy} from 'mobx';
import type {ICanvasManager} from "@datavisool/sdk/types";
import CanvasManager from '@/designer/header/items/canvas/CanvasManager.ts';
import designerHeaderStore from '@/designer/header/DesignerHeaderStore.ts';
import CanvasHdConfigImpl from '@/designer/header/items/canvas/CanvasHdConfigImpl.tsx';

const initialConfig: ICanvasManager = {
    width: 1280,
    height: 720,
    adaptationType: 'scale',
    rasterize: false,
    dragStep: 2,
    resizeStep: 4,
    boxSelectionMode: 'intersect',
};

const adaptationOptions = [
    {label: '自适应 (Scale)', value: 'scale'},
    {label: '撑满宽度 (Full Width)', value: 'full-x'},
    {label: '撑满高度 (Full Height)', value: 'full-y'},
    {label: '撑满全屏 (Full Screen)', value: 'full-screen'},
    {label: '无自适应 (None)', value: 'none'},
] as const;

describe('canvas settings interaction regressions', () => {
    const previousEnvironment = Object.getOwnPropertyDescriptor(window, 'DATAVISOOL_ENV');
    let canvasManager: CanvasManager;
    let updateArguments: unknown[];
    let stopMobxSpy: () => void;

    beforeEach(() => {
        canvasManager = new CanvasManager();
        canvasManager.init({...initialConfig});
        updateArguments = [];
        stopMobxSpy = mobxSpy((event) => {
            if (event.type === 'action' && event.name === 'updateCanvasConfig' && event.object === canvasManager)
                updateArguments.push(event.arguments?.[0]);
        });
        window.DATAVISOOL_ENV = {currentManager: {canvasManager}} as unknown as Window['DATAVISOOL_ENV'];
        designerHeaderStore.setCurrentKey('canvas');
    });

    afterEach(() => {
        cleanup();
        stopMobxSpy();
        designerHeaderStore.setCurrentKey('');
        if (previousEnvironment)
            Object.defineProperty(window, 'DATAVISOOL_ENV', previousEnvironment);
        else
            delete (window as Partial<Window>).DATAVISOOL_ENV;
        vi.restoreAllMocks();
    });

    const renderDialog = () => render(<CanvasHdConfigImpl/>);

    const numberInput = (label: string) => {
        const field = screen.getByText(label, {exact: true}).closest('.canvas-config-field');
        if (!field)
            throw new Error(`Missing canvas field: ${label}`);
        return within(field).getByRole('spinbutton') as HTMLInputElement;
    };

    const cardSwitch = (label: string) => {
        const card = screen.getByText(label, {exact: true}).closest('.canvas-config-toggle-card');
        if (!card)
            throw new Error(`Missing canvas toggle card: ${label}`);
        return within(card).getByRole('switch');
    };

    const selectAdaptation = (label: string) => {
        fireEvent.click(screen.getByRole('combobox'));
        fireEvent.click(screen.getByRole('option', {name: label}));
    };

    it('keeps size edits as a draft and discards them when canceled', () => {
        renderDialog();
        fireEvent.change(numberInput('宽度'), {target: {value: '1920'}});
        fireEvent.change(numberInput('高度'), {target: {value: '1080'}});

        expect(canvasManager.getData()).toEqual(initialConfig);
        expect(updateArguments).toHaveLength(0);

        fireEvent.click(screen.getByRole('button', {name: '取消'}));
        expect(canvasManager.getData()).toEqual(initialConfig);
        expect(updateArguments).toHaveLength(0);
        expect(designerHeaderStore.currentKey).toBe('');
    });

    it('discards the draft when the dialog close button is used', () => {
        renderDialog();
        fireEvent.change(numberInput('宽度'), {target: {value: '1920'}});

        fireEvent.click(screen.getByRole('button', {name: '关闭'}));

        expect(canvasManager.getData()).toEqual(initialConfig);
        expect(updateArguments).toHaveLength(0);
        expect(designerHeaderStore.currentKey).toBe('');
    });

    it.each(adaptationOptions)('maps the $label option to $value when saved', ({label, value}) => {
        renderDialog();
        selectAdaptation(label);

        expect(canvasManager.getData().adaptationType).toBe('scale');
        fireEvent.click(screen.getByRole('button', {name: '确定'}));

        expect(updateArguments).toEqual([{...initialConfig, adaptationType: value}]);
    });

    it('toggles contain selection in the draft without applying it early', () => {
        renderDialog();
        const containSwitch = cardSwitch('完整框选 (Contain)');

        expect(containSwitch).toHaveAttribute('aria-checked', 'false');
        fireEvent.click(containSwitch);
        expect(containSwitch).toHaveAttribute('aria-checked', 'true');
        expect(canvasManager.getData().boxSelectionMode).toBe('intersect');
        expect(updateArguments).toHaveLength(0);

        fireEvent.click(containSwitch);
        expect(containSwitch).toHaveAttribute('aria-checked', 'false');
        fireEvent.click(screen.getByRole('button', {name: '确定'}));
        expect(updateArguments).toEqual([initialConfig]);
    });

    it('shows rasterize steps conditionally and edits their values through NumberInput', () => {
        renderDialog();
        expect(screen.queryByText('拖拽步长', {exact: true})).not.toBeInTheDocument();
        expect(screen.queryByText('缩放步长', {exact: true})).not.toBeInTheDocument();

        const rasterizeSwitch = cardSwitch('栅格化 (Rasterize)');
        fireEvent.click(rasterizeSwitch);
        expect(rasterizeSwitch).toHaveAttribute('aria-checked', 'true');
        fireEvent.change(numberInput('拖拽步长'), {target: {value: '8'}});
        fireEvent.change(numberInput('缩放步长'), {target: {value: '12'}});
        expect(canvasManager.getData()).toEqual(initialConfig);

        fireEvent.click(rasterizeSwitch);
        expect(screen.queryByText('拖拽步长', {exact: true})).not.toBeInTheDocument();
        fireEvent.click(rasterizeSwitch);
        expect(numberInput('拖拽步长')).toHaveValue(8);
        expect(numberInput('缩放步长')).toHaveValue(12);
        fireEvent.click(screen.getByRole('button', {name: '确定'}));

        expect(updateArguments).toEqual([{
            ...initialConfig,
            rasterize: true,
            dragStep: 8,
            resizeStep: 12,
        }]);
    });

    it('applies the complete latest draft once and closes after save', () => {
        renderDialog();
        fireEvent.change(numberInput('宽度'), {target: {value: '1600'}});
        fireEvent.change(numberInput('高度'), {target: {value: '900'}});
        selectAdaptation('撑满宽度 (Full Width)');
        fireEvent.click(cardSwitch('完整框选 (Contain)'));
        fireEvent.click(cardSwitch('栅格化 (Rasterize)'));
        fireEvent.change(numberInput('拖拽步长'), {target: {value: '5'}});
        fireEvent.change(numberInput('缩放步长'), {target: {value: '10'}});

        expect(updateArguments).toHaveLength(0);
        fireEvent.click(screen.getByRole('button', {name: '确定'}));

        const savedConfig = {
            ...initialConfig,
            width: 1600,
            height: 900,
            adaptationType: 'full-x',
            boxSelectionMode: 'contain',
            rasterize: true,
            dragStep: 5,
            resizeStep: 10,
        };
        expect(updateArguments).toEqual([savedConfig]);
        expect(canvasManager.getData()).toEqual(savedConfig);
        expect(designerHeaderStore.currentKey).toBe('');
    });

    it('treats a missing boxSelectionMode as intersect and can save contain mode', () => {
        canvasManager.canvasConfig = {...initialConfig};
        delete (canvasManager.canvasConfig as Partial<ICanvasManager>).boxSelectionMode;
        renderDialog();

        const containSwitch = cardSwitch('完整框选 (Contain)');
        expect(containSwitch).toHaveAttribute('aria-checked', 'false');
        fireEvent.click(containSwitch);
        fireEvent.click(screen.getByRole('button', {name: '确定'}));

        expect(updateArguments).toEqual([{
            ...initialConfig,
            boxSelectionMode: 'contain',
        }]);
    });
});
