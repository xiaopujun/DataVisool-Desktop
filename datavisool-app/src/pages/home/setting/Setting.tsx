/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {Card, Input, Menu, Select} from "@datavisool/ui";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Close, FolderOpen, Help, Setting as SettingIcon } from "@icon-park/react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import styles from "@/pages/home/setting/Setting.module.less";
import packageJson from "@/../package.json";
import FetchUtil from "@/util/FetchUtil.ts";
import { globalMessage } from "@datavisool/ui";
import {getLocale, LocaleCode, setLocale, tx} from "@/i18n/miniI18n.ts";
import {useLocaleVersion} from "@/i18n/useLocaleVersion.ts";
import settingsGearArt from "@/pages/home/setting/assets/settings-gear-art.webp";
import generalGlobeArt from "@/pages/home/setting/assets/general-globe-art.webp";
import aboutVersionBanner from "@/pages/home/setting/assets/about-version-banner.webp";

export default function Setting() {
    useLocaleVersion();
    const [activeKey, setActiveKey] = useState<'general' | 'about'>('general');
    const [resourceStorageDir, setResourceStorageDir] = useState<string>('');
    const [resourceStorageLoading, setResourceStorageLoading] = useState<boolean>(true);
    const [resourceStorageLoadFailed, setResourceStorageLoadFailed] = useState<boolean>(false);
    const [resourceStorageSaving, setResourceStorageSaving] = useState<boolean>(false);
    const loadRequestVersion = useRef(0);
    const locale = getLocale();

    const isTauri =
        typeof window !== 'undefined' && (('__TAURI_INTERNALS__' in window) || ('__TAURI__' in window));

    const saveResourceStorageDir = useCallback(async (dir: string) => {
        setResourceStorageSaving(true);
        try {
            const normalized = dir.trim();
            const result = await FetchUtil.post<object | null>('/api/appConfig/save', {
                resourceStorageDir: normalized ? normalized : null
            }, {suppressErrorMessage: true});
            if (result === null) {
                globalMessage.error(tx('保存失败'));
                return;
            }
            setResourceStorageDir(normalized);
            globalMessage.success(tx('已保存'));
        } catch (e) {
            console.error('save appConfig failed', e);
            globalMessage.error(tx('保存失败'));
        } finally {
            setResourceStorageSaving(false);
        }
    }, []);

    const chooseStorageDir = useCallback(async () => {
        try {
            const selected = await openDialog({
                directory: true,
                multiple: false,
                title: tx('选择资源存储位置')
            });
            if (!selected) return;
            if (Array.isArray(selected)) return;
            await saveResourceStorageDir(selected);
        } catch (e) {
            console.error('chooseStorageDir failed', e);
            globalMessage.error(tx('选择目录失败'));
        }
    }, [saveResourceStorageDir]);

    const languageOptions = useMemo<Array<{ label: string; value: LocaleCode }>>(() => ([
        {label: tx('简体中文'), value: 'zh-CN'},
        {label: 'English', value: 'en-US'}
    ]), [locale]);

    const changeLocale = useCallback((nextLocale: LocaleCode) => {
        if (nextLocale === getLocale()) return;
        setLocale(nextLocale);
        globalMessage.success(tx('语言设置已更新'));
    }, []);

    const handleLocaleChange = useCallback((nextValue: string | number | Array<string | number>) => {
        if (Array.isArray(nextValue)) return;
        changeLocale(nextValue as LocaleCode);
    }, [changeLocale]);

    const menuItems = useMemo(() => ([
        { key: 'general', icon: <SettingIcon size={20} />, label: tx('通用') },
        { key: 'about', icon: <Help size={20} />, label: tx('关于') }
    ]), [locale]);

    const loadResourceStorageDir = useCallback(async () => {
        const version = ++loadRequestVersion.current;
        setResourceStorageLoading(true);
        try {
            const res = await FetchUtil.get<{ resourceStorageDir?: string | null } | null>(
                '/api/appConfig/get', {suppressErrorMessage: true}
            );
            if (version !== loadRequestVersion.current) return;
            if (res === null) {
                setResourceStorageLoadFailed(true);
                return;
            }
            setResourceStorageDir(typeof res.resourceStorageDir === 'string' ? res.resourceStorageDir : '');
            setResourceStorageLoadFailed(false);
        } catch (e) {
            if (version !== loadRequestVersion.current) return;
            console.error('load appConfig failed', e);
            setResourceStorageLoadFailed(true);
        } finally {
            if (version === loadRequestVersion.current) setResourceStorageLoading(false);
        }
    }, []);

    useEffect(() => {
        if (activeKey !== 'general' || resourceStorageSaving) return;
        void loadResourceStorageDir();
        return () => { loadRequestVersion.current++; };
    }, [activeKey, loadResourceStorageDir, resourceStorageSaving]);

    const section = useMemo(() => {
        if (activeKey === 'about') {
            return {
                title: '关于',
                subtitle: '版本信息',
                content: (
                    <div className={styles.aboutWrap}>
                        <div className={styles.aboutTopCard}>
                            <img className={styles.aboutVersionBanner} src={aboutVersionBanner} alt="" draggable={false} />
                            <div className={styles.aboutTopRow}>
                                <div className={styles.aboutTopLeft}>
                                    <div className={styles.aboutTopMeta}>
                                        <div className={styles.aboutVersionRow}>
                                            <span className={styles.aboutTopVersion}>版本：{packageJson.version}</span>
                                        </div>
                                    </div>
                                </div>
                            </div>
                        </div>
                    </div>
                )
            };
        }

        return {
            title: tx('通用'),
            subtitle: tx('外观、性能、缓存与基础偏好'),
            content: (
                <div className={styles.generalWrap}>
                    <div className={`${styles.generalBlock} ${styles.languageBlock}`}>
                        <img className={styles.languageArtwork} src={generalGlobeArt} alt="" draggable={false} />
                        <div className={styles.languageRow}>
                            <div className={styles.generalIcon} aria-hidden={true}>
                                <span>A文</span>
                            </div>
                            <div className={styles.generalMeta}>
                                <div className={styles.generalTitle}>{tx('界面语言')}</div>
                                <div className={styles.generalDesc}>{tx('切换应用界面的显示语言。')}</div>
                            </div>
                            <div className={styles.languageControl}>
                                <Select
                                    value={locale}
                                    options={languageOptions}
                                    className={styles.languageSelect}
                                    onChange={handleLocaleChange}
                                />
                            </div>
                        </div>
                    </div>
                    <div className={`${styles.generalBlock} ${styles.storageBlock}`}>
                        <div className={styles.storageRow}>
                            <div className={styles.generalIcon} aria-hidden={true}>
                                <FolderOpen size={36} />
                            </div>
                            <div className={styles.storageMain}>
                                <div className={styles.generalMeta}>
                                    <div className={styles.generalTitle}>{tx('资源存储位置')}</div>
                                    <div className={styles.generalDesc}>{tx('指定上传的静态资源存储在哪个目录下。')}</div>
                                </div>
                                <Input
                                    value={resourceStorageLoadFailed ? '' : resourceStorageDir}
                                    placeholder={resourceStorageLoading ? tx('正在读取配置') : resourceStorageLoadFailed ? tx('读取配置失败') : tx('默认使用 AppData（留空）')}
                                    readOnly={true}
                                    disabled={resourceStorageLoading || resourceStorageSaving || resourceStorageLoadFailed}
                                    className={styles.pathInput}
                                    suffix={(
                                        <div className={styles.inputSuffix}>
                                            {!resourceStorageLoadFailed && resourceStorageDir.trim() ? (
                                                <button
                                                    type="button"
                                                    className={styles.clearIconBtn}
                                                    onClick={() => saveResourceStorageDir('')}
                                                    disabled={resourceStorageLoading || resourceStorageSaving}
                                                    aria-label={tx('清空并恢复默认')}
                                                >
                                                    <Close size={14} />
                                                </button>
                                            ) : null}
                                            <button
                                                type="button"
                                                className={styles.folderIconBtn}
                                                onClick={chooseStorageDir}
                                                disabled={!isTauri || resourceStorageLoading || resourceStorageSaving || resourceStorageLoadFailed}
                                                aria-label={tx('选择目录')}
                                            >
                                                <FolderOpen size={16} />
                                            </button>
                                        </div>
                                    )}
                                />
                                {resourceStorageLoadFailed && !resourceStorageLoading && (
                                    <div className={styles.storageError} role="alert">
                                        <span>{tx('读取配置失败')}</span>
                                        <button type="button" onClick={() => void loadResourceStorageDir()}>{tx('重试')}</button>
                                    </div>
                                )}
                            </div>
                        </div>
                    </div>
                </div>
            )
        };
    }, [
        activeKey,
        chooseStorageDir,
        changeLocale,
        handleLocaleChange,
        languageOptions,
        locale,
        resourceStorageDir,
        resourceStorageLoading,
        resourceStorageLoadFailed,
        resourceStorageSaving,
        loadResourceStorageDir,
        saveResourceStorageDir,
        isTauri
    ]);

    return (
        <div className={styles.page}>
            <div className={styles.hero}>
                <span className={styles.heroBadge} aria-hidden="true">
                    <SettingIcon theme="outline" size={32} fill="currentColor" />
                </span>
                <div className={styles.heroText}>
                    <div className={styles.heroTitle}>设置</div>
                    <div className={styles.heroSubtitle}>个性化你的本地体验与偏好配置</div>
                </div>
            </div>

            <div className={styles.main}>
                <Card className={styles.navCard} bordered={false}>
                    <Menu
                        mode="inline"
                        items={menuItems}
                        selectedKeys={[activeKey]}
                        onClick={(info) => setActiveKey(info.key as any)}
                    />
                    <div className={styles.navArtwork} aria-hidden={true}>
                        <img src={settingsGearArt} alt="" draggable={false} />
                    </div>
                </Card>

                <Card className={styles.contentCard} bordered={false}>
                    <div className={styles.sectionHead}>
                        <div className={styles.sectionTitle}>{section.title}</div>
                        <div className={styles.sectionSubtitle}>{section.subtitle}</div>
                    </div>
                    <div className={styles.sectionBody}>
                        {section.content}
                    </div>
                </Card>
            </div>
        </div>
    )
}
