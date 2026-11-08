/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import AbstractDesignerController from "@/framework/core/AbstractDesignerController";
import {ComponentUtil} from "@datavisool/sdk/react";
import {ThreeModelComponent, ThreeModelComponentProps, ThreeModelComponentRef} from "@/comps/builtin/3d-model/ThreeModelComponent";
import {UpdateOptions} from "@/framework/core/AbstractController";
import {ObjectUtil} from "@datavisool/sdk/utils";
import BPExecutor from "@/designer/blueprint/core/BPExecutor.ts";

export class ThreeModelController extends AbstractDesignerController<ThreeModelComponentRef, ThreeModelComponentProps> {
    
    public async create(container: HTMLElement, config: ThreeModelComponentProps, bpExecutor: BPExecutor, index?: number): Promise<void> {
        await super.create(container, config, bpExecutor, index);
        // 使用 ComponentUtil 创建并渲染 React 组件
        this.instance = await ComponentUtil.createAndRender<ThreeModelComponentRef>(container, ThreeModelComponent, config);
    }

    destroy(): void {
        this.instance = null;
        this.config = null;
        super.destroy();
    }

    getConfig(): ThreeModelComponentProps | null {
        return this.config;
    }

    update(config: ThreeModelComponentProps, upOp?: UpdateOptions | undefined): void {
        // 合并配置
        this.config = ObjectUtil.merge(this.config, config);
        upOp = upOp || {reRender: true};
        if (upOp.reRender) {
            // 更新 React 组件的状态
            // 使用 updateConfig 方法更新组件配置
            this.instance?.updateConfig(this.config!);
        }
    }

    // 实现注册事件，例如点击事件
    registerEvent() {
        if (this.instance) {
            const nodeId = this.config?.base?.id ?? '';
            const executor = this.bpExecutor;
            // 绑定点击事件，触发蓝图中的事件
            this.instance.setEventHandler({
                click: () => executor?.triggerComponentEvent(nodeId!, "click", this.config, this.index)
            });
        }
    }

    setVisible(visible: boolean) {
        // 控制组件显隐
        this.instance?.changeVisible(visible);
    }
}
