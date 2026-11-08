/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {defineConfig, mergeConfig} from 'vitest/config'
import viteBaseConfig from "./vite.base.config";

export default mergeConfig(viteBaseConfig, defineConfig({
    test: {
        environment: 'jsdom',
        globals: true,
        setupFiles: ['./tests/setup/vitest.setup.ts'],
        include: ['tests/component/**/*.test.ts', 'tests/component/**/*.test.tsx'],
        coverage: {
            provider: 'v8',
            reporter: ['text', 'html'],
            reportsDirectory: './coverage'
        }
    }
}))
