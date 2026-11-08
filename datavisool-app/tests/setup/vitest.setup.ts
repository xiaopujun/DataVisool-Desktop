/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import '@testing-library/jest-dom/vitest'
import {cleanup} from '@testing-library/react'
import {afterEach, vi} from 'vitest'

afterEach(() => {
    cleanup()
})

Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: vi.fn().mockImplementation((query: string) => ({
        matches: false,
        media: query,
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(),
    })),
})

if (typeof window.localStorage?.getItem !== 'function') {
    const storageValues = new Map<string, string>()
    Object.defineProperty(window, 'localStorage', {
        configurable: true,
        value: {
            clear: () => storageValues.clear(),
            getItem: (key: string) => storageValues.get(key) ?? null,
            removeItem: (key: string) => storageValues.delete(key),
            setItem: (key: string, value: string) => storageValues.set(key, value),
        },
    })
}
