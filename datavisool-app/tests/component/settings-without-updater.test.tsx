/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {fireEvent, render, screen} from '@testing-library/react';
import {describe, expect, it, vi} from 'vitest';

vi.mock('@/util/FetchUtil.ts', () => ({
    default: {get: vi.fn().mockResolvedValue({resourceStorageDir: null})},
}));

import Setting from '@/pages/home/setting/Setting';

describe('settings without updater', () => {
    it('shows the version without a check or latest-version indicator', async () => {
        render(<Setting/>);
        fireEvent.click(await screen.findByRole('menuitem', {name: /关于|About/i}));
        expect(await screen.findByText(/^(版本|Version)\s*[:：]/i)).toBeInTheDocument();
        expect(screen.queryByText(/发布日期|release date|publish date/i)).not.toBeInTheDocument();
        expect(screen.queryByRole('button', {name: /检查更新|Check for updates/i})).not.toBeInTheDocument();
        expect(screen.queryByText(/^(最新版本|Latest version)$/i)).not.toBeInTheDocument();
        expect(screen.queryByRole('button', {name: /开源软件声明|Open.source software/i})).not.toBeInTheDocument();
        expect(screen.queryByRole('button', {name: /帮助文档|Help documentation/i})).not.toBeInTheDocument();
    });
});
