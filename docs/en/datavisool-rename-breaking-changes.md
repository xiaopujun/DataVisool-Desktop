# DataVisool Rename: Breaking Changes and Migration

This document describes an earlier rename. The current npm package names are `@datavisool/sdk`, `@datavisool/ui`, and `@datavisool/interaction-engine`. See the [package migration guide](./package-migration.md) for the first two; the interaction engine package-name constant also uses the new name.

[简体中文](../cn/datavisool-rename-breaking-changes.md) | English

## When to Use This Guide

This guide is for developers maintaining legacy LCP source code, components, or build scripts who need to understand the interface, path, and configuration changes introduced by the DataVisool rename. It records the effect of commit `15d9dd3` (`feat: 重命名项目名称`) relative to its parent; “old version” refers to code before that commit. For the current project ZIP import scope, see [Migrating Projects from LCP](./migration.md); for package dependency migration, see [Package Structure Migration](./package-migration.md).

The component runtime subsequently switched to new names, and the legacy server component runtime compatibility entry points have been removed. Eligible components can now be converted statically during project package import. See [Migrating Projects from LCP](./migration.md) for current behavior; references to runtime behavior at the time of the rename remain below as a historical record.

After that commit, the application identifier changed to `com.datavisool.desktop`, the product name to `DataVisool`, the executable to `datavisool`, and the database file to `db/datavisool.sqlite`. The new app uses a separate data directory and does not automatically copy the old database, assets, or settings. Export old projects from the old app and import them into the new one. Packaged installation, side-by-side installation on each platform, and complete legacy package imports still require testing on target machines.

The rename affected directories, packages, commands, environment variables, and build artifacts. **Renaming the folder alone is insufficient, and an old frontend cannot be mixed with the new Rust backend.** Compatibility statements reflect the code and tests described at the time and do not mean that every old version, component, or installation upgrade path has been verified.

## 1. Repository Directories and Workspaces

| Old directory | New directory |
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

**Impact:** Scripts, IDE run configurations, CI working directories, file watchers, asset copying, and packaging processes using old paths will not find the files. Installed workspace dependency links also need to be refreshed. After updating paths, run the following from the repository root:

```sh
pnpm install --frozen-lockfile
pnpm run build:packages
pnpm -F datavisool-app exec tsc --noEmit
```

The root workspace name changed from `light-chaser-pro` to `datavisool`. `pnpm-workspace.yaml`, `pnpm-lock.yaml`, and the Rust workspace configuration were updated; update workspace names and old lockfiles used by external scripts as well. Caches and artifacts in old build directories should not be treated as valid builds of the new packages.

Internal files were moved as well: `LcpUITheme.less` became `DataVisoolUITheme.less`, and `LCPUIContext.tsx` became `DataVisoolUIContext.tsx`. Related UI isolation tests use the `datavisool-ui-` prefix. External tools that refer directly to old source paths need updating. Import, export, and desktop release scripts were split into multiple files; tools, documentation, and breakpoints referring to old line numbers or implementation locations may no longer apply.

## 2. npm Packages, Imports, and CLI

| Old package or command | New package or command | Impact |
| --- | --- | --- |
| `lcp-app` | `datavisool-app` | Update workspace filters and scripts; the app package remains private. |
| `lcp-sdk` | `datavisool-sdk` | Update dependency declarations, imports, and scaffold templates; corresponding sub-entry names remain. |
| `@zhensq/lcp-ui` | `datavisool-ui` | The package changed from a scoped to an unscoped name; update dependencies and imports. |
| `lcp-interaction-engine` | `datavisool-interaction-engine` | Update dependency declarations, imports, and workspace filters. |
| `lcp-cli` | `datavisool-cli` | Both the npm package and executable command names changed. |

For example:

```ts
// Before
import {DesignerScene} from 'lcp-sdk/types';
import {Button} from '@zhensq/lcp-ui/basic';

// After
import {DesignerScene} from 'datavisool-sdk/types';
import {Button} from 'datavisool-ui/basic';
```

The new SDK entries `/core`, `/utils`, `/react`, `/host`, and `/types`, and UI entries `/basic`, `/schema`, `/code-editor`, `/feedback`, and `/style.css` use the new package name prefixes. Old dependencies are not automatically replaced by renaming the repository. Historical npm packages and new packages have different names; matching version numbers do not make their names interchangeable. The earlier migration limits for `lcp-interface`, `lcp-util`, and `lcp-types` still apply; see [Package Structure Migration](./package-migration.md).

The old `lcp-cli` command and `lcp-cli dev/build` scripts in old templates do not automatically change. Install or use an available `datavisool-cli`, and change template scripts to `datavisool-cli dev` and `datavisool-cli build`. The rename changed the repository only; **no npm publication was performed as part of that change**. Verify whether the new packages and versions are available in the public registry when publishing.

The root script `pnpm release:lcp-ui` became `pnpm release:datavisool-ui`. The app scripts `prepare:lcp-ui` and `prepare:lcp-interaction-engine` became `prepare:datavisool-ui` and `prepare:datavisool-interaction-engine`. Update CI that depends on those script names or `pnpm -F lcp-*`.

The UI's main build file changed from `dist/lcp-ui.js` to `dist/datavisool-ui.js`; the interaction engine's changed from `dist/lcp-interaction-engine.js` to `dist/datavisool-interaction-engine.js`. Update workflows that directly reference those files, fixed tarball paths, or filenames; prefer public package entry points.

The interaction engine constant's value changed from `lcp-interaction-engine` to `datavisool-interaction-engine`. Update code that compares or stores that string. The constant was subsequently renamed from `LCP_INTERACTION_ENGINE_PACKAGE` to `DATAVISOOL_INTERACTION_ENGINE_PACKAGE`; the old name is no longer exported, so external imports must change.

## 3. Rust Crates and Tauri Commands

The seven Rust crate package names changed to `datavisool-*` as shown in section 1. Rust library identifiers likewise use underscores, for example `lcp_db` → `datavisool_db` and `lcp_project_format` → `datavisool_project_format`. Update external Cargo dependencies, paths, `use` statements, build commands, and lockfiles:

```toml
# Before: lcp-db = { path = ".../crates/lcp-db" }
datavisool-db = { path = ".../crates/datavisool-db" }
```

Registered Tauri command names also changed. Old frontends, tests, or external integrations that call `invoke` directly must be updated together with the backend:

| Old command | New command |
| --- | --- |
| `lcp_http_request` | `datavisool_http_request` |
| `lcp_register_http_request` | `datavisool_register_http_request` |
| `lcp_cancel_http_request` | `datavisool_cancel_http_request` |
| `lcp_unregister_http_request` | `datavisool_unregister_http_request` |
| `lcp_complete_frontend_effect` | `datavisool_complete_frontend_effect` |
| `lcp_open_devtools` | `datavisool_open_devtools` |

Current code registers the new commands and does not register the old aliases above. An old frontend calling the new backend will encounter missing commands; in-repository callers were renamed. `datavisool_open_devtools` remains subject to the existing build configuration.

## 4. Environment Variables

The following table records variable mappings involved in the rename. Later code may have removed some features or variables; consult the current source and [build guide](./build.md) for available configuration. Rename settings still in use in local environments, CI secrets, and release configuration. **Keep values in your existing secure storage and do not commit them.**

| Old variable | New variable |
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
| `LCP_SQLSERVER_TEST_HOST` / `PORT` / `DATABASE` / `USERNAME` / `PASSWORD` / `URL` | Corresponding `DATAVISOOL_SQLSERVER_TEST_HOST` / `PORT` / `DATABASE` / `USERNAME` / `PASSWORD` / `URL`; each variable uses the full prefix. |

Oracle and SQL Server variables are used by tests requiring external databases. The internal subprocess test marker `LCP_DATASOURCE_READONLY_STACK_TEST_CHILD` also changed to `DATAVISOOL_DATASOURCE_READONLY_STACK_TEST_CHILD`.

**Pay particular attention:** If you previously disabled workspace file tools with `LCP_AGENT_WORKSPACE_FILE_TOOLS=0`, `false`, or `off`, the old variable no longer takes effect. The new switch defaults to enabled when unset; set `DATAVISOOL_AGENT_WORKSPACE_FILE_TOOLS=0` to keep it disabled. The release script after the rename no longer recognized the old `LCP_DEPLOY_*`, internal build, or target variables. Consult current code for options supported by later release scripts.

The release script now reads `datavisool-app/.env.release`, with `datavisool-app/.env.release.example` as the template. If an old file remains at the old path, move it manually and update its keys; if it moved with the directory, update the keys. The actual configuration file is ignored by `.gitignore`; do not put real credentials in the template. Third-party tool variables such as `TAURI_SIGNING_PRIVATE_KEY` and `APPLE_*` were not part of this rename.

## 5. Compatibility Markers and Boundaries at the Time (Historical)

- At the time, the host provided `window.LCPSDK`, `window.LCPTypes`, and `window.LCPUI`, and the CLI mapped old package entry points. These old server component compatibility entry points have since been removed.
- At the time, built components depending on the previously removed `LightChaserInterface` or `LightChaserUtil` needed source changes and rebuilding as described in [Package Structure Migration](./package-migration.md). Loading one component built with SDK externals in the new host was tested successfully. Eligible components can now also be converted statically during project package import; not every historical component has been tested.
- The project export format marker was still `light-chaser-project`, and the compression marker was still `LCPZ1`. Import/export tests passed, but that did not verify every historical file; do not replace these markers merely to make names consistent.
- The Tauri `identifier` was still `lcpdesigner`, and `productName` and the Rust binary name were still `LCP`. The rename did not migrate the app identity or actively delete local data. Cross-version installation, upgrades, and restoration of project data and assets still require testing on target platforms.
- Existing CSS variables, some public APIs, and storage keys still contained old names. They did not automatically change with the directory or package names; later changes required a separate compatibility assessment.

## 6. Validation and Untested Scope at the Time

The rename was checked with frontend type checking, 402 application tests, 159 interaction engine tests, 552 Rust desktop tests, Rust foundational library tests, package boundary checks, and actual package artifact checks. Two foundational library tests requiring external databases were ignored. The scaffold's package artifact check included template type checking and a component build.

No npm packages were published as part of that work. The repackaged desktop application, installation and upgrades across platforms, real external databases, and remote publication were not tested. The release script's `--help` and environment check entry points were checked; the complete signing, notarization, and upload flow was not run.

## 7. UI Public Interface and Style Name Migration

Source code using the new `datavisool-ui` directly must update `LCPUIProvider`, `LCPUIContext`, `useLCPUI`, and related `LCPUI*` types to `DataVisoolUIProvider`, `DataVisoolUIContext`, `useDataVisoolUI`, and `DataVisoolUI*`, respectively. `LCPStyleProps` and `LCPControlStyleProps` became `DataVisoolStyleProps` and `DataVisoolControlStyleProps`. The old names are no longer exported by the new UI package.

The new UI uses `datavisool-*` CSS classes, `--datavisool-*` variables, and `datavisool-ui.*` style layers instead of the old classes, `--lcp-*` variables, and `lcp-ui.*` layers. The CodeMirror theme `lcp-dark` became `datavisool-dark`. Update external code that overrides these selectors, variables, or layers, or queries these DOM class names. Old style variables do not automatically affect the new UI.

Old server component UI and global names, registration markers, and CLI mappings have been removed. Eligible old components can be converted statically during project package import; other components require source migration, rebuilding with the new CLI, and uploading. Components that hard-coded old UI APIs on the `DataVisoolUI` global during the transition also need updating. External applications that directly consume the new `datavisool-ui` must update their source.

Legacy project packages and `LCPZ1` compressed data are recognized and converted only at import. A package being importable does not mean its referenced legacy server components will run. New components remain unsupported in the old main application.

## 8. Workspace Temporary File Marker

Temporary files created by atomic writes changed from `.<filename>.lcp-tmp-<process-id>-<sequence>` to `.<filename>.datavisool-tmp-<process-id>-<sequence>`. Paths containing the new marker are reserved and cannot be used as ordinary workspace paths; the workspace index filters matching hidden temporary files.

The current workspace recognizes and cleans only the new temporary file marker. Old workspace files are not migrated.

## 9. Agent History Marker

Newly generated complete Agent history uses `datavisool-agent-history/v1`. External tools that filter logs or parse JSON using the old `lcp-agent-history/v1` marker need updating. The viewer can still read complete old records containing a `messages` array; old-marker records without a `messages` array are no longer accepted solely on the basis of the marker. The viewer retains its permissive reading behavior and does not strictly validate the history format.

## 10. Language Settings and Page Markers

The local storage key for language settings changed from `lc-i18n-locale` to `datavisool-i18n-locale`; the old key is not read or migrated. When the new key has no valid value, the app selects Chinese or English according to the system language, defaulting to Chinese for unsupported languages. The old value is not deleted, and users can select the language again in settings.

The language-change event became `datavisool-i18n-locale-change`, the global interface became `window.datavisoolI18n`, and page translation attributes changed from `data-lc-i18n-*` to `data-datavisool-i18n-*`. External code relying on the old event, global interface, or page attributes must be updated. See [Migrating Projects from LCP](./migration.md) for the legacy server component conversion scope.

## 11. Removal of Container Deployment Configuration

`datavisool-app/Dockerfile` and `datavisool-app/nginx.conf` were removed from the desktop project. External scripts that use those files to build images or deploy the frontend must be adjusted; desktop application builds do not use them.

## 12. Frontend Performance Test Marker

The global marker used for frontend performance collection changed from `window.__LCP_PERF_BASELINE__` to `window.__DATAVISOOL_PERF_BASELINE__`. External scripts that initialize or read performance data using the old marker must be updated.
