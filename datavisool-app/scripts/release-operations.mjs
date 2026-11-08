/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
// 本地发布构建、签名、公证和暂存操作。
export function createReleaseOperations(context) {
  const {
    appDir, captureCommand, chooseReleaseAssetName, cleanTargetBundleArtifacts, commandEnv, commandExists,
    cp, discoverArtifacts, divider, ensureDir, expandHomePath, internalDevtoolsEnabled,
    isTransientAppleServiceError, log, macBundles, maybePrependLlvmToPath, notaryRetryAttempts, notaryRetryDelayMs,
    path, pathExists, repoRoot, resolveBin, runAppleServiceCommandWithRetries, runCommand, rustBundleDir,
    selectedTargets, sleep, stageRoot, stat, usedReleaseNames, version,
  } = context;

const resolveNotarytoolAuthArgs = () => {
    const appleApiIssuer = (process.env.APPLE_API_ISSUER || '').trim();
    const appleApiKey = (process.env.APPLE_API_KEY || '').trim();
    const appleApiKeyPath = expandHomePath((process.env.APPLE_API_KEY_PATH || '').trim());
    const appleId = (process.env.APPLE_ID || '').trim();
    const applePassword = (process.env.APPLE_PASSWORD || '').trim();
    const appleTeamId = (process.env.APPLE_TEAM_ID || '').trim();

    if (appleApiIssuer && appleApiKey && appleApiKeyPath) {
        return ['--issuer', appleApiIssuer, '--key-id', appleApiKey, '--key', appleApiKeyPath];
    }

    if (appleId && applePassword && appleTeamId) {
        return ['--apple-id', appleId, '--password', applePassword, '--team-id', appleTeamId];
    }

    throw new Error('未找到可用的 Apple notarization 凭据，请设置 APPLE_API_* 或 APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID');
};

const verifyMacAppCodeSignature = async (appBundlePath, label) => {
    await runCommand(resolveBin('codesign'), ['--verify', '--deep', '--strict', appBundlePath], {
        cwd: repoRoot,
        label: `codesign verify ${label}`,
    });
};

const assessMacArtifact = async (artifactPath, type, label) => {
    const args = ['--assess', '--type', type, '-vv'];
    if (type === 'open') {
        args.push('--context', 'context:primary-signature');
    }
    args.push(artifactPath);

    await runCommand(resolveBin('spctl'), args, {
        cwd: repoRoot,
        label: `spctl assess ${label}`,
    });
};

const stapleMacArtifact = async (artifactPath, label) => {
    await runAppleServiceCommandWithRetries({
        command: resolveBin('xcrun'),
        args: ['stapler', 'staple', artifactPath],
        label: `stapler staple ${label}`,
        retryLabel: `stapler staple ${label}`,
    });
    await runAppleServiceCommandWithRetries({
        command: resolveBin('xcrun'),
        args: ['stapler', 'validate', artifactPath],
        label: `stapler validate ${label}`,
        retryLabel: `stapler validate ${label}`,
    });
};

const hasStapledTicket = async (artifactPath, label) => {
    try {
        await runCommand(resolveBin('xcrun'), ['stapler', 'validate', artifactPath], {
            cwd: repoRoot,
            label: `stapler validate ${label}`,
        });
        return true;
    } catch {
        return false;
    }
};

const notarizeMacArchive = async (archivePath, label) => {
    const command = resolveBin('xcrun');
    const commandArgs = [
        'notarytool',
        'submit',
        archivePath,
        '--wait',
        '--timeout',
        '2h',
        ...resolveNotarytoolAuthArgs(),
    ];

    for (let attempt = 1; attempt <= notaryRetryAttempts; attempt += 1) {
        try {
            await runCommand(command, commandArgs, {
                cwd: repoRoot,
                label: `notarytool submit ${label}`,
            });
            return;
        } catch (error) {
            if (attempt >= notaryRetryAttempts) {
                throw error;
            }

            const delayMs = notaryRetryDelayMs * attempt;
            const waitSeconds = Math.ceil(delayMs / 1000);
            log(`- Apple notarization 提交失败，准备重试 ${label}（第 ${attempt + 1}/${notaryRetryAttempts} 次，${waitSeconds}s 后重试）`);
            await sleep(delayMs);
        }
    }
};

const finalizeDarwinBuildArtifacts = async (buildResult) => {
    const appBundlePath = buildResult.artifacts.find((filePath) => filePath.endsWith('.app'));
    if (!appBundlePath) throw new Error(`未找到 ${buildResult.label} 的 .app 目录`);
    if (!(await pathExists(appBundlePath))) {
        throw new Error(`未找到 ${buildResult.label} 的 .app 目录：${appBundlePath}`);
    }

    log(`- 校验 ${buildResult.label} .app 的代码签名与 Gatekeeper`);
    await verifyMacAppCodeSignature(appBundlePath, `${buildResult.targetKey} app`);
    if (await hasStapledTicket(appBundlePath, `${buildResult.targetKey} app`)) {
        log(`- 复用已存在的 ${buildResult.targetKey} app stapled ticket`);
    } else {
        await stapleMacArtifact(appBundlePath, `${buildResult.targetKey} app`);
    }
    await assessMacArtifact(appBundlePath, 'execute', `${buildResult.targetKey} app`);

    const dmgAssets = buildResult.artifacts.filter((filePath) => filePath.endsWith('.dmg'));
    for (const dmgPath of dmgAssets) {
        if (await hasStapledTicket(dmgPath, `${buildResult.targetKey} dmg`)) {
            log(`- 复用已存在的 ${path.basename(dmgPath)} stapled ticket`);
        } else {
            log(`- 提交 ${path.basename(dmgPath)} 到 Apple notarization`);
            await notarizeMacArchive(dmgPath, `${buildResult.targetKey} dmg`);
            await stapleMacArtifact(dmgPath, `${buildResult.targetKey} dmg`);
        }
        await assessMacArtifact(dmgPath, 'open', `${buildResult.targetKey} dmg`);
    }

    return buildResult;
};

const finalizeBuildResultArtifacts = async (buildResult) => {
    if (buildResult.family !== 'darwin') {
        return buildResult;
    }

    return finalizeDarwinBuildArtifacts(buildResult);
};

const readBuildResultFromBundle = async (target) => {
    const bundleDir = rustBundleDir(target.rustTarget);
    const artifacts = await discoverArtifacts(target, bundleDir);
    return finalizeBuildResultArtifacts({...target, bundleDir, artifacts});
};

const isTransientAppleNotarizationError = (error) => {
    return isTransientAppleServiceError(error);
};

const runTauriBuildWithRetries = async (target, buildArgs) => {
    const attempts = target.family === 'darwin' ? notaryRetryAttempts : 1;

    for (let attempt = 1; attempt <= attempts; attempt += 1) {
        try {
            await runCommand(resolveBin('pnpm'), buildArgs, {
                cwd: repoRoot,
                label: `tauri build ${target.targetKey}`,
            });
            return;
        } catch (error) {
            const shouldRetry = target.family === 'darwin'
                && attempt < attempts
                && isTransientAppleNotarizationError(error);
            if (!shouldRetry) {
                throw error;
            }

            const delayMs = notaryRetryDelayMs * attempt;
            const waitSeconds = Math.ceil(delayMs / 1000);
            log(`- Apple notarization 上传超时，准备重试 ${target.targetKey} 构建（第 ${attempt + 1}/${attempts} 次，${waitSeconds}s 后重试）`);
            await sleep(delayMs);
        }
    }
};

const formatNumberedCommands = (commands) => commands
    .map((command, index) => `${index + 1}. ${command}`)
    .join('\n');

const parseCodeSigningIdentities = (stdout) => stdout
    .split(/\r?\n/)
    .map((line) => {
        const match = line.match(/"(.+?)"/);
        return match ? match[1] : null;
    })
    .filter(Boolean);

const readCodeSigningIdentities = async () => {
    try {
        const {stdout} = await captureCommand(resolveBin('security'), ['find-identity', '-v', '-p', 'codesigning'], {
            cwd: repoRoot,
            label: 'security find-identity -v -p codesigning',
            env: process.env,
        });
        return parseCodeSigningIdentities(stdout);
    } catch {
        return [];
    }
};

const checkXcodeLicenseAccepted = async () => {
    try {
        await captureCommand(resolveBin('xcodebuild'), ['-license', 'check'], {
            cwd: repoRoot,
            label: 'xcodebuild -license check',
            env: process.env,
        });
        return true;
    } catch {
        return false;
    }
};

const buildInstallHints = ({
    hasBrew,
    hasRustup,
    missingRustTargets,
    needsCargoXwin,
    needsLlvmRc,
    needsMakensis,
    needsXcrun,
    needsXcodeLicense,
    needsAppleSigningIdentity,
    needsAppleDeveloperId,
    needsAppleNotaryCredentials,
    preferredNotaryMode,
}) => {
    const commands = [];
    const notes = [];

    if (needsXcrun) {
        commands.push('xcode-select --install');
    }

    if (needsXcodeLicense) {
        commands.push('sudo xcodebuild -license accept');
        commands.push('sudo xcodebuild -runFirstLaunch');
    }

    if (missingRustTargets.length > 0) {
        if (hasRustup) {
            commands.push(`rustup target add ${missingRustTargets.join(' ')}`);
        } else {
            notes.push(`先安装 Rust toolchain: https://rustup.rs，然后执行 rustup target add ${missingRustTargets.join(' ')}`);
        }
    }

    const brewPackages = [];
    if (needsLlvmRc) {
        brewPackages.push('llvm');
    }
    if (needsMakensis) {
        brewPackages.push('nsis');
    }

    if (brewPackages.length > 0) {
        const brewCommand = `brew install ${[...new Set(brewPackages)].join(' ')}`;
        if (hasBrew) {
            commands.push(brewCommand);
        } else {
            notes.push(`先安装 Homebrew: https://brew.sh，然后执行 ${brewCommand}`);
        }
    }

    if (needsCargoXwin) {
        if (hasRustup) {
            commands.push('cargo install --locked cargo-xwin');
        } else {
            notes.push('安装 Rust toolchain 后再执行: cargo install --locked cargo-xwin');
        }
    }

    if (needsAppleSigningIdentity) {
        commands.push('security find-identity -v -p codesigning');
        commands.push('export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"');
        notes.push('APPLE_SIGNING_IDENTITY 必须与 security find-identity 输出中的签名身份完全一致');
    }

    if (needsAppleDeveloperId) {
        notes.push('要对外分发且避免安装警告，请导入 Developer ID Application 证书及其私钥到登录钥匙串');
        notes.push('如果钥匙串里仍显示 0 valid identities found，请重新导入 .cer/.p12，并确认私钥也一并导入');
    }

    if (needsAppleNotaryCredentials) {
        if (preferredNotaryMode === 'apple-id') {
            commands.push('export APPLE_ID="your-apple-id@example.com"');
            commands.push('export APPLE_PASSWORD="your-app-specific-password"');
            commands.push('export APPLE_TEAM_ID="YOURTEAMID"');
        } else {
            commands.push('export APPLE_API_ISSUER="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"');
            commands.push('export APPLE_API_KEY="ABC123DEFG"');
            commands.push('export APPLE_API_KEY_PATH="$HOME/private_keys/AuthKey_ABC123DEFG.p8"');
        }
        notes.push('macOS Developer ID 对外分发必须完成 Apple notarization；本脚本会在构建后继续补齐 DMG 公证和 stapling');
        notes.push('推荐优先使用 App Store Connect API key，而不是 Apple ID + app-specific password');
    }

    return {commands, notes};
};

const runPreflightChecks = async (skipBuild) => {
    log('步骤 1/4: 检查 macOS 发布环境');
    const llvmBinDir = await maybePrependLlvmToPath();
    if (llvmBinDir) {
        log(`- 已将 LLVM 加入 PATH: ${llvmBinDir}`);
    }

    if (skipBuild) {
        log('- 已启用 --skip-build，跳过构建工具链强校验');
        return;
    }

    const missing = [];
    const hasBrew = await commandExists('brew', process.env);
    const hasXcrun = await commandExists('xcrun', commandEnv);
    const hasXcodebuild = await commandExists('xcodebuild', commandEnv);
    const hasRustup = await commandExists('rustup', commandEnv);
    const darwinTargetsSelected = selectedTargets.some((target) => target.family === 'darwin');
    const appleSigningIdentity = (process.env.APPLE_SIGNING_IDENTITY || '').trim();
    const appleCertificate = (process.env.APPLE_CERTIFICATE || '').trim();
    const appleCertificatePassword = (process.env.APPLE_CERTIFICATE_PASSWORD || '').trim();
    const appleApiIssuer = (process.env.APPLE_API_ISSUER || '').trim();
    const appleApiKey = (process.env.APPLE_API_KEY || '').trim();
    const appleApiKeyPath = expandHomePath((process.env.APPLE_API_KEY_PATH || '').trim());
    const appleId = (process.env.APPLE_ID || '').trim();
    const applePassword = (process.env.APPLE_PASSWORD || '').trim();
    const appleTeamId = (process.env.APPLE_TEAM_ID || '').trim();
    const appleApiModeConfigured = Boolean(appleApiIssuer && appleApiKey && appleApiKeyPath);
    const appleIdModeConfigured = Boolean(appleId && applePassword && appleTeamId);
    const appleNotaryConfigured = appleApiModeConfigured || appleIdModeConfigured;
    const preferredNotaryMode = appleId || applePassword || appleTeamId ? 'apple-id' : 'api-key';
    const appleApiKeyFileExists = appleApiKeyPath ? Boolean(await stat(appleApiKeyPath).catch(() => null)) : false;
    const codeSigningIdentities = darwinTargetsSelected ? await readCodeSigningIdentities() : [];
    const xcodeLicenseAccepted = hasXcodebuild ? await checkXcodeLicenseAccepted() : false;
    const missingRustTargets = [];
    let needsCargoXwin = false;
    let needsMakensis = false;
    let needsLlvmRc = false;
    let needsXcodeLicense = false;
    let needsAppleSigningIdentity = false;
    let needsAppleDeveloperId = false;
    let needsAppleNotaryCredentials = false;

    if (!hasXcrun) {
        missing.push('未找到 xcrun，请安装 Xcode 与 Xcode Command Line Tools');
    }
    if (!hasRustup) {
        missing.push('未找到 rustup，请先安装 Rust toolchain');
    }
    if (hasXcodebuild && !xcodeLicenseAccepted) {
        needsXcodeLicense = true;
        missing.push('尚未同意 Xcode 许可协议，请先执行 sudo xcodebuild -license accept');
    }
    if (darwinTargetsSelected) {
        if (!appleSigningIdentity) {
            needsAppleSigningIdentity = true;
            missing.push('未设置 APPLE_SIGNING_IDENTITY，无法对 macOS App 做 Apple 代码签名');
        } else if (appleSigningIdentity === '-') {
            needsAppleDeveloperId = true;
            missing.push('APPLE_SIGNING_IDENTITY 当前为 ad-hoc 签名（-），无法满足“安装不提示警告”的正式分发要求');
        } else if (appleSigningIdentity.startsWith('Apple Development')) {
            needsAppleDeveloperId = true;
            missing.push('APPLE_SIGNING_IDENTITY 当前是 Apple Development 证书，不能替代 Developer ID Application 正式分发证书');
        }

        if (appleCertificate && !appleCertificatePassword) {
            needsAppleDeveloperId = true;
            missing.push('已设置 APPLE_CERTIFICATE，但缺少 APPLE_CERTIFICATE_PASSWORD');
        }

        if (!appleCertificate) {
            if (codeSigningIdentities.length === 0) {
                needsAppleDeveloperId = true;
                missing.push('钥匙串中未找到任何有效的 codesign identity（security find-identity -v -p codesigning 显示 0 valid identities found）');
            } else if (appleSigningIdentity && !codeSigningIdentities.includes(appleSigningIdentity)) {
                needsAppleDeveloperId = true;
                missing.push(`APPLE_SIGNING_IDENTITY 未出现在当前钥匙串可用身份中：${appleSigningIdentity}`);
            }
        }

        if (!appleNotaryConfigured) {
            needsAppleNotaryCredentials = true;
            missing.push('未配置 Apple notarization 凭据，请设置 APPLE_API_* 或 APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID');
        } else if (appleApiIssuer || appleApiKey || appleApiKeyPath) {
            if (!appleApiIssuer || !appleApiKey || !appleApiKeyPath) {
                needsAppleNotaryCredentials = true;
                missing.push('APPLE_API_ISSUER / APPLE_API_KEY / APPLE_API_KEY_PATH 需要同时设置');
            } else if (!appleApiKeyFileExists) {
                needsAppleNotaryCredentials = true;
                missing.push(`APPLE_API_KEY_PATH 指向的文件不存在：${appleApiKeyPath}`);
            }
        } else if (appleId || applePassword || appleTeamId) {
            if (!appleId || !applePassword || !appleTeamId) {
                needsAppleNotaryCredentials = true;
                missing.push('APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID 需要同时设置');
            }
        }
    }

    const installedTargets = new Set();
    if (hasRustup) {
        const {stdout} = await captureCommand(resolveBin('rustup'), ['target', 'list', '--installed'], {
            cwd: repoRoot,
            label: 'rustup target list --installed',
        });
        stdout
            .split(/\r?\n/)
            .map((line) => line.trim())
            .filter(Boolean)
            .forEach((target) => installedTargets.add(target));
    }

    for (const target of selectedTargets) {
        if (!installedTargets.has(target.rustTarget)) {
            missingRustTargets.push(target.rustTarget);
            missing.push(`缺少 Rust target: ${target.rustTarget}`);
        }
    }

    if (selectedTargets.some((target) => target.family === 'windows')) {
        if (!(await commandExists('cargo-xwin', commandEnv))) {
            needsCargoXwin = true;
            missing.push('未找到 cargo-xwin，请执行: cargo install --locked cargo-xwin');
        }
        if (!(await commandExists('makensis', commandEnv))) {
            needsMakensis = true;
            missing.push('未找到 makensis，请执行: brew install nsis');
        }
        if (!(await commandExists('llvm-rc', commandEnv))) {
            needsLlvmRc = true;
            missing.push('未找到 llvm-rc，请执行: brew install llvm，并确保 llvm/bin 在 PATH 中');
        }
    }

    if (missing.length > 0) {
        const installHints = buildInstallHints({
            hasBrew,
            hasRustup,
            missingRustTargets,
            needsCargoXwin,
            needsLlvmRc,
            needsMakensis,
                    needsXcrun: !hasXcrun,
            needsXcodeLicense,
            needsAppleSigningIdentity,
            needsAppleDeveloperId,
            needsAppleNotaryCredentials,
            preferredNotaryMode,
        });
        const sections = [`发布环境检查失败：\n- ${missing.join('\n- ')}`];

        if (installHints.commands.length > 0) {
            sections.push(`可直接执行的修复命令：\n${formatNumberedCommands(installHints.commands)}`);
        }

        if (installHints.notes.length > 0) {
            sections.push(`补充说明：\n- ${installHints.notes.join('\n- ')}`);
        }

        throw new Error(sections.join('\n\n'));
    }

    log('- 发布环境检查通过');
};

const buildTarget = async (target) => {
    divider();
    log(`开始构建 ${target.label} (${target.targetKey})`);
    log(`- 使用 Tauri 平台配置: ${target.configPath}`);
    if (internalDevtoolsEnabled) {
        log('- 内部调试包: 已启用 Tauri DevTools，应用启动时会自动打开');
    }
    if (target.family === 'darwin') {
        log(`- macOS bundles: ${macBundles.join(',')}`);
    }
    await cleanTargetBundleArtifacts(target);

    const buildArgs = [
        '-F',
        'datavisool-app',
        'exec',
        'tauri',
        'build',
        '--config',
        target.configPath,
        '--target',
        target.rustTarget,
    ];
    if (internalDevtoolsEnabled) {
        buildArgs.push('--features', 'internal-devtools');
    }
    if (target.family === 'darwin') {
        buildArgs.push('--bundles', macBundles.join(','));
    }
    if (target.runner) {
        buildArgs.push('--runner', target.runner);
    }

    try {
        await runTauriBuildWithRetries(target, buildArgs);
    } catch (error) {
        if (target.family === 'darwin' && macBundles.includes('dmg')) {
            const message = error instanceof Error ? error.message : String(error);
            throw new Error(`${message}\n\n提示：若 DMG 在受限环境中构建失败，可改用 --mac-bundles app，或在 .env.release 中设置 DATAVISOOL_MAC_BUNDLES=app。`);
        }
        throw error;
    }

    const result = await readBuildResultFromBundle(target);
    log(`- Bundle 目录: ${result.bundleDir}`);
    for (const artifact of result.artifacts) log(`- 构建产物: ${artifact}`);
    return result;
};

const stageTargetArtifacts = async (buildResult) => {
    const releaseDir = path.join(stageRoot, 'releases', version);
    await ensureDir(releaseDir);
    const stagedArtifacts = [];
    for (const artifact of buildResult.artifacts) {
        const name = chooseReleaseAssetName(artifact, buildResult.targetKey, usedReleaseNames);
        const destination = path.join(releaseDir, name);
        if (artifact.endsWith('.app')) {
            await runCommand(resolveBin('ditto'), [artifact, destination], {cwd: repoRoot, label: '复制 macOS 应用包'});
        } else {
            await cp(artifact, destination);
        }
        stagedArtifacts.push(destination);
    }
    return {...buildResult, stagedArtifacts};
};

  return {readBuildResultFromBundle, runPreflightChecks, buildTarget, stageTargetArtifacts};
}
