/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {useEffect, useState} from 'react';
import {getCurrentWindow} from '@tauri-apps/api/window';
import {Close, Minus, Square, Copy} from '@icon-park/react';
import '@/layouts/title-bar/TitleBar.less';
import logo from '@/images/datavisool-logo.webp';

function syncWindowCorners(windowLabel: string, zoomed: boolean) {
    document.documentElement.classList.toggle('rounded-app-window', windowLabel === 'main' && !zoomed);
}

export default function TitleBar() {
    const [appWindow, setAppWindow] = useState<any>(null);
    const [isMaximized, setIsMaximized] = useState(false);
    const [isFullscreen, setIsFullscreen] = useState(false);
    const [isMac, setIsMac] = useState(false);

    useEffect(() => {
        let mounted = true;
        let unlisten: (() => void) | null = null;
        const initWindow = async () => {
            try {
                const win = getCurrentWindow();
                if (win) {
                    if (!mounted) return;
                    setAppWindow(win);
                    const mac =
                        typeof navigator !== 'undefined' &&
                        /Mac/i.test(navigator.platform || navigator.userAgent);
                    if (mounted) setIsMac(mac);

                    const syncWindowState = async () => {
                        try {
                            const mainWindow = win.label === 'main';
                            const fullscreen = mac || mainWindow ? await win.isFullscreen() : false;
                            const maximized = !mac || mainWindow ? await win.isMaximized() : false;
                            if (!mounted) return;
                            if (mac) setIsFullscreen(fullscreen);
                            else setIsMaximized(maximized);
                            if (mainWindow) syncWindowCorners(win.label, fullscreen || maximized);
                        } catch (err) {
                            console.error('Failed to sync window state:', err);
                        }
                    };

                    await syncWindowState();
                    try {
                        unlisten = await win.onResized(syncWindowState);
                    } catch (err) {
                        console.error('Failed to setup window resize listener:', err);
                    }
                }
            } catch (e) {
                console.error('Failed to initialize window controls:', e);
            }
        };
        initWindow();
        return () => {
            mounted = false;
            document.documentElement.classList.remove('rounded-app-window');
            if (unlisten) unlisten();
        };
    }, []);

    const handleMinimize = async () => {
        if (!appWindow) return;
        try {
            if (isMac && isFullscreen) {
                await appWindow.setFullscreen(false);
                setIsFullscreen(false);
                syncWindowCorners(appWindow.label, false);
                await new Promise<void>((resolve) => setTimeout(resolve, 120));
            }
            await appWindow.minimize();
        } catch (err) {
            console.error('Failed to minimize window:', err);
        }
    };

    const handleToggleFullscreenOrMaximize = async () => {
        if (!appWindow) return;
        try {
            if (isMac) {
                const next = !isFullscreen;
                await appWindow.setFullscreen(next);
                setIsFullscreen(next);
                syncWindowCorners(appWindow.label, next);
            } else {
                await appWindow.toggleMaximize();
                const max = await appWindow.isMaximized();
                setIsMaximized(max);
                if (appWindow.label === 'main') {
                    syncWindowCorners(appWindow.label, max || await appWindow.isFullscreen());
                }
            }
        } catch (err) {
            console.error('Failed to toggle window state:', err);
        }
    };

    const handleClose = async () => {
        try {
            await appWindow?.close();
        } catch (err) {
            console.error('Failed to close window:', err);
        }
    };

    const isZoomed = isMac ? isFullscreen : isMaximized;
    
    return (
        <div
            className={`custom-title-bar${isMac ? ' mac-title-bar' : ''}`}
            data-tauri-drag-region
        >
            {isMac && (
                <div className="mac-traffic-lights">
                    <button className="mac-traffic-light close" onClick={handleClose}/>
                    <button className="mac-traffic-light minimize" onClick={handleMinimize}/>
                    <button className="mac-traffic-light zoom" onClick={handleToggleFullscreenOrMaximize}/>
                </div>
            )}
            <div className="title-bar-left" data-tauri-drag-region>
                <img src={logo} alt="logo" className="window-logo"/>
                <span className="window-title">DataVisool</span>
            </div>

            {!isMac && (
                <div className="title-bar-right" data-tauri-drag-region={false}>
                    <div className="title-bar-button" onClick={handleMinimize}>
                        <Minus theme="outline" size="16" fill="#a0a0b2"/>
                    </div>
                    <div className="title-bar-button" onClick={handleToggleFullscreenOrMaximize}>
                        {isZoomed ? (
                            <Copy theme="outline" size="14" fill="#a0a0b2"/>
                        ) : (
                            <Square theme="outline" size="14" fill="#a0a0b2"/>
                        )}
                    </div>
                    <div className="title-bar-button close-button" onClick={handleClose}>
                        <Close theme="outline" size="16" fill="#a0a0b2"/>
                    </div>
                </div>
            )}
        </div>
    );
}
