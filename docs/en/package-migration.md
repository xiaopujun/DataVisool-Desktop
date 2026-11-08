# Package Structure Migration

[简体中文](../cn/package-migration.md) | English

## When to Use This Guide

This guide is for developers who built custom components with legacy LCP packages and need to migrate their source code to the DataVisool SDK and UI and rebuild them. For importing LCP project ZIPs and learning which legacy server components can be converted automatically during import, see [Migrating Projects from LCP](./migration.md).

For the effects of the DataVisool rename on directories, package names, commands, and environment variables, see [Renaming Breaking Changes and Migration](./datavisool-rename-breaking-changes.md).

The main public packages have been consolidated into the SDK and UI. `lcp-interface` and `lcp-util` have been removed; components should use the SDK and UI directly.

| Package | Responsibility | Dependencies |
| --- | --- | --- |
| `@datavisool/sdk` | Data and component contracts, public lifecycle APIs, rendering, and shared utilities | React |
| `@datavisool/ui` | General UI, configuration forms, code editor, and error displays | React, CodeMirror |

Consumers of `datavisool-sdk` and `datavisool-ui` must update dependencies and imports to `@datavisool/sdk` and `@datavisool/ui`, respectively. Subpath names remain unchanged.

The application handles network data loading, blueprint execution, and built-in configuration panels. `SafeEvalUtil` has moved back into the application; it executes expressions and is not a security sandbox.

## Breaking Change: Legacy Remote Component Runtime

`lcp-interface` and `lcp-util` have been removed, and the host no longer provides the `LightChaserInterface` and `LightChaserUtil` globals. Built remote components that depend on these globals cannot run directly in the new host, including components uploaded previously.

When importing an LCP project package, eligible legacy server components can be converted statically during import; see [Migrating Projects from LCP](./migration.md) for the supported scope. For other legacy components, migrate the source imports to the SDK or UI using the table below, rebuild with the new CLI, and upload the component again. Built-in components have already been migrated to the SDK.

## Import Migration

| Previous entry point | New entry point |
| --- | --- |
| `lcp-types` and `lcp-types/types/*.ts` | `@datavisool/sdk/types` |
| `MenuInfo`, `ICategorize`, and `MenuToConfigMappingType` from types | `@datavisool/sdk/core` |
| Controllers and definitions from `lcp-interface` | `@datavisool/sdk/core` or `@datavisool/sdk` |
| Built-in configuration panels from `lcp-interface` | `@datavisool/sdk/host` or `@datavisool/sdk` |
| `ObjectUtil`, `IdGenerate`, Base64, color, file, and URL utilities from util | `@datavisool/sdk/utils` |
| `ComponentUtil` from util | `@datavisool/sdk/react` |
| Error boundaries and error messages from util | `@datavisool/ui/feedback` |
| `@puyinzhen/lcp-ui`, `@zhensq/lcp-ui` | `@datavisool/ui` |

The UI retains its root entry point and adds `/basic`, `/schema`, `/code-editor`, and `/feedback`. `/basic` and `/feedback` do not load the code editor; the default control registry in `/schema` still includes it. The stylesheet entry point is `@datavisool/ui/style.css`.

## Runtime and Compatibility

The CLI maps the SDK, types, and UI entry points to `window.DataVisoolSDK`, `window.DataVisoolTypes`, and `window.DataVisoolUI`, respectively. The host no longer provides the old component runtime names, and the CLI no longer maps the old packages. Legacy server components that cannot be converted during project package import require source migration and rebuilding. New components are not supported in the old host; see [Migrating Projects from LCP](./migration.md) for the scope and limitations.

Source migration also includes the following:

- `lcp-types` has been removed; import data types and enums from `@datavisool/sdk/types`, and React menu types from `@datavisool/sdk/core`.
- The old deep import path for `SafeEvalUtil` is not a public package API.
- Deep imports into source directories are no longer supported; those source files were not included in packages that published only `dist` either.
- Historical versions under the original npm package names remain separate. This migration does not delete npm versions or guarantee that components using the new SDK can run in the old host.

SDK built-in configuration panels are registered by the editor. The preview-only page provides only the host capabilities required to run components. The standalone development scaffold displays descriptions of host configuration panels; verify their actual behavior in the editor.

## Build and Checks

```sh
pnpm install --frozen-lockfile
pnpm run build:packages
pnpm run check:packages
pnpm run test:package-artifacts
pnpm -F datavisool-app exec tsc --noEmit
pnpm -F datavisool-app test
```

The boundary check prevents public packages from depending on the application and prevents the application from restoring deep source imports. The artifact check extracts the actual packages into a temporary consumer project, checks entry points and declarations, compiles the scaffold, builds a remote component, and checks UI entry-point isolation.

The dependency order for publishing is SDK and UI → CLI. The new scaffold requires SDK 0.2.0; the SDK does not depend on UI. These code changes do not automatically publish npm packages.
