/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import AbstractDesignerController from "@/framework/core/AbstractDesignerController";
import BaseImageComponent, {type BaseImageComponentProps, type BaseImageComponentRef} from "./BaseImageComponent";
import {ComponentUtil} from "@datavisool/sdk/react";
import {UpdateOptions} from "@/framework/core/AbstractController";
import {ObjectUtil} from "@datavisool/sdk/utils";
import {IThemeItem} from "@datavisool/sdk/types";
import BPExecutor from "@/designer/blueprint/core/BPExecutor.ts";

export type {BaseImageComponentProps};

export default class BaseImageController extends AbstractDesignerController<BaseImageComponentRef, BaseImageComponentProps> {

    public async create(container: HTMLElement, config: BaseImageComponentProps, bpExecutor: BPExecutor, index?: number): Promise<void> {
        await super.create(container, config, bpExecutor, index);
        this.instance = await ComponentUtil.createAndRender<BaseImageComponentRef>(container, BaseImageComponent, config);
    }

    destroy(): void {
        this.instance = null;
        this.config = null;
        super.destroy();
    }

    getConfig(): BaseImageComponentProps | null {
        return this.config;
    }

    update(config: BaseImageComponentProps, upOp?: UpdateOptions | undefined): void {
        this.config = ObjectUtil.merge(this.config, config);
        upOp = upOp || {reRender: true};
        if (upOp.reRender) {
            this.instance?.updateConfig(this.config!);
        }
    }

    changeData(data: any) {
        if (!this.config)
            this.config = {};
        if (!this.config.data)
            this.config.data = {sourceType: 'static'};
        this.config.data.value = data;
        this.instance?.updateConfig(this.config);
    }

    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    updateTheme(newTheme: IThemeItem): void {

    }

    registerEvent() {
        if (this.instance) {
            const nodeId = this.config?.base?.id ?? '';
            const executor = this.bpExecutor;
            this.instance.eventHandlerMap = {
                click: () => executor?.triggerComponentEvent(nodeId!, "click", this.config, this.index)
            }
        }
    }

    setVisible(visible: boolean) {
        this.instance?.changeVisible(visible);
    }
}
