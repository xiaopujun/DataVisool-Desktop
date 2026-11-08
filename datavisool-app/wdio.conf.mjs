import path from 'node:path';
import {fileURLToPath} from 'node:url';

const appDir = path.dirname(fileURLToPath(import.meta.url));
const binaryName = process.platform === 'win32' ? 'datavisool.exe' : 'datavisool';
const appBinaryPath = path.join(appDir, 'src-tauri', 'target', 'debug', binaryName);

export const config = {
    runner: 'local',
    specs: [path.join(appDir, 'tests', 'desktop', '**', '*.spec.mjs')],
    maxInstances: 1,
    logLevel: 'warn',
    framework: 'mocha',
    reporters: ['spec'],
    mochaOpts: {timeout: 60000},
    services: [['@wdio/tauri-service', {
        appBinaryPath,
        driverProvider: 'embedded',
    }]],
    capabilities: [{
        browserName: 'tauri',
        'tauri:options': {application: appBinaryPath},
    }],
};
