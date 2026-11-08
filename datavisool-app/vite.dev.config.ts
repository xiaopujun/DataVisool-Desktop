/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */

import {defineConfig} from 'vite'
import viteBaseConfig from "./vite.base.config";

// https://vitejs.dev/config/
const devConfig = defineConfig({
    clearScreen: false,
    server: {
        port: 5174,
        strictPort: true,
        proxy: {
            '/api': {
                target: 'http://127.0.0.1:8080',
                changeOrigin: true,
            },
            '/static': {
                target: 'http://127.0.0.1:8080',
                changeOrigin: true,
            }
        },
        watch: {
            ignored: ['**/src-tauri/**'],
        }
    },
    css: {
        preprocessorOptions: {
            //对css预处理器默认配置的覆盖
        },
        devSourcemap: true,
        postcss: {}
    },
})

export default {...viteBaseConfig, ...devConfig}
