# DataVisool 文档总览

本文基于当前仓库目录结构、workspace 配置和已实现源码，对 DataVisool 的工程组成、交付形态、模块职责和验证入口做集中说明。它用于作为 `docs/` 目录的入口文档，帮助开发者先理解项目全局，再进入具体模块。

## 1. 项目定位

DataVisool 是一个面向大屏可视化、交互编排和项目发布的 Monorepo。当前仓库提供桌面端设计器：

- **桌面端设计器**：由 `datavisool-app` 的 React 前端和 `datavisool-app/src-tauri` 的 Tauri 2 后端组成，提供项目管理、设计器编辑、蓝图编排、资源管理、数据源配置、项目导入导出和桌面端发版能力。

项目使用 `pnpm workspace` 管理前端包，`crates/` 为 Rust 基础库工作区；桌面端 Tauri Rust 包位于 `datavisool-app/src-tauri`。

## 2. 顶层目录职责

| 目录 | 当前职责 |
| --- | --- |
| `datavisool-app/` | 主应用。包含设计器前端、Tauri 配置、桌面端发版脚本、Vitest 组件测试和桌面端自动化测试。 |
| `datavisool-ui/` | 自研 React UI 组件库，导出按钮、表单、弹窗、编辑器、布局、反馈、选择器等基础组件和 `style.css`。 |
| `datavisool-interaction-engine/` | 高性能交互引擎，导出拖拽、缩放、旋转、吸附、命中测试、空间索引、选区和 DOM overlay 能力。 |
| `datavisool-sdk/` | 组件定义、控制器公共生命周期、React 渲染与共享工具。 |
| `datavisool-cli/` | 用户侧组件开发脚手架。支持创建模板项目、开发预览、构建远程组件包和本地 npm 打包检查。 |
| `crates/` | 桌面端使用的 Rust 基础库，包括数据库、数据源、归档、编码、资产和项目格式。 |
| `scripts/` | 根目录工程脚本，包括包边界和构建产物检查。 |
| `docs/` | 项目文档入口和后续领域文档承载目录。 |

## 3. 前端 workspace 关系

`pnpm-workspace.yaml` 当前纳入以下包：

```text
datavisool-app
datavisool-ui
datavisool-sdk
datavisool-interaction-engine
datavisool-cli
```

核心依赖方向：

```text
datavisool-app
  -> datavisool-ui
  -> datavisool-interaction-engine
  -> datavisool-sdk

datavisool-sdk -> React
datavisool-ui -> React / ReactDOM peer dependency
datavisool-interaction-engine -> 无 workspace 依赖，独立输出交互引擎能力
```

`datavisool-app` 是主要消费者。`datavisool-ui`、`datavisool-interaction-engine`、`datavisool-sdk` 均以构建后的 `dist/` 作为发布/消费产物。`datavisool-cli` 发布后作为用户侧命令行工具使用。公共包的入口、边界和升级范围见 [包结构迁移说明](./cn/package-migration.md)。

## 4. `datavisool-app` 实现分层

主应用前端入口：

- `src/index.tsx`：设计器/桌面端入口，使用 `createBrowserRouter` 加载主路由。

主要源码分层：

| 路径 | 说明 |
| --- | --- |
| `src/pages/home/` | 首页管理区，包括控制台、项目列表、数据库、API、AI 模型、远程组件、发布、字体、视频、3D 模型、动态脚本、设置等页面。 |
| `src/pages/designer/`、`src/designer/` | 设计器页面与设计器内部模块，包括画布、蓝图、左/右面板、资源、事件、发布、运行时和状态模型。 |
| `src/designer/left/ai-fusion/` | 设计器 AI Fusion 能力，包括自然语言工作流规划、样式/数据优化、组件生成、组件替换、Skill Runtime、Agent Session 和 AI 历史。详见 [桌面端 AI 能力](./桌面端/AI能力/)。 |
| `src/pages/preview/`、`src/pages/release/` | 设计器预览和设计器发布展示入口。 |
| `src/framework/` | 设计器基础框架能力，包括组件容器、控制器抽象、缓存、拖拽缩放、菜单、屏幕适配和数据转换。 |
| `src/comps/` | 内置可视化组件定义与配置，覆盖 ECharts、地图、Antd 风格组件、循环容器、分组图层等。 |
| `src/api/` | 前端 API 抽象、工程 API 和响应处理。 |
| `src/util/` | 应用侧工具函数，包括资产 URL、数据源、导出风险、加密、坐标和平台环境等。 |
| `src-tauri/` | Tauri 2 桌面端配置与 Rust 后端，负责桌面壳和打包。 |
| `scripts/` | 应用侧构建、桌面端发版、updater manifest 生成/合并脚本。 |
| `tests/` | Vitest 组件/单元测试和桌面端自动化测试。 |

当前路由体现出的主要产品功能包括控制台、项目列表、设计器、预览、发布页、数据库管理、API 管理、AI 模型管理、远程组件管理、发布管理、字体管理、视频资源管理、3D 模型资源管理、动态脚本管理和系统设置。

## 5. Rust 基础库

`crates/` 中的 Rust 库由桌面端使用，覆盖数据库、数据源、归档、编码、文件系统、资产和项目格式。

## 6. 构建与运行链路

### 桌面端设计器

```bash
pnpm install
pnpm -F datavisool-app dev
pnpm -F datavisool-app tauri:dev
pnpm -F datavisool-app build
pnpm -F datavisool-app tauri:build
```

`datavisool-app/src-tauri/tauri.conf.json` 配置无边框主窗口、图标和前端构建产物目录。应用内更新功能已移除；发布脚本仍可单独生成签名产物和 manifest，须显式指定发布资源地址。

### CLI 脚手架

```bash
pnpm -F datavisool-cli cli --help
pnpm -F datavisool-cli cli create demo /private/tmp/datavisool-cli-demo
pnpm -F datavisool-cli template:build
pnpm -F datavisool-cli pack:local
```

`datavisool-cli` 构建远程组件时将 React、ReactDOM、`@datavisool/sdk` 与 UI 公共入口映射到宿主全局对象，并生成组件入口、静态资产和 zip 包。旧的 interface 和 util 包及其全局对象不再提供。

## 7. 测试与验证入口

按修改范围选择验证命令：

| 修改范围 | 推荐验证 |
| --- | --- |
| `datavisool-app/src/` | `pnpm -F datavisool-app test`，涉及类型或公共组件时补充 `pnpm -F datavisool-app exec tsc --noEmit`。 |
| `datavisool-app` 桌面端自动化 | `pnpm -F datavisool-app test:desktop`；范围和限制见 [桌面端自动化测试说明](../datavisool-app/tests/desktop/README.md)。 |
| 公共包及入口 | `pnpm run build:packages`、`pnpm run check:packages`、`pnpm run test:package-artifacts`。 |
| `datavisool-ui/` | `pnpm -F @datavisool/ui build`，并检查 `datavisool-app` 消费兼容性。 |
| `datavisool-interaction-engine/` | `pnpm -F @datavisool/interaction-engine test`；核心交互改动补充性能基线或验收脚本。 |
| `crates/` | `cargo test --manifest-path crates/Cargo.toml`。 |
| `datavisool-app/src-tauri/` | `cargo test --manifest-path datavisool-app/src-tauri/Cargo.toml`。 |
| `datavisool-cli/` | `pnpm -F datavisool-cli cli --help`、模板构建和 `pnpm -F datavisool-cli pack:local`。 |

## 8. 维护约束

- 修改前应先理解调用链、数据流和已有测试，避免只按局部文件做变更。
- 前端跨包类型、接口、发布协议和构建配置变更，需要同步检查直接消费者。
- `@datavisool/sdk/types` 的类型变更需要特别关注 `datavisool-sdk` 和 `datavisool-app` 的兼容性。
- `datavisool-cli` 的变更需要考虑 npm 发布内容、`bin` 入口、模板项目和外部依赖配置。
- Less 文件中避免使用会被 Less 预计算并触发混合单位错误的 `min()`、`max()`、`clamp()`。
- Tauri 安全基线、asset protocol scope、updater 和发版脚本变更需要同时评估桌面端加载、自动更新影响。

## 9. 阅读建议

新开发者建议按以下顺序理解项目：

1. 先阅读本文件，确认仓库全局结构。
2. 阅读 [中文版 README](../README_CN.md)，了解产品功能和上手方式；开发和发版命令见 [桌面端开发与构建](./cn/build.md)。
   从 LCP 导入项目时，先阅读 [从 LCP 迁移项目](./cn/migration.md)。
3. 涉及 Rust 代码组织、模块拆分或重构时，先阅读 [Rust 代码组织规范](./Rust代码组织规范.md)。
4. 从旧仓库或组件迁移时，先阅读 [重命名破坏性变更与迁移](./cn/datavisool-rename-breaking-changes.md)。
5. 了解组件运行时与旧数据支持范围时，阅读 [从 LCP 迁移项目](./cn/migration.md)。
6. 修改前查找相关测试，修改后按第 7 节选择验证命令。
