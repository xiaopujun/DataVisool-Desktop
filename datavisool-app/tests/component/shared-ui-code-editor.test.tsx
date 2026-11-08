/* Licensed under the DataVisool License. See the LICENSE file distributed with this software. */
import {render, waitFor} from '@testing-library/react';
import {expect, it, vi} from 'vitest';
import {CodeEditor} from '@datavisool/ui/code-editor';

it('initializes the code editor, JavaScript mode, folding and change events', async () => {
    const rect = {left: 0, right: 100, top: 0, bottom: 20, width: 100, height: 20, x: 0, y: 0, toJSON: () => ({})};
    const originalRect = Object.getOwnPropertyDescriptor(Range.prototype, 'getBoundingClientRect');
    const originalRects = Object.getOwnPropertyDescriptor(Range.prototype, 'getClientRects');
    Object.defineProperty(Range.prototype, 'getBoundingClientRect', {value: () => rect, configurable: true});
    Object.defineProperty(Range.prototype, 'getClientRects', {value: () => [rect], configurable: true});
    const onChange = vi.fn();
    const {container, unmount} = render(<CodeEditor defaultValue="const value = 1;" language="javascript" onChange={onChange}/>);
    try {
        const element = await waitFor(() => {
            const found = container.querySelector('.CodeMirror');
            expect(found).not.toBeNull();
            return found;
        });
        const editor = (element as any).CodeMirror;
        expect(editor.getValue()).toBe('const value = 1;');
        expect(editor.getMode().name).toBe('javascript');
        expect(editor.getOption('foldGutter')).toBeTruthy();
        expect(editor.foldCode).toBeTypeOf('function');
        editor.setValue('const value = 2;');
        await waitFor(() => expect(onChange).toHaveBeenCalledWith('const value = 2;'));
    } finally {
        unmount();
        if (originalRect) Object.defineProperty(Range.prototype, 'getBoundingClientRect', originalRect);
        else delete (Range.prototype as any).getBoundingClientRect;
        if (originalRects) Object.defineProperty(Range.prototype, 'getClientRects', originalRects);
        else delete (Range.prototype as any).getClientRects;
    }
});
