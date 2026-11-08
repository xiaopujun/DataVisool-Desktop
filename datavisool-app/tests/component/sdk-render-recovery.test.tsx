/* Licensed under the DataVisool License. See the LICENSE file distributed with this software. */
import {Component, type ReactNode} from 'react';
import {act} from '@testing-library/react';
import {expect, it, vi} from 'vitest';
import {ComponentUtil} from '@datavisool/sdk/react';

it('recovers after a render error in the same root and preserves healthy instances on update', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    class Sample extends Component<{broken: boolean; text: string}> {
        render(): ReactNode {
            if (this.props.broken) throw new Error('render failed');
            return <span>{this.props.text}</span>;
        }
    }
    const container = document.createElement('div');
    document.body.append(container);
    const render = async (broken: boolean, text: string) => {
        let result!: Promise<Sample | null>;
        await act(async () => { result = ComponentUtil.createAndRender<Sample>(container, Sample, {broken, text}); });
        return result;
    };
    try {
        const first = await render(false, 'one');
        expect(first).toBeInstanceOf(Sample);
        expect(await render(false, 'two')).toBe(first);
        expect(container.textContent).toBe('two');
        expect(await render(true, '')).toBeNull();
        expect(container.querySelector('[role="alert"]')).not.toBeNull();
        const recovered = await render(false, 'recovered');
        expect(recovered).toBeInstanceOf(Sample);
        expect(container.textContent).toBe('recovered');
    } finally {
        act(() => ComponentUtil.unmount(container));
        container.remove();
        vi.restoreAllMocks();
    }
});
