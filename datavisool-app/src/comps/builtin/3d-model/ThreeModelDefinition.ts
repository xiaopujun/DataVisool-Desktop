/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import { lazy } from 'react';
import AbstractDesignerDefinition from "@/framework/core/AbstractDesignerDefinition";
import { ThreeModelController } from "./ThreeModelController";
import { ThreeModelComponent } from "./ThreeModelComponent";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuToConfigMappingType} from "@datavisool/sdk/core";
import threeModel from './three-model.webp';

const ThreeModelStyleConfig = lazy(() => import("./ThreeModelStyleConfig"));

export default class ThreeModelDefinition extends AbstractDesignerDefinition<ThreeModelController, any> {
    getBaseInfo(): BaseInfoType {
        return {
            compName: "3D模型",
            compKey: "ThreeModel",
            categorize: "media",
            subCategorize: "3d",
            width: 400,
            height: 300,
        };
    }
    getChartImg(): string | null {
        return threeModel;
    }

    getController(): new () => ThreeModelController {
        return ThreeModelController;
    }

    getInitConfig(): any {
        return {
            base: {
                id: "",
                name: "3D模型",
                type: "ThreeModel",
            },
            style: {
                source: {
                    type: "online",
                    url: "https://threejs.org/examples/models/gltf/Parrot.glb",
                    format: "glb"
                },
                scene: {
                    background: {
                        type: "transparent",
                        value: "#f0f0f0"
                    },
                    environment: {
                        preset: "city",
                        intensity: 1,
                        blur: 0
                    }
                },
                camera: {
                    fov: 37,
                    position: [
                        0,
                        0,
                        120
                    ],
                    controls: {
                        enable: true,
                        interactive: false,
                        autoRotate: true,
                        autoRotateSpeed: 1,
                        enableDamping: true
                    }
                },
                lights: {
                    ambient: {
                        intensity: 0.5,
                        color: "#ffffff"
                    },
                    directional: {
                        intensity: 1,
                        color: "#ffffff",
                        position: [
                            5,
                            5,
                            5
                        ]
                    }
                },
                display: {
                    mode: "standard",
                    scale: 1,
                    autoCenter: false
                }
            },
        };
    }

    getReactComponent(): any {
        return ThreeModelComponent;
    }

    getMenuList() {
        // 移除默认的数据和主题菜单，因为目前不需要
        return super.getMenuList().filter(item => item.key !== 'data' && item.key !== 'theme');
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        const map = super.getMenuToConfigContentMap();
        // 替换 style 菜单的内容为自定义的 ThreeModelStyleConfig
        map['style'] = ThreeModelStyleConfig;
        return map;
    }
}
