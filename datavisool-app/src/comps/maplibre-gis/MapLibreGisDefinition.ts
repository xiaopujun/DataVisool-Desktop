/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import { lazy } from "react";
import AbstractDesignerDefinition from "@/framework/core/AbstractDesignerDefinition.ts";
import {BaseInfoType} from "@datavisool/sdk/types";
import type {MenuInfo, MenuToConfigMappingType} from "@datavisool/sdk/core";
import {ClazzTemplate} from "@datavisool/sdk/types";
import {EventInfo} from "@datavisool/sdk/types";
import mapLibreGisImg from './maplibre.webp';
import MapLibreGisController from "./MapLibreGisController.ts";
import { MapLibreGisComponentProps } from "./MapLibreGisComponent.tsx";
import {
    Camera,
    Click,
    ColorFilter,
    Connect,
    Earth,
    Layers,
    Light,
    Mountain,
    SettingOne,
    SettingTwo,
    Sun,
    Theme
} from "@icon-park/react";

const MapLibreGisConfig = lazy(() => import('./map-base/MapLibreGisConfig.tsx'));
const MapLibreGisCameraConfig = lazy(() => import('./camera/MapLibreGisCameraConfig.tsx'));
const MapLibreGisInteractionConfig = lazy(() => import('./interaction/MapLibreGisInteractionConfig.tsx'));
const MapLibreGisProjectionConfig = lazy(() => import('./projection/MapLibreGisProjectionConfig.tsx'));
const MapLibreGisSourcesConfig = lazy(() => import('./source/MapLibreGisSourcesConfig.tsx'));
const MapLibreGisLayersConfig = lazy(() => import('./layer/MapLibreGisLayersConfig.tsx'));
const MapLibreGisLightConfig = lazy(() => import('./light/MapLibreGisLightConfig.tsx'));
const MapLibreGisSkyConfig = lazy(() => import('./sky/MapLibreGisSkyConfig.tsx'));
const MapLibreGisTerrainConfig = lazy(() => import('./terrain/MapLibreGisTerrainConfig.tsx'));

/**
 * 组件定义（Designer Definition）
 *
 * 该文件是设计器侧的“注册信息”：
 * - 基础信息（尺寸/分类/名称）
 * - 绑定 controller
 * - 右侧配置面板菜单（getMenuList + getMenuToConfigContentMap）
 * - 组件初始配置（getInitConfig）
 * - 事件列表（getEventList）
 *
 * 注意：
 * - 这里返回的是“配置面板组件”，配置面板内部会通过 controller.getConfig()/controller.update() 读写配置
 * - MapLibre 的实际渲染与运行态更新由 MapLibreGisController + MapLibreGisComponent 完成
 */
export default class MapLibreGisDefinition extends AbstractDesignerDefinition<MapLibreGisController, MapLibreGisComponentProps> {
    getBaseInfo(): BaseInfoType {
        return {
            compName: 'GIS(MapLibre)',
            compKey: 'MapLibreGis',
            categorize: 'map',
            width: 640,
            height: 360,
        };
    }
    getController(): ClazzTemplate<MapLibreGisController> | null {
        return MapLibreGisController;
    }

    getChartImg(): string | null {
        return mapLibreGisImg;
    }

    getMenuList(): Array<MenuInfo> {
        return [
            { icon: SettingOne, name: '基础', key: 'base' },
            { icon: SettingTwo, name: '地图', key: 'map_base' },
            { icon: Connect, name: '数据源', key: 'sources' },
            { icon: Layers, name: '图层', key: 'layers' },
            { icon: Camera, name: '视角', key: 'camera' },
            { icon: Click, name: '交互', key: 'interaction' },
            { icon: Earth, name: '投影', key: 'projection' },
            { icon: Sun, name: '光照', key: 'light' },
            { icon: Light, name: '天空', key: 'sky' },
            { icon: Mountain, name: '地形', key: 'terrain' },
            { icon: ColorFilter, name: '滤镜', key: 'filter' },
            { icon: Theme, name: '主题', key: 'theme' },
        ];
    }

    getMenuToConfigContentMap(): MenuToConfigMappingType {
        /**
         * 继承基类默认的菜单映射，但删除 data/style：
         * - 该组件把“数据源/图层/相机/交互”等收敛到自己的结构化配置里（style.sources/style.layers/...）
         * - 不使用通用的 data/style 面板
         */
        const base = super.getMenuToConfigContentMap();
        delete base['data'];
        delete base['style'];
        return {
            ...base,
            map_base: MapLibreGisConfig,
            camera: MapLibreGisCameraConfig,
            interaction: MapLibreGisInteractionConfig,
            projection: MapLibreGisProjectionConfig,
            light: MapLibreGisLightConfig,
            sky: MapLibreGisSkyConfig,
            terrain: MapLibreGisTerrainConfig,
            sources: MapLibreGisSourcesConfig,
            layers: MapLibreGisLayersConfig,
        };
    }

    getInitConfig(): MapLibreGisComponentProps {
        /**
         * 默认配置尽量满足“开箱即用”：
         * - 使用官方 demo styleUrl
         * - 初始化一个可视角（center/zoom/bearing/pitch）
         * - projection 使用 mercator
         * - interaction 默认开启常见 handlers，并给出合理默认参数
         */
        return {
            base: {
                id: '',
                name: 'GIS(MapLibre)',
                type: 'MapLibreGis',
            },
            style: {
                base: {
                    styleUrl: 'https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json',
                },
                camera: {
                    mode: 'center',
                    center: [120.2446746826172, 30.199146446037616],
                    zoom: 2,
                    bearing: 0,
                    pitch: 0,
                },
                projection: {
                    mode: 'static',
                    type: 'mercator',
                },
                light: {
                    enabled: true,
                    anchor: 'map',
                    position: { r: 1.5, a: 90, p: 80 },
                    color: '#ffffff',
                    intensity: 0.5,
                },
                sky: {
                    enabled: true,
                    mode: 'inherit',
                    skyColor: '#88C6FC',
                    horizonColor: '#ffffff',
                    fogColor: '#ffffff',
                    skyHorizonBlend: 0.8,
                    horizonFogBlend: 0.8,
                    fogGroundBlend: 0.5,
                    atmosphereBlend: { mode: 'zoom', z1: 10, z2: 12 },
                },
                interaction: {
                    enabled: true,
                    handlers: {
                        dragPan: true,
                        scrollZoom: true,
                        boxZoom: true,
                        doubleClickZoom: true,
                        dragRotate: true,
                        keyboard: true,
                        touchZoomRotate: true,
                        touchPitch: true,
                    },
                    cooperativeGestures: false,
                    clickTolerance: 3,
                    bearingSnap: 7,
                    scrollZoom: { aroundCenter: false, wheelZoomRate: 1 / 450, trackpadZoomRate: 1 / 100 },
                    dragPan: { linearity: 0, maxSpeed: 1400, deceleration: 2500 },
                },
                sources: [],
                layers: [],
            },
            filter: {
                enable: false,
                blur: 0,
                brightness: 1,
                contrast: 1,
                opacity: 1,
                saturate: 1,
                hueRotate: 0,
            }
        };
    }

    getEventList(): EventInfo[] {
        const events = super.getEventList();
        return events.concat([
            { id: 'click', name: '地图点击' },
            { id: 'featureClick', name: '要素点击' },
            { id: 'moveEnd', name: '视角变化' },
            { id: 'zoomEnd', name: '缩放结束' },
            { id: 'dataChange', name: '数据更新' },
        ]);
    }
}
