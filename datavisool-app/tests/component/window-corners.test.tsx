/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, describe, expect, test, vi} from 'vitest';
import {fireEvent, render, waitFor} from '@testing-library/react';

const windowHarness = vi.hoisted(() => ({current: null as unknown}));

vi.mock('@tauri-apps/api/window', () => ({getCurrentWindow: () => windowHarness.current}));

import TitleBar from '@/layouts/title-bar/TitleBar.tsx';

describe('main window corners', () => {
    afterEach(() => {
        document.documentElement.classList.remove('rounded-app-window');
        vi.restoreAllMocks();
    });

    test('rounds the Windows main window only while restored', async () => {
        vi.spyOn(navigator, 'platform', 'get').mockReturnValue('Win32');
        let maximized = false;
        windowHarness.current = {
            label: 'main',
            isFullscreen: vi.fn().mockResolvedValue(false),
            isMaximized: vi.fn(async () => maximized),
            toggleMaximize: vi.fn(async () => { maximized = !maximized; }),
            onResized: vi.fn(async () => () => {}),
        };

        const {container} = render(<TitleBar/>);
        await waitFor(() => expect(document.documentElement).toHaveClass('rounded-app-window'));

        const maximizeButton = container.querySelectorAll('.title-bar-button')[1];
        fireEvent.click(maximizeButton);
        await waitFor(() => expect(document.documentElement).not.toHaveClass('rounded-app-window'));

        fireEvent.click(maximizeButton);
        await waitFor(() => expect(document.documentElement).toHaveClass('rounded-app-window'));
    });

    test('keeps macOS preview window state checks within its permissions', async () => {
        vi.spyOn(navigator, 'platform', 'get').mockReturnValue('MacIntel');
        const isFullscreen = vi.fn().mockResolvedValue(false);
        const isMaximized = vi.fn();
        windowHarness.current = {
            label: 'preview-1',
            isFullscreen,
            isMaximized,
            onResized: vi.fn(async () => () => {}),
        };

        render(<TitleBar/>);
        await waitFor(() => expect(isFullscreen).toHaveBeenCalled());
        expect(isMaximized).not.toHaveBeenCalled();
        expect(document.documentElement).not.toHaveClass('rounded-app-window');
    });
});
