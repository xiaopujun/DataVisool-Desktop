# 构建 DataVisool

简体中文 | [English](../en/build.md)

本文介绍从源码运行和构建 DataVisool 桌面应用，以及使用仓库脚本制作桌面发布包。除非特别说明，所有命令均在仓库根目录执行。

## 准备环境

- 安装 Node.js、pnpm 和 Rust 工具链。仓库目前没有指定统一的 Node.js 或 pnpm 版本。
- 按 [Tauri 环境准备](https://v2.tauri.app/start/prerequisites/)安装当前操作系统所需的依赖。macOS 需要 Xcode Command Line Tools；Windows 需要 Microsoft C++ 构建工具和 WebView2。
- 确保可以访问依赖包来源；首次安装和准备构建工具可能需要网络。

检查工具是否可用：

```bash
node --version
pnpm --version
rustc --version
cargo --version
```

## 安装依赖

```bash
pnpm install --frozen-lockfile
```

该命令安装整个 pnpm workspace 的依赖，并按仓库锁文件解析版本。

桌面应用还需要项目配置的 ripgrep 二进制文件（Sidecar，随应用打包的辅助程序）。开发和构建时会校验所有配置目标的文件及其 SHA-256；若提示缺失或校验失败，运行：

```bash
pnpm -F datavisool-app sidecar:prepare
```

此命令会下载并校验各目标的 ripgrep 文件，需要网络和 `tar` 命令。只检查已有文件可运行 `pnpm -F datavisool-app sidecar:verify`。

## 本地开发

启动桌面应用的开发环境：

```bash
pnpm -F datavisool-app tauri:dev
```

只启动前端开发服务器：

```bash
pnpm -F datavisool-app dev
```

前端服务器不包含 Tauri 桌面后端；需要桌面能力时使用 `tauri:dev`。这些命令会先构建应用依赖的 workspace 包。

## 本地构建

仅构建前端，产物位于 `datavisool-app/dist/`：

```bash
pnpm -F datavisool-app build
```

构建当前机器的桌面应用和安装包：

```bash
pnpm -F datavisool-app tauri:build
```

Tauri 会先校验 Sidecar、构建前端，再编译 Rust 后端并打包；生成的桌面产物通常位于 `datavisool-app/src-tauri/target/release/bundle/`，具体格式和路径以构建输出为准。

如需启用内部调试工具，可使用 `pnpm -F datavisool-app tauri:build:internal`。该构建面向内部调试。

## 检查和测试

按修改范围选择需要的检查：

```bash
# 前端单元和组件测试
pnpm -F datavisool-app test

# 桌面端自动化测试
pnpm -F datavisool-app test:desktop

# 公共包边界与构建产物检查
pnpm run check:packages
pnpm run test:package-artifacts

# 桌面端 Rust 测试
cargo test --manifest-path datavisool-app/src-tauri/Cargo.toml
```

桌面端自动化测试的覆盖范围和使用要求见 [测试说明](../../datavisool-app/tests/desktop/README.md)。其他模块的测试入口见 [文档总览](../README.md#7-测试与验证入口)。

## 组件脚手架

在仓库内查看 CLI 命令并构建组件模板：

```bash
pnpm -F datavisool-cli cli --help
pnpm -F datavisool-cli template:build
```

CLI 构建的组件包包含入口脚本及相关资源，可以上传到应用的服务器组件管理页面。

## 桌面发布包

仓库的 `release:desktop` 脚本仅能在 macOS 主机上运行，默认依次构建 macOS Apple Silicon、macOS Intel 和 Windows x64。它会整理 `.app`、`.dmg` 和 Windows NSIS `.exe` 产物；默认发布目录为 `datavisool-app/dist/desktop-release/releases/<version>/`。

包含 macOS 目标的发布构建需要 Apple Developer ID Application 签名身份及公证凭据；从 macOS 构建 Windows 目标还需要 `cargo-xwin`、LLVM 的 `llvm-rc` 和 NSIS。相关配置示例见 [`datavisool-app/.env.release.example`](../../datavisool-app/.env.release.example)。可以通过环境变量注入配置，也可以按需复制示例为 `datavisool-app/.env.release` 并填写；凭据不要提交到仓库。

准备完成后，先检查所选目标的构建环境：

```bash
pnpm -F datavisool-app release:desktop -- --doctor --targets darwin-aarch64,darwin-x86_64,windows-x86_64
```

按照检查结果补齐工具、Rust 目标和签名配置，再执行：

```bash
# 默认构建三个目标
pnpm -F datavisool-app release:desktop

# 只构建指定目标
pnpm -F datavisool-app release:desktop -- --targets darwin-aarch64

# 中断后尝试复用已有进度和产物；目标和构建选项需与原命令一致
pnpm -F datavisool-app release:desktop -- --resume
```

发布前检查 `datavisool-app/package.json` 和 `datavisool-app/src-tauri/Cargo.toml` 的版本。非 `--resume` 模式会清理暂存目录，并在每个目标重新构建前清理对应的 bundle 目录；需要保留旧产物时，先将其移到其他位置。更多选项可通过 `pnpm -F datavisool-app release:desktop -- --help` 查看。
