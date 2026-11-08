# 包结构迁移说明

简体中文 | [English](../en/package-migration.md)

## 适用场景

本文面向使用旧 LCP 包开发过自定义组件、需要将组件源码迁移到 DataVisool SDK 和 UI 并重新构建的开发者。若只需了解如何导入 LCP 项目 ZIP，以及哪些旧服务器组件可在导入时自动转换，请参阅 [从 LCP 迁移项目](./migration.md)。

目录、包名、命令和环境变量的 DataVisool 重命名影响，另见 [重命名破坏性变更与迁移](./datavisool-rename-breaking-changes.md)。

主要公共包收敛为 SDK 和 UI。`lcp-interface`、`lcp-util` 已移除，组件应直接使用 SDK 和 UI。

| 包 | 职责 | 依赖方向 |
| --- | --- | --- |
| `@datavisool/sdk` | 数据契约、组件契约、公共生命周期、渲染与共享工具 | React |
| `@datavisool/ui` | 通用 UI、配置表单、代码编辑器、错误展示 | React、CodeMirror |

原 `datavisool-sdk` 和 `datavisool-ui` 的使用者需要将依赖及导入分别改为 `@datavisool/sdk` 和 `@datavisool/ui`，子入口路径保持不变。

应用负责网络数据加载、蓝图执行与内置配置面板。`SafeEvalUtil` 移回应用，它执行表达式，不是安全沙箱。

## 破坏性改动：旧远程组件的运行时变化

`lcp-interface` 和 `lcp-util` 已移除，宿主也不再提供 `LightChaserInterface` 和 `LightChaserUtil` 全局对象。依赖这些全局对象的已构建远程组件无法直接在新宿主中正常运行，包括已上传的组件。

导入 LCP 项目包时，符合条件的旧服务器组件可在导入阶段静态转换；具体范围见 [从 LCP 迁移项目](./migration.md)。其他旧组件需要将源码中的旧包导入按下表迁移到 SDK 或 UI，使用新版 CLI 重新构建，然后重新上传组件。项目内置组件已迁移到 SDK。

## 导入迁移

| 原入口 | 新入口 |
| --- | --- |
| `lcp-types` 和 `lcp-types/types/*.ts` | `@datavisool/sdk/types` |
| types 中的 `MenuInfo`、`ICategorize`、`MenuToConfigMappingType` | `@datavisool/sdk/core` |
| `lcp-interface` 的控制器和定义 | `@datavisool/sdk/core` 或 `@datavisool/sdk` |
| `lcp-interface` 的内置配置面板 | `@datavisool/sdk/host` 或 `@datavisool/sdk` |
| util 的 `ObjectUtil`、`IdGenerate`、Base64、颜色、文件与 URL 工具 | `@datavisool/sdk/utils` |
| util 的 `ComponentUtil` | `@datavisool/sdk/react` |
| util 的错误边界与错误提示 | `@datavisool/ui/feedback` |
| `@puyinzhen/lcp-ui`、`@zhensq/lcp-ui` | `@datavisool/ui` |

UI 保留原根入口，增加 `/basic`、`/schema`、`/code-editor`、`/feedback`。`/basic` 与 `/feedback` 不加载代码编辑器；`/schema` 的默认控件表仍包含代码编辑器。样式入口为 `@datavisool/ui/style.css`。

## 运行时与兼容范围

CLI 将 SDK、类型和 UI 入口分别映射到 `window.DataVisoolSDK`、`window.DataVisoolTypes` 和 `window.DataVisoolUI`。宿主不再提供旧组件运行时名称，CLI 也不再映射旧包；未能在项目包导入时转换的旧服务器组件需迁移源码并重新构建。新组件不支持在旧宿主运行；范围和限制见 [从 LCP 迁移项目](./migration.md)。

以下属于源码迁移：

- `lcp-types` 已移除；数据类型和枚举改从 `@datavisool/sdk/types` 导入，React 菜单类型从 `@datavisool/sdk/core` 导入。
- 原 `SafeEvalUtil` 深层路径不属于公共包 API。
- 源码目录深层导入不再支持；这些源码原本也不包含在只发布 `dist` 的包中。
- 原 npm 包名的历史版本保持独立；这里不删除 npm 版本，也不保证新 SDK 组件可以在旧宿主中运行。

SDK 内置配置面板由编辑器注册。纯预览页面仅提供运行组件所需的宿主能力；独立开发脚手架显示宿主配置面板说明，实际配置效果在编辑器验证。

## 构建与检查

```sh
pnpm install --frozen-lockfile
pnpm run build:packages
pnpm run check:packages
pnpm run test:package-artifacts
pnpm -F datavisool-app exec tsc --noEmit
pnpm -F datavisool-app test
```

边界检查阻止公共包反向依赖应用，并阻止应用恢复源码深层导入。产物检查将实际打包文件解压到临时消费者项目，检查入口和声明、编译脚手架、构建远程组件，并检查 UI 入口隔离。

发布依赖顺序为 SDK 与 UI → CLI。新脚手架要求 SDK 0.2.0；SDK 不依赖 UI。本次代码改造不自动发布 npm 包。
