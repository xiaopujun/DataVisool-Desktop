/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {afterEach, expect, test, vi} from 'vitest';
import {fireEvent, render, screen} from '@testing-library/react';
import {Menu} from '@datavisool/ui';

afterEach(() => vi.restoreAllMocks());

test('shows the indicator for the selected item and moves it after selection changes', () => {
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
        const top = this.classList.contains('datavisool-menu-item')
            ? (this.textContent?.includes('第二项') ? 70 : 30)
            : 10;
        return {top, left: 10, right: 130, bottom: top + 40, width: 120, height: 40, x: 10, y: top, toJSON: () => ({})};
    });
    const {container} = render(<Menu defaultSelectedKeys={['first']} items={[
        {key: 'first', label: '第一项'},
        {key: 'second', label: '第二项'},
    ]}/>);
    const indicator = container.querySelector<HTMLElement>('.datavisool-menu-active-indicator')!;
    expect(indicator).toHaveClass('is-visible');
    expect(indicator.style.top).toBe('20px');
    fireEvent.click(screen.getByRole('menuitem', {name: '第二项'}));
    expect(indicator).toHaveClass('is-visible');
    expect(indicator.style.top).toBe('60px');
});
