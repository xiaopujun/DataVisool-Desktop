/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {chmod, copyFile, lstat, mkdtemp, mkdir, readFile, readdir, rm, writeFile} from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const appDir = path.resolve(scriptDir, '..');
const binariesDir = path.join(appDir, 'src-tauri', 'binaries');
const manifestPath = path.join(binariesDir, 'ripgrep-manifest.json');
const verifyOnly = process.argv.includes('--verify');

const sha256 = async (filePath) => createHash('sha256').update(await readFile(filePath)).digest('hex');

const findBinary = async (directory, expectedName) => {
    for (const entry of await readdir(directory, {withFileTypes: true})) {
        const candidate = path.join(directory, entry.name);
        if (entry.isDirectory()) {
            const nested = await findBinary(candidate, expectedName);
            if (nested) return nested;
        } else if (entry.isFile() && entry.name === expectedName) {
            return candidate;
        }
    }
    return null;
};

const verifyBinary = async (target) => {
    const binaryPath = path.join(binariesDir, target.binary);
    const metadata = await lstat(binaryPath).catch(() => null);
    if (!metadata?.isFile() || metadata.isSymbolicLink()) {
        throw new Error(`缺少 ripgrep Sidecar：${target.binary}。请运行 pnpm run sidecar:prepare`);
    }
    const digest = await sha256(binaryPath);
    if (digest !== target.binarySha256) {
        throw new Error(`ripgrep Sidecar 校验失败：${target.binary}`);
    }
};

const prepareBinary = async (target) => {
    const workDir = await mkdtemp(path.join(os.tmpdir(), `datavisool-ripgrep-${target.triple}-`));
    try {
        const archivePath = path.join(workDir, target.archive);
        const response = await fetch(target.downloadUrl);
        if (!response.ok) {
            throw new Error(`下载失败：HTTP ${response.status} ${target.archive}`);
        }
        await writeFile(archivePath, Buffer.from(await response.arrayBuffer()));
        if (await sha256(archivePath) !== target.archiveSha256) {
            throw new Error(`下载包 SHA-256 校验失败：${target.archive}`);
        }
        const extractDir = path.join(workDir, 'extract');
        await mkdir(extractDir);
        const extracted = spawnSync('tar', ['-xf', archivePath, '-C', extractDir], {encoding: 'utf8'});
        if (extracted.status !== 0) {
            throw new Error(`解压失败：${target.archive}\n${extracted.stderr || extracted.stdout}`.trim());
        }
        const sourceName = target.binary.endsWith('.exe') ? 'rg.exe' : 'rg';
        const sourcePath = await findBinary(extractDir, sourceName);
        if (!sourcePath) {
            throw new Error(`下载包中缺少 ${sourceName}：${target.archive}`);
        }
        if (await sha256(sourcePath) !== target.binarySha256) {
            throw new Error(`解压后的 Sidecar SHA-256 校验失败：${target.binary}`);
        }
        const destination = path.join(binariesDir, target.binary);
        await copyFile(sourcePath, destination);
        if (!destination.endsWith('.exe')) await chmod(destination, 0o755);
    } finally {
        await rm(workDir, {recursive: true, force: true});
    }
};

const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
if (
    !manifest.version
    || !Array.isArray(manifest.targets)
    || manifest.targets.length === 0
    || manifest.targets.some((target) => !target.downloadUrl)
) {
    throw new Error('ripgrep-manifest.json 格式无效');
}

await mkdir(binariesDir, {recursive: true});
if (!verifyOnly) {
    for (const target of manifest.targets) {
        process.stdout.write(`准备 ripgrep ${manifest.version} (${target.triple})\n`);
        await prepareBinary(target);
    }
}
for (const target of manifest.targets) await verifyBinary(target);

const hostTriple = process.platform === 'darwin'
    ? `${process.arch === 'arm64' ? 'aarch64' : 'x86_64'}-apple-darwin`
    : process.platform === 'win32' && process.arch === 'x64'
        ? 'x86_64-pc-windows-msvc'
        : null;
const host = manifest.targets.find((target) => target.triple === hostTriple);
if (host) {
    const version = spawnSync(path.join(binariesDir, host.binary), ['--version'], {encoding: 'utf8'});
    if (version.status !== 0 || !version.stdout.startsWith(`ripgrep ${manifest.version}`)) {
        throw new Error(`ripgrep 原生 Sidecar 冒烟检查失败：${host.binary}`);
    }
}

process.stdout.write(`ripgrep ${manifest.version} Sidecar 校验通过（${manifest.targets.length} 个目标）\n`);
