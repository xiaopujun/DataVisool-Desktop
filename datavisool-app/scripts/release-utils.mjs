/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {readFile} from 'node:fs/promises';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
export const appDir = path.resolve(scriptDir, '..');
export const srcTauriDir = path.join(appDir, 'src-tauri');

export const parseCliArgs = (argv) => {
    const args = {_ : []};
    for (let i = 0; i < argv.length; i++) {
        const token = argv[i];
        if (token === '--') {
            continue;
        }
        if (!token.startsWith('--')) {
            args._.push(token);
            continue;
        }

        const withoutPrefix = token.slice(2);
        const eqIndex = withoutPrefix.indexOf('=');
        if (eqIndex !== -1) {
            const key = withoutPrefix.slice(0, eqIndex);
            args[key] = withoutPrefix.slice(eqIndex + 1);
            continue;
        }

        const next = argv[i + 1];
        if (!next || next.startsWith('--')) {
            args[withoutPrefix] = true;
            continue;
        }

        args[withoutPrefix] = next;
        i += 1;
    }
    return args;
};

export const ensureAbsolute = (inputPath, baseDir = process.cwd()) => {
    if (!inputPath) return null;
    return path.isAbsolute(inputPath) ? inputPath : path.resolve(baseDir, inputPath);
};

export const readPackageVersion = async () => {
    const file = path.join(appDir, 'package.json');
    const {version} = JSON.parse(await readFile(file, 'utf8'));
    if (!version) throw new Error(`package.json 未配置 version：${file}`);
    return version;
};
