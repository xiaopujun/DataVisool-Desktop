/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';
import {build} from 'vite';
import react from '@vitejs/plugin-react';

// Build in Node: esbuild must not run in jsdom's separate typed-array realm.
const require = createRequire(import.meta.url);
const {externalPackages, externalGlobals} = require('../../../datavisool-cli/config/externals.js');
const entry = fileURLToPath(new URL('../../../datavisool-cli/template/src/components/remote-button/definition.ts', import.meta.url));
const result = await build({
    configFile: false,
    logLevel: 'silent',
    plugins: [react({jsxRuntime: 'classic'})],
    define: {'process.env.NODE_ENV': JSON.stringify('production')},
    build: {
        write: false,
        minify: false,
        lib: {entry, name: 'RemoteComponentFixture', formats: ['iife']},
        rollupOptions: {external: externalPackages, output: {globals: externalGlobals}},
    },
});
const outputs = Array.isArray(result) ? result : [result];
const chunk = outputs.flatMap(output => output.output).find(item => item.type === 'chunk');
if (!chunk) throw new Error('No component bundle generated');
process.stdout.write(chunk.code);
