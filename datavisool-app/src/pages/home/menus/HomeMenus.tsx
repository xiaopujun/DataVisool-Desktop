/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import { useLocation, useNavigate } from "react-router-dom";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { Api, Box, Brain, Cube, DashboardOne, DatabaseSetting, DataOne, DataServer, FileCodeOne, FolderClose, FontSize, Send, Setting, VideoOne } from "@icon-park/react";
import {Menu, type MenuItemType} from "@datavisool/ui";
import "@/pages/home/menus/HomeMenus.less";
import {tx} from "@/i18n/miniI18n.ts";
import {useLocaleVersion} from "@/i18n/useLocaleVersion.ts";
import sidebarLogo from "@/images/datavisool-logo.webp";

export const HomeMenus = memo(() => {
    useLocaleVersion();

    const navigate = useNavigate();
    const location = useLocation();
    const navigateTimerRef = useRef<number>();
    const { pathname } = location;
    const currentMenu = pathname.substring(pathname.lastIndexOf('/') + 1, pathname.length);
    const routeSwitchDelay = 260;
    const menuParentMap = useMemo<Record<string, string>>(() => ({
        database: 'datasource',
        api: 'datasource',
        font: 'resourceManagement',
        dynamicScript: 'resourceManagement',
        videoResource: 'resourceManagement',
        modelResource: 'resourceManagement'
    }), []);
    const [openKeys, setOpenKeys] = useState<string[]>(() => {
        const parent = menuParentMap[currentMenu];
        return parent ? [parent] : [];
    });
    const [activeMenuKey, setActiveMenuKey] = useState(currentMenu);

    useEffect(() => {
        const parent = menuParentMap[currentMenu];
        setOpenKeys(parent ? [parent] : []);
        setActiveMenuKey(currentMenu);
    }, [currentMenu, menuParentMap]);

    useEffect(() => {
        return () => {
            if (navigateTimerRef.current) {
                window.clearTimeout(navigateTimerRef.current);
            }
        }
    }, []);

    const changeMenu = (menu: { key: string }) => {
        if (menu.key === currentMenu) return;
        if (navigateTimerRef.current) {
            window.clearTimeout(navigateTimerRef.current);
        }
        setActiveMenuKey(menu.key);
        const parent = menuParentMap[menu.key];
        setOpenKeys(parent ? [parent] : []);
        window.dispatchEvent(new CustomEvent('datavisool:home-route-leave'));
        navigateTimerRef.current = window.setTimeout(() => navigate(`${menu.key}`), routeSwitchDelay);
    }

    const getMenus = () => {
        const baseMenus: MenuItemType[] = [
            {
                key: 'console',
                className: 'datavisool-app-menu-node datavisool-app-menu-node-console',
                icon: <DashboardOne theme="outline" size={18} fill="currentColor" />,
                label: tx('控制台')
            },
            {
                key: 'server',
                className: 'datavisool-app-menu-node datavisool-app-menu-node-server',
                icon: <FolderClose theme="outline" size={18} fill="currentColor" />,
                label: tx('项目列表')
            },
            {
                key: 'aiModel',
                className: 'datavisool-app-menu-node datavisool-app-menu-node-aiModel',
                icon: <Brain theme="outline" size={18} fill="currentColor" />,
                label: tx('AI模型管理')
            },
            {
                key: 'datasource',
                icon: <DataOne theme="outline" size={18} fill="currentColor" />,
                label: tx('数据源管理'),
                children: [
                    {
                        key: 'database',
                        className: 'datavisool-app-menu-node datavisool-app-menu-node-database',
                        icon: <DatabaseSetting theme="outline" size={18} fill="currentColor" />,
                        label: tx('数据库管理')
                    },
                    {
                        key: 'api',
                        className: 'datavisool-app-menu-node datavisool-app-menu-node-api',
                        icon: <Api theme="outline" size={18} fill="currentColor" />,
                        label: tx('API管理')
                    }
                ]
            },
            {
                key: 'resourceManagement',
                icon: <Box theme="outline" size={18} fill="currentColor" />,
                label: tx('资源管理'),
                children: [
                    {
                        key: 'font',
                        className: 'datavisool-app-menu-node datavisool-app-menu-node-font',
                        icon: <FontSize theme="outline" size={18} fill="currentColor" />,
                        label: tx('字体'),
                    },
                    {
                        key: 'dynamicScript',
                        className: 'datavisool-app-menu-node datavisool-app-menu-node-dynamicScript',
                        icon: <FileCodeOne theme="outline" size={18} fill="currentColor" />,
                        label: tx('动态脚本'),
                    },
                    {
                        key: 'videoResource',
                        className: 'datavisool-app-menu-node datavisool-app-menu-node-videoResource',
                        icon: <VideoOne theme="outline" size={18} fill="currentColor" />,
                        label: tx('视频'),
                    },
                    {
                        key: 'modelResource',
                        className: 'datavisool-app-menu-node datavisool-app-menu-node-modelResource',
                        icon: <Cube theme="outline" size={18} fill="currentColor" />,
                        label: tx('3D模型'),
                    }
                ]
            },
            {
                key: 'remoteComponent',
                className: 'datavisool-app-menu-node datavisool-app-menu-node-remoteComponent',
                icon: <DataServer theme="outline" size={18} fill="currentColor" />,
                label: tx('服务器组件')
            },
            {
                key: 'releaseManagement',
                className: 'datavisool-app-menu-node datavisool-app-menu-node-releaseManagement',
                icon: <Send theme="outline" size={18} fill="currentColor" />,
                label: tx('发布管理')
            },
            {
                key: 'setting',
                className: 'datavisool-app-menu-node datavisool-app-menu-node-setting',
                icon: <Setting theme="outline" size={18} fill="currentColor" />,
                label: tx('设置')
            }
        ]
        return baseMenus;
    }

    return (
        <div className={'datavisool-app-home-menus'}>
            <div className="sidebar-brand" aria-label="DataVisool">
                <img src={sidebarLogo} alt="" draggable={false} />
                <span>DataVisool</span>
            </div>
            <div className="menus-container">
                <Menu onClick={changeMenu}
                    className="home-primary-menu"
                    defaultSelectedKeys={[activeMenuKey]}
                    selectedKeys={[activeMenuKey]}
                    openKeys={openKeys}
                    onOpenChange={(keys) => {
                        setOpenKeys(keys.slice(-1) as string[]);
                    }}
                    mode="inline"
                    items={getMenus()}
                />
            </div>
        </div>
    );
})
