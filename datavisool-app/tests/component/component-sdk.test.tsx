/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, describe, expect, it, vi} from 'vitest';
import {act, render, screen} from '@testing-library/react';
import {Component, type ReactNode} from 'react';
import {AbstractDesignerController, AbstractDesignerDefinition, BPExecutor} from '@datavisool/sdk/core';
import {ComponentUtil, setRenderErrorFallback} from '@datavisool/sdk/react';
import {BaseInfo, registerHostConfigPanels} from '@datavisool/sdk/host';
import * as BasicUI from '@datavisool/ui/basic';
import * as SchemaUI from '@datavisool/ui/schema';
import * as CodeEditorUI from '@datavisool/ui/code-editor';
import * as FeedbackUI from '@datavisool/ui/feedback';
import {componentSdk, exportComponentRuntime} from '@/global/ComponentRuntime';
import {DesignerMode} from '@datavisool/sdk/types';
import type {ComponentBaseProps, IFilterConfigType} from '@datavisool/sdk/types';

class Controller extends AbstractDesignerController<unknown, ComponentBaseProps> {
    update(config: ComponentBaseProps) { this.config = config; }
    getConfig() { return this.config; }
}

const executor: BPExecutor = {triggerComponentEvent: vi.fn()};
afterEach(() => vi.restoreAllMocks());

describe('shared SDK lifecycle and host runtime', () => {
    it('initializes, filters, hides and cleans up a component through the shared contract', async () => {
        const controller = new Controller();
        const element = document.createElement('div');
        const filter: IFilterConfigType = {enable: true, blur: 2, brightness: 1, contrast: 1, opacity: 1, saturate: 1, hueRotate: 0};
        const config: ComponentBaseProps = {filter};
        await controller.create(element, config, executor, 2);
        controller.updateFilter(filter);
        expect(element.style.filter).toContain('blur(2px)');
        controller.setVisible(false);
        expect(element.style.visibility).toBe('hidden');
        expect(controller.index).toBe(2);
        const destroy = vi.fn();
        controller.componentDataUpdater = {run: vi.fn(), destroy};
        controller.destroy();
        expect(destroy).toHaveBeenCalledOnce();
        expect(controller.container).toBeNull();
        expect(controller.getConfig()).toBeNull();
        expect(controller.bpExecutor).toBeNull();
    });

    it('exposes the host constructors through the SDK', () => {
        exportComponentRuntime();
        expect(Object.keys(window.DataVisoolSDK)).toEqual(expect.arrayContaining([
            'AbstractController', 'AbstractDefinition', 'AbstractDesignerController', 'AbstractDesignerDefinition',
            'BPExecutor', 'ComponentUtil', 'ObjectUtil', 'BaseInfo',
        ]));
        expect(window.DataVisoolSDK.AbstractDesignerController).toBe(componentSdk.AbstractDesignerController);
        expect(window.DataVisoolSDK.AbstractDesignerDefinition).toBe(AbstractDesignerDefinition);
        expect(window.DataVisoolSDK.ComponentUtil).toBe(ComponentUtil);
        expect(window.ReactDOM.createRoot).toBeTypeOf('function');
        expect(window.DataVisoolTypes.DesignerMode.EDIT).toBe('0');
        expect(window.DataVisoolTypes.DesignerMode).toBe(DesignerMode);
        expect(componentSdk.AbstractDesignerController.prototype).toBeInstanceOf(AbstractDesignerController);
    });

    it('delegates built-in config panels to the registered host component', () => {
        const Panel = () => <div>Host config</div>;
        registerHostConfigPanels({BaseInfo: Panel, AnimationConfig: Panel, ThemeConfig: Panel, DataConfig: Panel, FilterConfig: Panel});
        render(<BaseInfo controller={new Controller()}/>);
        expect(screen.getByText('Host config')).toBeInTheDocument();
    });

    it('provides every UI subentry export through the shared plugin global', () => {
        exportComponentRuntime();
        for (const entry of [BasicUI, SchemaUI, CodeEditorUI, FeedbackUI]) {
            for (const [name, value] of Object.entries(entry)) {
                expect((window.DataVisoolUI as Record<string, unknown>)[name]).toBe(value);
            }
        }
    });

    it('renders and unmounts a React component without importing the application', async () => {
        class Sample extends Component {
            render(): ReactNode { return <span>SDK component</span>; }
        }
        const container = document.createElement('div');
        document.body.append(container);
        let instance: Promise<Sample | null>;
        await act(async () => {
            instance = ComponentUtil.createAndRender<Sample>(container, Sample);
        });
        expect(await instance!).toBeInstanceOf(Sample);
        expect(container.textContent).toBe('SDK component');
        act(() => ComponentUtil.unmount(container));
        expect(container.childNodes.length).toBe(0);
        container.remove();
    });

    it('catches render errors with the default fallback and the application fallback', async () => {
        vi.spyOn(console, 'error').mockImplementation(() => {});
        class Broken extends Component {
            render(): ReactNode { throw new Error('render failed'); }
        }
        const container = document.createElement('div');
        document.body.append(container);
        const renderBroken = async () => {
            let result: Promise<Broken | null>;
            await act(async () => { result = ComponentUtil.createAndRender<Broken>(container, Broken); });
            expect(await result!).toBeNull();
        };
        try {
            setRenderErrorFallback();
            await renderBroken();
            expect(container.querySelector('[role="alert"]')?.textContent).toBe('组件渲染失败');
            act(() => ComponentUtil.unmount(container));

            await import('@/global/configureComponentRendering');
            await renderBroken();
            expect(container.textContent).toContain('FAULT');
            expect(container.textContent).toContain('请检查配置后重试');
        } finally {
            act(() => ComponentUtil.unmount(container));
            setRenderErrorFallback();
            container.remove();
        }
    });
});
