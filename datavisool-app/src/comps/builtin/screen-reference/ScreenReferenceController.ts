/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {UpdateOptions} from "@/framework/core/AbstractController";
import AbstractDesignerController from "@/framework/core/AbstractDesignerController";
import ScreenReferenceComponent, {ScreenReferenceComponentProps, ScreenReferenceComponentRef} from './ScreenReferenceComponent.tsx';
import {ObjectUtil} from "@datavisool/sdk/utils";
import BPExecutor from "@/designer/blueprint/core/BPExecutor.ts";
import {ComponentUtil} from "@datavisool/sdk/react";

export class ScreenReferenceController extends AbstractDesignerController<ScreenReferenceComponentRef, ScreenReferenceComponentProps> {

    async create(container: HTMLElement, config: ScreenReferenceComponentProps, bpExecutor: BPExecutor, index?: number): Promise<void> {
        await super.create(container, config, bpExecutor, index);
        this.instance = await ComponentUtil.createAndRender<ScreenReferenceComponentRef>(container, ScreenReferenceComponent, config);
    }

    destroy(): void {
        this.instance = null;
        this.config = null;
        super.destroy();
    }

    getConfig(): ScreenReferenceComponentProps | null {
        return this.config;
    }

    update(config: ScreenReferenceComponentProps, upOp?: UpdateOptions | undefined): void {
        this.config = ObjectUtil.merge(this.config, config);
        upOp = upOp || {reRender: true};
        if (upOp.reRender)
            this.instance?.updateConfig(this.config!);
    }

    registerEvent() {
        const nodeId = this.config?.base?.id ?? '';
        const executor = this.bpExecutor;
        this.instance?.setEventHandler({
            click: () => executor?.triggerComponentEvent(nodeId!, "click", this.config, this.index),
        })
    }


    setVisible(visible: boolean) {
        this.instance?.changeVisible(visible);
    }
}