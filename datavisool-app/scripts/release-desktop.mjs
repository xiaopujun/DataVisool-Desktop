/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {createReleaseOperations} from './release-operations.mjs';
import {cp, mkdir, readdir, readFile, rm, stat, writeFile} from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {spawn} from 'node:child_process';
import {
    appDir,
    ensureAbsolute,
    parseCliArgs,
    readPackageVersion,
    srcTauriDir,
} from './release-utils.mjs';

const args = parseCliArgs(process.argv.slice(2));
const releaseEnvPath = path.join(appDir, '.env.release');

const TARGET_MATRIX = {
    'darwin-aarch64': {
        family: 'darwin',
        label: 'macOS Apple Silicon',
        rustTarget: 'aarch64-apple-darwin',
        runner: null,
        configPath: path.join(srcTauriDir, 'tauri.macos.conf.json'),
    },
    'darwin-x86_64': {
        family: 'darwin',
        label: 'macOS Intel',
        rustTarget: 'x86_64-apple-darwin',
        runner: null,
        configPath: path.join(srcTauriDir, 'tauri.macos.conf.json'),
    },
    'windows-x86_64': {
        family: 'windows',
        label: 'Windows x64',
        rustTarget: 'x86_64-pc-windows-msvc',
        runner: 'cargo-xwin',
        configPath: path.join(srcTauriDir, 'tauri.windows.conf.json'),
    },
};

const DEFAULT_TARGET_KEYS = ['darwin-aarch64', 'darwin-x86_64', 'windows-x86_64'];
const KNOWN_FILE_EXTENSIONS = ['.app', '.dmg', '.exe'];

const loadReleaseEnv = async () => {
    const envFile = await readFile(releaseEnvPath, 'utf8').catch(() => null);
    if (!envFile) return false;

    for (const rawLine of envFile.split(/\r?\n/)) {
        const line = rawLine.trim();
        if (!line || line.startsWith('#')) continue;

        const eqIndex = line.indexOf('=');
        if (eqIndex === -1) continue;

        const key = line.slice(0, eqIndex).trim();
        let value = line.slice(eqIndex + 1).trim();
        if (!key) continue;

        if ((value.startsWith('"') && value.endsWith('"')) || (value.startsWith('\'') && value.endsWith('\''))) {
            value = value.slice(1, -1);
        }

        if (!(key in process.env)) {
            process.env[key] = value;
        }
    }

    return true;
};

const loadedReleaseEnv = await loadReleaseEnv();
const isTruthy = (value) => ['1', 'true', 'yes', 'on'].includes(String(value || '').trim().toLowerCase());

const helpText = `
DataVisool Desktop Release Script (Mac host)

Usage:
  pnpm -F datavisool-app release:desktop
  pnpm -F datavisool-app release:desktop:internal

What it does:
  - Cleans stale target bundle artifacts before every build
  - Builds macOS Apple Silicon
  - Builds macOS Intel
  - Cross-builds Windows x64 with cargo-xwin
  - Stages local app bundles and installers

Common options:
  --mac-bundles <list>  Comma-separated macOS bundles. Defaults to app,dmg. Use app to skip DMG
  --stage-dir <path>    Local staging root. Defaults to desktop-release, or desktop-release-internal in internal mode
  --targets <list>      Comma-separated build targets. Defaults to darwin-aarch64,darwin-x86_64,windows-x86_64
  --internal-devtools   Build an internal package that opens Tauri DevTools on startup
  --doctor              Only run environment checks and print setup hints
  --resume              Resume from current staged/bundle artifacts when possible
  --skip-install        Skip pnpm install --frozen-lockfile
  --skip-build          Skip all tauri builds
  --help                Show this help

Optional internal build env vars:
  DATAVISOOL_INTERNAL_DEVTOOLS     Set to 1/true to build an internal DevTools-enabled package

macOS code signing / notarization env vars:
  APPLE_SIGNING_IDENTITY    Example: Developer ID Application: Your Name (TEAMID)
  APPLE_API_ISSUER          App Store Connect API issuer ID
  APPLE_API_KEY             App Store Connect API key ID
  APPLE_API_KEY_PATH        Path to AuthKey_XXXXXX.p8
  APPLE_ID                  Apple account email (fallback to API key mode)
  APPLE_PASSWORD            Apple app-specific password
  APPLE_TEAM_ID             Apple Team ID
  DATAVISOOL_NOTARY_RETRY_ATTEMPTS Optional, defaults to 3 for transient notarization network failures
  DATAVISOOL_NOTARY_RETRY_DELAY_MS Optional, defaults to 15000 between retry attempts
`;

if (args.help) {
    process.stdout.write(helpText);
    process.exit(0);
}

if (process.platform !== 'darwin') {
    throw new Error('release:desktop 现在只支持在 macOS 上执行。请在 Mac 主机上运行该脚本。');
}

const resolveBin = (bin) => (process.platform === 'win32' ? `${bin}.cmd` : bin);
const repoRoot = path.resolve(appDir, '..');
const commandEnv = {...process.env};
const parsePositiveInt = (value, fallback) => {
    const parsed = Number.parseInt(String(value ?? '').trim(), 10);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
};
const notaryRetryAttempts = parsePositiveInt(process.env.DATAVISOOL_NOTARY_RETRY_ATTEMPTS, 3);
const notaryRetryDelayMs = parsePositiveInt(process.env.DATAVISOOL_NOTARY_RETRY_DELAY_MS, 15000);
const appendCommandOutput = (output, chunk, maxLength = 20000) => {
    const next = output + chunk.toString();
    return next.length > maxLength ? next.slice(next.length - maxLength) : next;
};

const runCommand = (command, commandArgs, {cwd = appDir, label = command, env = commandEnv, interactive = false} = {}) => (
    new Promise((resolve, reject) => {
        const child = spawn(command, commandArgs, {
            cwd,
            env,
            stdio: interactive ? 'inherit' : ['ignore', 'pipe', 'pipe'],
        });

        let stdout = '';
        let stderr = '';
        if (!interactive) {
            child.stdout.on('data', (chunk) => {
                stdout = appendCommandOutput(stdout, chunk);
                process.stdout.write(chunk);
            });
            child.stderr.on('data', (chunk) => {
                stderr = appendCommandOutput(stderr, chunk);
                process.stderr.write(chunk);
            });
        }
        child.on('error', (error) => reject(new Error(`${label} 启动失败：${error.message}`)));
        child.on('close', (code) => {
            if (code === 0) {
                resolve();
                return;
            }
            const details = stderr || stdout;
            reject(new Error(`${label} 执行失败，退出码 ${code}${details ? `\n${details}` : ''}`));
        });
    })
);

const captureCommand = (command, commandArgs, {cwd = appDir, label = command, env = commandEnv} = {}) => (
    new Promise((resolve, reject) => {
        const child = spawn(command, commandArgs, {
            cwd,
            env,
            stdio: ['ignore', 'pipe', 'pipe'],
        });

        let stdout = '';
        let stderr = '';
        child.stdout.on('data', (chunk) => {
            stdout += chunk.toString();
        });
        child.stderr.on('data', (chunk) => {
            stderr += chunk.toString();
        });
        child.on('error', (error) => reject(new Error(`${label} 启动失败：${error.message}`)));
        child.on('close', (code) => {
            if (code === 0) {
                resolve({stdout, stderr});
                return;
            }
            reject(new Error(`${label} 执行失败，退出码 ${code}\n${stderr || stdout}`.trim()));
        });
    })
);

const now = () => new Date().toISOString().replace('T', ' ').replace('Z', '');
const log = (message) => process.stdout.write(`[${now()}] ${message}\n`);
const divider = () => process.stdout.write(`${'-'.repeat(72)}\n`);
const expandHomePath = (value) => (value.startsWith('~/') ? path.join(os.homedir(), value.slice(2)) : value);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const isTransientAppleServiceError = (error) => {
    const message = String(error instanceof Error ? error.message : error);
    return [
        'HTTPClientError.connectTimeout',
        'NSURLErrorDomain Code=-1005',
        'abortedUpload',
        'notary-submissions-prod',
        'ticket-delivery',
        'CloudKit',
        'Error 68',
        'failed to notarize app',
        'timed out',
        'connection timed out',
        'network connection was lost',
    ].some((needle) => message.toLowerCase().includes(needle.toLowerCase()));
};

const runAppleServiceCommandWithRetries = async ({command, args: commandArgs, cwd = repoRoot, label, retryLabel = label}) => {
    for (let attempt = 1; attempt <= notaryRetryAttempts; attempt += 1) {
        try {
            await runCommand(command, commandArgs, {cwd, label});
            return;
        } catch (error) {
            const shouldRetry = attempt < notaryRetryAttempts && isTransientAppleServiceError(error);
            if (!shouldRetry) {
                throw error;
            }

            const delayMs = notaryRetryDelayMs * attempt;
            const waitSeconds = Math.ceil(delayMs / 1000);
            log(`- Apple 服务网络异常，准备重试 ${retryLabel}（第 ${attempt + 1}/${notaryRetryAttempts} 次，${waitSeconds}s 后重试）`);
            await sleep(delayMs);
        }
    }
};

const splitKnownExtension = (fileName) => {
    const matched = KNOWN_FILE_EXTENSIONS.find((extension) => fileName.endsWith(extension));
    if (matched) {
        return {
            stem: fileName.slice(0, -matched.length),
            extension: matched,
        };
    }

    const extension = path.extname(fileName);
    return {
        stem: extension ? fileName.slice(0, -extension.length) : fileName,
        extension,
    };
};

const tagFileName = (fileName, tag) => {
    const {stem, extension} = splitKnownExtension(fileName);
    return `${stem}_${tag}${extension}`;
};

const chooseReleaseAssetName = (sourcePath, targetKey, usedNames) => {
    const baseName = path.basename(sourcePath);
    let candidate = baseName;
    if (usedNames.has(candidate) || targetKey.startsWith('darwin-')) {
        candidate = tagFileName(baseName, targetKey);
    }
    usedNames.add(candidate);
    return candidate;
};

const maybePrependLlvmToPath = async () => {
    try {
        const {stdout} = await captureCommand(resolveBin('brew'), ['--prefix', 'llvm'], {
            cwd: repoRoot,
            label: 'brew --prefix llvm',
            env: process.env,
        });
        const prefix = stdout.trim();
        if (!prefix) return null;

        const llvmBinDir = path.join(prefix, 'bin');
        const llvmStat = await stat(llvmBinDir).catch(() => null);
        if (!llvmStat?.isDirectory()) return null;

        const currentPath = commandEnv.PATH || '';
        const parts = currentPath.split(path.delimiter).filter(Boolean);
        if (!parts.includes(llvmBinDir)) {
            commandEnv.PATH = [llvmBinDir, ...parts].join(path.delimiter);
        }
        return llvmBinDir;
    } catch {
        return null;
    }
};

const commandExists = async (command, env = commandEnv) => {
    try {
        await captureCommand(resolveBin('which'), [command], {
            cwd: repoRoot,
            label: `which ${command}`,
            env,
        });
        return true;
    } catch {
        return false;
    }
};

const resolveSelectedTargetKeys = () => {
    const raw = args.targets || args['target-key'] || process.env.DATAVISOOL_RELEASE_TARGETS;
    if (!raw) return DEFAULT_TARGET_KEYS;

    const values = String(raw)
        .split(',')
        .map((item) => item.trim())
        .filter(Boolean);

    if (values.length === 0) {
        throw new Error('参数 --targets 不能为空');
    }

    for (const targetKey of values) {
        if (!TARGET_MATRIX[targetKey]) {
            throw new Error(`不支持的 targetKey：${targetKey}`);
        }
    }

    return [...new Set(values)];
};

const discoverArtifacts = async (target, bundleDir) => {
    const locations = target.family === 'darwin'
        ? [...(macBundles.includes('app') ? [['macos', '.app', true]] : []),
           ...(macBundles.includes('dmg') ? [['dmg', '.dmg', false]] : [])]
        : [['nsis', '.exe', false]];
    const artifacts = [];
    for (const [directory, extension, isDirectory] of locations) {
        const dir = path.join(bundleDir, directory);
        const entries = await readdir(dir, {withFileTypes: true}).catch(() => []);
        const candidates = entries.filter((entry) => entry.name.endsWith(extension)
            && (isDirectory ? entry.isDirectory() : entry.isFile()));
        if (candidates.length !== 1) {
            throw new Error(`预期在 ${dir} 找到一个 ${extension} 产物，实际找到 ${candidates.length} 个`);
        }
        artifacts.push(path.join(dir, candidates[0].name));
    }
    return artifacts;
};

const rustBundleDir = (rustTarget) => path.join(srcTauriDir, 'target', rustTarget, 'release', 'bundle');

const selectedTargetKeys = resolveSelectedTargetKeys();
const selectedTargets = selectedTargetKeys.map((targetKey) => ({targetKey, ...TARGET_MATRIX[targetKey]}));
const internalDevtoolsEnabled = isTruthy(args['internal-devtools'] || process.env.DATAVISOOL_INTERNAL_DEVTOOLS);
const defaultStageDirName = internalDevtoolsEnabled ? 'desktop-release-internal' : 'desktop-release';
const stageRoot = ensureAbsolute(args['stage-dir']) || path.join(appDir, 'dist', defaultStageDirName);
const version = args.version || await readPackageVersion();
const isDoctorMode = Boolean(args.doctor);
const resumeMode = Boolean(args.resume) && !isDoctorMode;
const skipBuild = Boolean(args['skip-build']) && !isDoctorMode;

const macBundles = (() => {
    const raw = args['mac-bundles'] || process.env.DATAVISOOL_MAC_BUNDLES || 'app,dmg';
    const values = String(raw)
        .split(',')
        .map((item) => item.trim())
        .filter(Boolean);

    if (values.length === 0) {
        throw new Error('参数 --mac-bundles 不能为空');
    }

    const allowed = new Set(['app', 'dmg']);
    for (const value of values) {
        if (!allowed.has(value)) {
            throw new Error(`不支持的 macOS bundle 类型：${value}。可选值: app,dmg`);
        }
    }

    if (values.includes('dmg') && !values.includes('app')) values.unshift('app');
    return [...new Set(values)];
})();

const usedReleaseNames = new Set();
const progressStatePath = path.join(stageRoot, '.release-progress.json');
let releaseProgress = null;

const ensureDir = async (dirPath) => {
    await mkdir(dirPath, {recursive: true});
};

const pathExists = async (targetPath) => Boolean(await stat(targetPath).catch(() => null));
const allPathsExist = async (paths) => {
    for (const targetPath of paths.filter(Boolean)) {
        if (!(await pathExists(targetPath))) {
            return false;
        }
    }
    return true;
};

const createReleaseProgress = () => ({
    format: 2,
    version,
    targetKeys: selectedTargetKeys,
    internalDevtoolsEnabled,
    macBundles,
    targets: {},
    completed: false,
    updatedAt: new Date().toISOString(),
});

const normalizeReleaseProgress = (input) => ({
    ...createReleaseProgress(),
    ...input,
    targets: input?.targets && typeof input.targets === 'object' ? input.targets : {},
});

const saveReleaseProgress = async () => {
    if (isDoctorMode || !releaseProgress) return;

    releaseProgress.updatedAt = new Date().toISOString();
    await ensureDir(stageRoot);
    await writeFile(progressStatePath, `${JSON.stringify(releaseProgress, null, 2)}\n`, 'utf8');
};

const loadReleaseProgress = async () => {
    if (!resumeMode) {
        releaseProgress = createReleaseProgress();
        return;
    }

    const raw = await readFile(progressStatePath, 'utf8').catch(() => null);
    if (!raw) {
        releaseProgress = createReleaseProgress();
        log(`- 未找到断点进度文件，将优先从现有 bundle 产物恢复: ${progressStatePath}`);
        await saveReleaseProgress();
        return;
    }

    let parsed = null;
    try {
        parsed = JSON.parse(raw);
    } catch (error) {
        throw new Error(`断点进度文件无法解析：${progressStatePath}\n${error instanceof Error ? error.message : String(error)}`);
    }

    if (parsed?.format !== 2) {
        throw new Error('旧版断点记录不兼容当前本地构建流程，请重新开始构建');
    }

    if (parsed?.version && parsed.version !== version) {
        throw new Error(`断点进度文件版本与当前版本不一致：${parsed.version} != ${version}`);
    }

    if (Array.isArray(parsed?.targetKeys)) {
        const previous = JSON.stringify(parsed.targetKeys);
        const current = JSON.stringify(selectedTargetKeys);
        if (previous !== current) {
            throw new Error(`断点进度文件的目标集合与当前命令不一致：${previous} != ${current}`);
        }
    }

    if (Array.isArray(parsed?.macBundles)) {
        const previous = JSON.stringify(parsed.macBundles);
        const current = JSON.stringify(macBundles);
        if (previous !== current) {
            throw new Error(`断点进度文件的 macOS bundle 配置与当前命令不一致：${previous} != ${current}`);
        }
    }

    if (typeof parsed?.internalDevtoolsEnabled === 'boolean' && parsed.internalDevtoolsEnabled !== internalDevtoolsEnabled) {
        throw new Error('断点进度文件与当前 internal-devtools 构建模式不一致，请重新开始一轮新的发布');
    }

    releaseProgress = normalizeReleaseProgress(parsed);
    log(`- 已加载断点进度文件: ${progressStatePath}`);
};

const ensureTargetProgress = (targetKey) => {
    if (!releaseProgress.targets[targetKey] || typeof releaseProgress.targets[targetKey] !== 'object') {
        releaseProgress.targets[targetKey] = {};
    }
    return releaseProgress.targets[targetKey];
};

const markBuildProgress = async (result) => {
    ensureTargetProgress(result.targetKey).build = {
        completed: true, bundleDir: result.bundleDir, artifacts: result.artifacts,
    };
    await saveReleaseProgress();
};

const markStageProgress = async (result) => {
    ensureTargetProgress(result.targetKey).stage = {completed: true, artifacts: result.stagedArtifacts};
    await saveReleaseProgress();
};

const restoreBuildResultFromProgress = async (target) => {
    if (!resumeMode) return null;
    const entry = releaseProgress?.targets?.[target.targetKey]?.build;
    if (!entry?.completed || !Array.isArray(entry.artifacts) || !entry.artifacts.length
        || !(await allPathsExist([entry.bundleDir, ...entry.artifacts]))) return null;
    return {...target, bundleDir: entry.bundleDir, artifacts: entry.artifacts};
};

const restoreStagedResultFromProgress = async (buildResult) => {
    if (!resumeMode) return null;
    const entry = releaseProgress?.targets?.[buildResult.targetKey]?.stage;
    if (!entry?.completed || !Array.isArray(entry.artifacts) || !entry.artifacts.length
        || !(await allPathsExist(entry.artifacts))) return null;
    for (const artifact of entry.artifacts) usedReleaseNames.add(path.basename(artifact));
    return {...buildResult, stagedArtifacts: entry.artifacts};
};

const cleanTargetBundleArtifacts = async (target) => {
    const bundleDir = rustBundleDir(target.rustTarget);
    await rm(bundleDir, {recursive: true, force: true});
    log(`- 已清理历史 bundle 目录: ${bundleDir}`);
};

const {readBuildResultFromBundle, runPreflightChecks, buildTarget, stageTargetArtifacts} = createReleaseOperations({
    appDir, captureCommand, chooseReleaseAssetName, cleanTargetBundleArtifacts, commandEnv, commandExists,
    cp, discoverArtifacts, divider, ensureDir, expandHomePath, internalDevtoolsEnabled,
    isTransientAppleServiceError, log, macBundles, maybePrependLlvmToPath, notaryRetryAttempts, notaryRetryDelayMs,
    path, pathExists, repoRoot, resolveBin, runAppleServiceCommandWithRetries, runCommand, rustBundleDir,
    selectedTargets, sleep, stageRoot, stat, usedReleaseNames, version,
});

try {
    divider();
    log('DataVisool Desktop 全平台发布脚本开始执行');
    log(`版本: ${version}`);
    log(`宿主平台: ${process.platform}`);
    log(`宿主架构: ${process.arch}`);
    if (isDoctorMode) {
        log('模式: 发布环境自检 (--doctor)');
    } else {
        log(internalDevtoolsEnabled ? '构建类型: 内部调试包（启用 DevTools）' : '构建类型: 标准发布包');
        if (resumeMode) {
            log('执行模式: 断点续跑 (--resume)');
        }
        log(`本次构建目标: ${selectedTargets.map((target) => `${target.targetKey}(${target.rustTarget})`).join(', ')}`);
        log(`本地暂存目录: ${stageRoot}`);
    }
    if (loadedReleaseEnv) {
        log(`已加载配置文件: ${releaseEnvPath}`);
    }
    divider();

    if (!isDoctorMode) {
        if (resumeMode) {
            await ensureDir(stageRoot);
            await loadReleaseProgress();
        } else {
            await rm(stageRoot, {recursive: true, force: true});
            await ensureDir(stageRoot);
            await loadReleaseProgress();
            await saveReleaseProgress();
        }
    }

    await runPreflightChecks(skipBuild);

    if (isDoctorMode) {
        divider();
        log('发布环境自检通过，可以直接执行 pnpm -F datavisool-app release:desktop');
        divider();
        process.exit(0);
    }

    if (!args['skip-install']) {
        log('步骤 2/4: 安装依赖 (pnpm install --frozen-lockfile)');
        await runCommand(resolveBin('pnpm'), ['install', '--frozen-lockfile'], {
            cwd: repoRoot,
            label: 'pnpm install --frozen-lockfile',
        });
    } else {
        log('步骤 2/4: 跳过依赖安装 (--skip-install)');
    }

    const buildResults = [];
    if (args['skip-build']) {
        log(resumeMode
            ? '步骤 3/4: 断点续跑模式下跳过新构建，改为从现有 bundle 目录恢复产物'
            : '步骤 3/4: 跳过桌面构建 (--skip-build)，改为从现有 bundle 目录读取产物');
    } else if (resumeMode) {
        log('步骤 3/4: 断点续跑，优先复用已完成目标，缺失目标继续构建');
    } else {
        log('步骤 3/4: 顺序构建所有目标');
    }

    for (const target of selectedTargets) {
        const restoredBuildResult = await restoreBuildResultFromProgress(target);
        if (restoredBuildResult) {
            buildResults.push(restoredBuildResult);
            log(`- 已从断点进度恢复 ${target.label}: ${restoredBuildResult.bundleDir}`);
            continue;
        }

        const bundleDir = rustBundleDir(target.rustTarget);
        if ((resumeMode || args['skip-build']) && await pathExists(bundleDir)) {
            const recoveredResult = await readBuildResultFromBundle(target);
            buildResults.push(recoveredResult);
            await markBuildProgress(recoveredResult);
            log(`- 已从当前 bundle 产物恢复 ${target.label}: ${bundleDir}`);
            continue;
        }

        if (args['skip-build']) {
            throw new Error(`已启用 --skip-build，但未找到 ${target.label} 的现有 bundle 目录：${bundleDir}`);
        }

        const result = await buildTarget(target);
        buildResults.push(result);
        await markBuildProgress(result);
    }

    log('步骤 4/4: 整理本地构建产物');
    const stagedResults = [];
    for (const buildResult of buildResults) {
        const staged = await restoreStagedResultFromProgress(buildResult) || await stageTargetArtifacts(buildResult);
        stagedResults.push(staged);
        await markStageProgress(staged);
    }
    for (const result of stagedResults) {
        log(`- ${result.label}:`);
        for (const artifact of result.stagedArtifacts) log(`  ${artifact}`);
    }

    if (releaseProgress) {
        releaseProgress.completed = true;
        await saveReleaseProgress();
    }

    divider();
    log('全平台发布脚本执行完成');
    divider();
} catch (error) {
    divider();
    process.stderr.write(`[${now()}] 发布脚本执行失败\n`);
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    divider();
    process.exit(1);
}
