import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';

const appDir = fileURLToPath(new URL('..', import.meta.url));
const pnpm = process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm';
const env = {...process.env, VITE_DESKTOP_E2E: '1'};

function run(args) {
    const result = spawnSync(pnpm, args, {cwd: appDir, env, stdio: 'inherit'});
    if (result.error) throw result.error;
    if (result.status !== 0) process.exit(result.status ?? 1);
}

run(['exec', 'tauri', 'build', '--debug', '--no-bundle', '--features', 'desktop-e2e', '--config', 'src-tauri/tauri.e2e.conf.json']);
run(['exec', 'wdio', 'run', 'wdio.conf.mjs', ...process.argv.slice(2)]);
