# DataVisool 重命名：破坏性变更与迁移

本文记录此前的重命名；当前 npm 包名已改为 `@datavisool/sdk`、`@datavisool/ui` 和 `@datavisool/interaction-engine`。前两个包的迁移说明见[包结构迁移说明](./package-migration.md)；交互引擎的包名常量也已改为新包名。

简体中文 | [English](../en/datavisool-rename-breaking-changes.md)

## 适用场景

本文面向维护旧 LCP 源码、组件或构建脚本，需要了解其改名为 DataVisool 时发生的接口、路径和配置变化的开发者。本文记录提交 `15d9dd3`（`feat: 重命名项目名称`）相对于其父提交的影响；这里的“旧版”指该提交之前的代码。具体项目 ZIP 导入范围见 [从 LCP 迁移项目](./migration.md)，包依赖迁移见 [包结构迁移说明](./package-migration.md)。

组件运行时随后切换到新名称；旧服务器组件运行时兼容入口现已移除，符合条件的组件可在项目包导入时静态转换。当前行为见 [从 LCP 迁移项目](./migration.md)；本文有关该提交时的运行时描述保留作为历史记录。

应用身份在该提交之后改为 `com.datavisool.desktop`，产品名为 `DataVisool`，可执行文件为 `datavisool`，数据库文件为 `db/datavisool.sqlite`。新应用使用独立数据目录，不自动复制旧数据库、资源或设置；旧项目通过旧应用导出后在新应用导入。打包安装、各平台并存和完整旧项目包导入仍需实机验证。

本次改动涉及目录、包、命令、环境变量和构建产物。**不能只改文件夹名，也不能将旧前端和新 Rust 后端混用。** 以下兼容说明以当前代码和已执行的测试为依据，不表示所有旧版本、组件及安装升级路径都经过验证。

## 1. 仓库目录和工作区

| 旧目录 | 新目录 |
| --- | --- |
| `lcp-app/` | `datavisool-app/` |
| `lcp-cli/` | `datavisool-cli/` |
| `lcp-sdk/` | `datavisool-sdk/` |
| `lcp-ui/` | `datavisool-ui/` |
| `lcp-interaction-engine/` | `datavisool-interaction-engine/` |
| `crates/lcp-archive/` | `crates/datavisool-archive/` |
| `crates/lcp-asset/` | `crates/datavisool-asset/` |
| `crates/lcp-codec/` | `crates/datavisool-codec/` |
| `crates/lcp-datasource/` | `crates/datavisool-datasource/` |
| `crates/lcp-db/` | `crates/datavisool-db/` |
| `crates/lcp-fs/` | `crates/datavisool-fs/` |
| `crates/lcp-project-format/` | `crates/datavisool-project-format/` |

**影响：** 使用旧路径的脚本、IDE 运行配置、CI 工作目录、文件监控、资源复制和打包流程会找不到文件；已安装的工作区依赖链接也需要刷新。更新路径后，在仓库根目录执行：

```sh
pnpm install --frozen-lockfile
pnpm run build:packages
pnpm -F datavisool-app exec tsc --noEmit
```

根工作区名称由 `light-chaser-pro` 改为 `datavisool`。`pnpm-workspace.yaml`、`pnpm-lock.yaml` 和 Rust workspace 配置已同步；外部脚本中使用的工作区名称或旧锁文件应一并更新。旧构建目录中的缓存和产物不能视为新包的有效构建产物。

本次还移动了内部文件：`LcpUITheme.less` 改为 `DataVisoolUITheme.less`，`LCPUIContext.tsx` 改为 `DataVisoolUIContext.tsx`；相关 UI 隔离测试文件也改用 `datavisool-ui-` 前缀。直接引用旧源码路径的外部工具需要调整。导入、导出和桌面发布脚本已拆分为多个文件；指向原文件内部行号或实现位置的工具、文档和断点可能失效。

## 2. npm 包、导入和 CLI

| 旧包或命令 | 新包或命令 | 影响 |
| --- | --- | --- |
| `lcp-app` | `datavisool-app` | workspace 过滤器和脚本需更新；应用包仍为私有包。 |
| `lcp-sdk` | `datavisool-sdk` | 依赖声明、导入和脚手架模板需更新，子入口保持相应名称。 |
| `@zhensq/lcp-ui` | `datavisool-ui` | 由带作用域包名改为无作用域包名，依赖和导入必须更新。 |
| `lcp-interaction-engine` | `datavisool-interaction-engine` | 依赖声明、导入和 workspace 过滤器需更新。 |
| `lcp-cli` | `datavisool-cli` | npm 包名及可执行命令均改变。 |

例如：

```ts
// 旧
import {DesignerScene} from 'lcp-sdk/types';
import {Button} from '@zhensq/lcp-ui/basic';

// 新
import {DesignerScene} from 'datavisool-sdk/types';
import {Button} from 'datavisool-ui/basic';
```

新 SDK 的 `/core`、`/utils`、`/react`、`/host`、`/types` 和 UI 的 `/basic`、`/schema`、`/code-editor`、`/feedback`、`/style.css` 入口需要使用新包名前缀。旧依赖不会因为仓库重命名而自动替换；已有 npm 历史包和新包是不同名称，版本号相同也不代表包名可以互换。此前 `lcp-interface`、`lcp-util`、`lcp-types` 的迁移限制仍适用，详见 [包结构迁移说明](./package-migration.md)。

旧的 `lcp-cli` 命令及旧模板中的 `lcp-cli dev/build` 不会自动变成新命令。应安装或使用可获得的 `datavisool-cli`，并将模板脚本改为 `datavisool-cli dev`、`datavisool-cli build`。本次仅修改仓库，**未执行 npm 发布**；公开 registry 中是否能安装新包及其版本，应在实际发布时核实。

根脚本 `pnpm release:lcp-ui` 改为 `pnpm release:datavisool-ui`；应用内的 `prepare:lcp-ui` 和 `prepare:lcp-interaction-engine` 分别改为 `prepare:datavisool-ui`、`prepare:datavisool-interaction-engine`。依赖这些脚本名或 `pnpm -F lcp-*` 的 CI 应更新。

UI 主构建文件从 `dist/lcp-ui.js` 改为 `dist/datavisool-ui.js`；交互引擎主构建文件从 `dist/lcp-interaction-engine.js` 改为 `dist/datavisool-interaction-engine.js`。直接引用这些物理文件、固定 tarball 路径或文件名的流程需要调整；优先使用包公开入口。

交互引擎常量的值由 `lcp-interaction-engine` 改成 `datavisool-interaction-engine`；若代码比较或存储其字符串值，应同步更新。该常量随后又从 `LCP_INTERACTION_ENGINE_PACKAGE` 改名为 `DATAVISOOL_INTERACTION_ENGINE_PACKAGE`，旧名称不再导出，外部调用需更新导入名称。

## 3. Rust crate 与 Tauri 命令

七个 Rust crate 的包名按第 1 节改为 `datavisool-*`；Rust 源码中的库标识符相应使用下划线，例如 `lcp_db` → `datavisool_db`、`lcp_project_format` → `datavisool_project_format`。外部 Cargo 依赖、路径、`use`、构建命令和锁文件都需要更新：

```toml
# 旧：lcp-db = { path = ".../crates/lcp-db" }
datavisool-db = { path = ".../crates/datavisool-db" }
```

Tauri 注册的命令名也发生变化。任何直接使用 `invoke` 的旧前端、测试或外部集成需要与后端一起升级：

| 旧命令 | 新命令 |
| --- | --- |
| `lcp_http_request` | `datavisool_http_request` |
| `lcp_register_http_request` | `datavisool_register_http_request` |
| `lcp_cancel_http_request` | `datavisool_cancel_http_request` |
| `lcp_unregister_http_request` | `datavisool_unregister_http_request` |
| `lcp_complete_frontend_effect` | `datavisool_complete_frontend_effect` |
| `lcp_open_devtools` | `datavisool_open_devtools` |

当前代码注册新命令，未注册上述旧命令别名。旧前端调用新后端时会遇到命令不存在；项目内调用已同步更名。`datavisool_open_devtools` 仍受原有构建配置约束。

## 4. 环境变量

以下是该次重命名涉及的变量映射；后续代码可能已移除部分功能或变量，实际可用配置以当前源码和 [构建说明](./build.md) 为准。需要继续使用的配置应在本地环境、CI 密钥配置和发布配置中更新名称；**值按原有安全方式保存，不要提交到仓库**。

| 旧变量 | 新变量 |
| --- | --- |
| `LCP_RELEASE_NOTES_FILE` | `DATAVISOOL_RELEASE_NOTES_FILE` |
| `LCP_RELEASE_TARGETS` | `DATAVISOOL_RELEASE_TARGETS` |
| `LCP_MAC_BUNDLES` | `DATAVISOOL_MAC_BUNDLES` |
| `LCP_INTERNAL_DEVTOOLS` | `DATAVISOOL_INTERNAL_DEVTOOLS` |
| `LCP_ALLOW_INTERNAL_REMOTE` | `DATAVISOOL_ALLOW_INTERNAL_REMOTE` |
| `LCP_DEPLOY_HOST` | `DATAVISOOL_DEPLOY_HOST` |
| `LCP_DEPLOY_USER` | `DATAVISOOL_DEPLOY_USER` |
| `LCP_DEPLOY_PORT` | `DATAVISOOL_DEPLOY_PORT` |
| `LCP_DEPLOY_ROOT_DIR` | `DATAVISOOL_DEPLOY_ROOT_DIR` |
| `LCP_DEPLOY_SSH_KEY_PATH` | `DATAVISOOL_DEPLOY_SSH_KEY_PATH` |
| `LCP_DEPLOY_BATCH_MODE` | `DATAVISOOL_DEPLOY_BATCH_MODE` |
| `LCP_NOTARY_RETRY_ATTEMPTS` | `DATAVISOOL_NOTARY_RETRY_ATTEMPTS` |
| `LCP_NOTARY_RETRY_DELAY_MS` | `DATAVISOOL_NOTARY_RETRY_DELAY_MS` |
| `LCP_AGENT_WORKSPACE_FILE_TOOLS` | `DATAVISOOL_AGENT_WORKSPACE_FILE_TOOLS` |
| `LCP_ORACLE_URL` / `LCP_ORACLE_USERNAME` / `LCP_ORACLE_PASSWORD` | `DATAVISOOL_ORACLE_URL` / `DATAVISOOL_ORACLE_USERNAME` / `DATAVISOOL_ORACLE_PASSWORD` |
| `LCP_SQLSERVER_TEST_HOST` / `PORT` / `DATABASE` / `USERNAME` / `PASSWORD` / `URL` | 对应的 `DATAVISOOL_SQLSERVER_TEST_HOST` / `PORT` / `DATABASE` / `USERNAME` / `PASSWORD` / `URL`，每项都带完整前缀 |

Oracle 和 SQL Server 变量用于需要外部数据库的测试。内部子进程测试标记 `LCP_DATASOURCE_READONLY_STACK_TEST_CHILD` 也改为 `DATAVISOOL_DATASOURCE_READONLY_STACK_TEST_CHILD`。

**需要特别注意：** 若此前使用 `LCP_AGENT_WORKSPACE_FILE_TOOLS=0`、`false` 或 `off` 关闭工作区文件工具，旧变量不再生效；未设置新变量时该开关默认为启用。要保持关闭，应设置 `DATAVISOOL_AGENT_WORKSPACE_FILE_TOOLS=0`。该次重命名后的发布脚本不再识别旧的 `LCP_DEPLOY_*`、内部构建或目标变量；后续发布脚本支持的选项以当前代码为准。

发布脚本现在读取 `datavisool-app/.env.release`，模板为 `datavisool-app/.env.release.example`。旧文件如果仍在旧路径，需要手动迁移至新路径并更新键名；如果随目录移动，也仍需更新键名。该实际配置文件被 `.gitignore` 忽略，模板不应填入真实凭据。`TAURI_SIGNING_PRIVATE_KEY`、`APPLE_*` 等第三方工具变量不属于上述重命名。

## 5. 当时的兼容标记与边界（历史记录）

- 当时宿主提供 `window.LCPSDK`、`window.LCPTypes`、`window.LCPUI`，CLI 映射旧包入口；这些旧服务器组件兼容入口现已移除。
- 当时已构建组件若依赖此前已移除的 `LightChaserInterface`、`LightChaserUtil`，需要按 [包结构迁移说明](./package-migration.md) 修改源码并重新构建。新宿主加载一个通过 SDK externals 构建的组件已通过测试；现在符合条件的组件也可在项目包导入时静态转换，未对所有历史组件逐一验证。
- 项目导出格式标记仍为 `light-chaser-project`，压缩标记仍为 `LCPZ1`。导入导出相关测试通过，但这不等于所有历史文件均已验证；不要只为统一名称而直接替换这些标记。
- Tauri 的 `identifier` 仍为 `lcpdesigner`，`productName` 和 Rust 二进制名仍为 `LCP`；本次未迁移应用身份或主动删除本地数据。项目数据和资源的跨版本安装、升级及恢复仍需在目标平台实测。
- 已有的 CSS 变量、部分公开 API 和存储键仍包含旧名称；它们不因目录或包名变化而自动更名。之后调整应单独评估兼容性。

## 6. 本次验证及未验证范围

已执行前端类型检查、应用 402 项测试、交互引擎 159 项测试、Rust 桌面端 552 项测试、Rust 基础库测试，以及包边界和实际打包产物检查；基础库 2 项依赖外部数据库的测试被忽略。脚手架的实际打包检查包含模板类型检查和组件构建。

本次未发布 npm 包，也未实测重新打包的桌面应用、各平台安装升级、真实外部数据库或远程发布。发布脚本的 `--help` 和环境自检入口已检查；完整签名、公证及上传流程未执行。

## 7. UI 公共接口和样式命名迁移

直接使用新 `datavisool-ui` 的源码需同步更新：`LCPUIProvider`、`LCPUIContext`、`useLCPUI` 和相关 `LCPUI*` 类型分别改为 `DataVisoolUIProvider`、`DataVisoolUIContext`、`useDataVisoolUI` 和 `DataVisoolUI*`；`LCPStyleProps`、`LCPControlStyleProps` 分别改为 `DataVisoolStyleProps`、`DataVisoolControlStyleProps`。这些旧名称不再从新 UI 包导出。

新 UI 的 CSS 类名、`--lcp-*` 变量和 `lcp-ui.*` 样式层分别改用 `datavisool-*`、`--datavisool-*` 和 `datavisool-ui.*`；CodeMirror 主题 `lcp-dark` 改为 `datavisool-dark`。直接覆盖这些选择器、变量、样式层或查询这些 DOM 类名的外部代码需要同步更改。原有样式变量不会自动影响新 UI。

旧服务器组件的 UI、全局名称、注册标记及 CLI 映射已移除；符合条件的旧组件可在项目包导入时静态转换，其他组件需要迁移源码，使用新 CLI 重新构建并上传。过渡期将旧 UI API 写死在 `DataVisoolUI` 全局上的组件也需要更新。直接消费新 `datavisool-ui` 的外部应用需要更新源码。

旧项目包和 `LCPZ1` 压缩数据仅在导入时识别和转换；项目包可以导入不代表其中引用的旧服务器组件能够运行。新组件仍不支持在旧主程序中运行。

## 8. 工作区临时文件标记

原子写入产生的临时文件由 `.<文件名>.lcp-tmp-<进程号>-<序号>` 改为 `.<文件名>.datavisool-tmp-<进程号>-<序号>`。包含新标记的路径现在被保留，不能作为普通工作区路径使用；工作区索引会过滤符合规则的隐藏临时文件。

当前工作区仅识别和清理新临时文件标记，不迁移旧工作区文件。

## 9. Agent 历史记录标记

新生成的 Agent 完整历史使用 `datavisool-agent-history/v1`，依赖旧 `lcp-agent-history/v1` 标记筛选日志或解析 JSON 的外部工具需要更新。查看器仍可按 `messages` 数组读取原程序生成的完整旧记录；没有 `messages` 数组的旧标记记录将不再仅凭标记被接受。查看器沿用原有的宽松读取行为，并非严格校验历史格式。

## 10. 语言设置与页面标记

语言设置的本地存储键由 `lc-i18n-locale` 改为 `datavisool-i18n-locale`；不读取或迁移旧键。新键没有有效值时，程序根据系统语言选择中文或英文，不支持的语言默认中文。旧值不会被删除，用户可在设置中重新选择语言。

语言变化事件改为 `datavisool-i18n-locale-change`，全局接口改为 `window.datavisoolI18n`；页面翻译的 `data-lc-i18n-*` 属性改为 `data-datavisool-i18n-*`。依赖旧事件、全局接口或页面属性的外部代码需要同步更新；旧服务器组件的转换范围见 [从 LCP 迁移项目](./migration.md)。

## 11. 移除容器部署配置

桌面项目中的 `datavisool-app/Dockerfile` 和 `datavisool-app/nginx.conf` 已移除。依赖这两个文件构建镜像或部署前端的外部脚本需要自行调整；桌面应用构建不使用这两个文件。

## 12. 前端性能测试标记

前端性能采集使用的全局标记由 `window.__LCP_PERF_BASELINE__` 改为 `window.__DATAVISOOL_PERF_BASELINE__`。依赖旧标记初始化或读取性能数据的外部脚本需要同步调整。
