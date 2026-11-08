# Migrating Projects from LCP

[简体中文](../cn/migration.md) | English

## When to Use This Guide

This guide is for users who have created projects in LCP and want to import them into DataVisool for further editing and use, as well as developers who need to handle custom server components in those projects. Migration starts with a project ZIP exported from LCP. For changes to component source dependencies, see [Package Structure Migration](./package-migration.md).

DataVisool is a separate desktop application. Recognized legacy project formats and configurations are converted during import. A successful import means the project data was saved; it does not guarantee that every component, external service, and interaction will work.

This guide describes the scope of the current implementation. Legacy package compatibility is temporary and is planned for removal in a future version; no removal date has been set. After migration, verify the project and export a new backup from DataVisool.

## Choose the Right Path

| What you have | What to do | Limits |
| --- | --- | --- |
| An LCP project ZIP meeting the format requirements below, using mainly supported built-in components | Import it into an empty project and check the result | Automatic import can be attempted, but passing format validation does not guarantee identical canvas, data, or interaction behavior. |
| Custom server components, dynamic scripts, or legacy map configuration | Import and verify; migrate the source and rebuild any components that cannot be converted | Only recognized component interfaces are converted during import. Legacy scripts, styles, maps, and runtime behavior may differ or fail. |
| Shared APIs, databases, external assets, or network services | Check the export and target environment, then test connections after import | Files outside the package, database contents, and external services are not migrated. A shared configuration with the same reference key may be updated. |
| Only an LCP application directory, database, or screenshot, with no exportable ZIP | Export from LCP; if that is impossible, rebuild manually | There is no supported automatic migration entry point for an old application directory. |
| A ZIP with a missing manifest, unsupported format or encryption, or incomplete files | Check the original export and re-export from LCP if needed | The importer rejects packages that do not meet its requirements; exports from every LCP version are not guaranteed to work. |

## Before You Migrate

1. Export the project as a ZIP from LCP. Keep the original project and export package. If there are export warnings, check for missing images, videos, fonts, components, or data sources first.
2. Install DataVisool, create an empty project from the project list, and open it in the editor. Import writes into the current target project; using an empty project avoids overwriting existing project data.
3. If the old package is encrypted, have the passphrase used during export ready. Project packages can contain sensitive information such as API URLs, request headers, and database connection details; store them securely.
4. In the editor's upper-right menu, select “Import project” and choose the ZIP. Leave the passphrase blank for an unencrypted package; whole-package or legacy field-level encryption requires the correct passphrase.
5. After import, check the canvas, assets, data, and interactions. Then export a new project package from DataVisool as a backup.

Import can also add asset records and reuse or update shared API and database configurations based on their reference keys. These may be used by other projects. Back up important existing data first and check the shared data sources after import.

**Effect on the target project:** Import writes project data into the project currently open in the editor; it does not create a completely separate project. Do not try it in an existing project without a backup. For version 3 packages, the entry project is written into the current project, while other packaged project nodes are created as new projects and their hierarchy and recognized references are restored from the package.

## Supported Project Packages

| Case | Current behavior |
| --- | --- |
| Project ZIP exported by LCP | Packages marked `light-chaser-project` with an internal manifest `formatVersion` of 2 or 3 are supported, subject to structure and file validation. Version 2 imports a single project; version 3 restores the project structure and references from the project nodes in the package. |
| DataVisool project ZIP | The same import entry point recognizes the `datavisool-project` format. Unencrypted project manifests currently support versions 2 and 3. |
| Supported whole-package encryption | Packages using `package` mode and the `passphrase-v1` algorithm are supported. After the correct passphrase is entered, the decrypted inner project package must also pass validation. The outer manifest of a whole-package encrypted archive can use version 4; this is distinct from the inner project manifest version. |
| Legacy field-level encryption | Supported formats of encrypted shared API and database fields can be imported with the correct passphrase. |
| Legacy compressed project data | The importer recognizes the `LCPZ1:gzip:` marker, decompresses the data, then converts and saves it. |

This does not promise support for every LCP release. The current importer rejects packages without `manifest.json`, with an unsupported format marker, with an internal manifest version below 2 or above 3, or with missing required files or failed validation. Other encryption algorithms are unsupported. `formatVersion` is the project package format version, not the LCP application version.

**How to check a package initially:** Open the ZIP read-only with an archive tool and inspect `format` and `formatVersion` in the root `manifest.json`; do not modify the archive before import. The manifest of an unencrypted package can be inspected directly. For whole-package encryption, the outer manifest version and the encrypted inner project manifest version are separate concepts, so the outer version alone cannot establish whether the project is supported. The final result depends on decryption and structural and file validation during import.

## Project Content and Assets

| Content | Current behavior and limits |
| --- | --- |
| Canvas, layers, and configuration | The project JSON is imported. Whether it renders correctly depends on whether its components and configurations are still supported. Unrecognized legacy structures are not guaranteed to be repaired automatically. |
| Recognized legacy configuration | Import converts legacy static-data wrappers, maps a loop container's old `gap` to horizontal and vertical spacing, and converts legacy base-table scrolling settings to current carousel settings. Fields with the same names inside user data are not treated as configuration. |
| Images and videos | Assets listed in the package are imported. Legacy local `asset://localhost/` or `http://asset.localhost/` URLs are rewritten to new application URLs when they can be matched to packaged assets. Files absent from the package cannot be restored automatically from the old machine's paths; video playback also depends on the platform's supported codecs. |
| Fonts and 3D models | Fonts and model assets listed in the manifest can be imported. Model entry points and dependencies must be complete. Check font appearance and model rendering after import. |
| Dynamic scripts | Local scripts and remote script URLs listed in the manifest can be imported. Version 3 can restore the project-level ownership recorded in the package. Remote scripts still need to be accessible from the target network; scripts that depend on legacy interfaces are not guaranteed to work. |
| Shared APIs and databases | Records in the package can be imported and recognized project references rewritten. Unmapped references are cleared and reported after import so they can be selected again. A matching reference key may cause an existing configuration to be reused and updated. Only connection configuration is migrated, not data in the external database. Verify connections, permissions, and network access again. |
| Blueprints, subprojects, and screen references | Blueprint data is imported with the project and each interaction must be checked. The version 3 project graph can restore the packaged hierarchy and rewrite recognized screen references; import may fail if a referenced project is missing. Version 2 follows the single-project path and does not provide version 3 project-graph restoration. |
| GIS maps | Complete conversion of legacy map configuration is not guaranteed. In particular, inspect old `sources` objects, `urlTemplate` fields, and inline layer structures after import, and reconfigure them to the current component format if needed. |

Asset URL rewriting applies only to packaged static assets that the importer can match. External URLs do not become files inside the package. Check that each asset displays or plays correctly even when ZIP import succeeds.

## User-Developed Server Components

When importing a project package, DataVisool performs **static conversion** on eligible legacy server components in the package. It analyzes recognized legacy interfaces and registration markers in component scripts and rewrites them to current host entry points; it does not execute the component scripts during import. This applies to eligible user-developed components regardless of whether a similarly named example exists. Both individual `.js` / `.mjs` files and component ZIP packages with an entry script have corresponding handling.

The legacy interface members currently recognized for conversion are:

| Legacy interface | Recognized members |
| --- | --- |
| `LightChaserInterface` | `AbstractDesignerController`, `AbstractDesignerDefinition`, `BaseInfo`, `AnimationConfig`, `ThemeConfig`, `DataConfig`, `FilterConfig` |
| `LightChaserUtil` | `ObjectUtil`, `ComponentUtil` |

The component must use a direct function-call structure recognized by the converter and have a unique legacy component registration location. In a component ZIP, legacy code that needs conversion must be in the selected entry file. If other script files contain legacy code requiring conversion, import fails.

The following cases **are not supported for automatic static conversion**. The importer rejects recognized unsafe or ambiguous usage:

- Using legacy interface members outside the table above, or legacy globals such as `LightChaserUI`, `LCPSDK`, `LCPTypes`, or `LCPUI`.
- Using dynamic properties that cannot be determined statically, dynamically executed code, destructured entry parameters, shadowed names, or passing an entire legacy interface object to other code.
- Scripts that cannot be parsed, entry or registration locations that do not meet the requirements, or component packages without a usable JavaScript entry point.

Conversion only rewrites recognized interface references; it does not prove that a component behaves identically to its legacy environment. Components can still fail if they depend on unrecognized interfaces, external scripts, assets, or old runtime behavior. For components that cannot be converted, update the source to use the current SDK/UI and rebuild with the new CLI; see the [package migration guide](./package-migration.md). If a legacy component prevents the entire project package from importing, keep a backup of the original project, remove that component from the LCP project, and export again. After a successful import, upload the rebuilt component and manually restore its configuration and related blueprints. Uploading a legacy component separately does not invoke the project-package conversion described above.

## What Is Not Migrated Automatically

- **Legacy application data:** DataVisool does not directly read or upgrade LCP's application data directory, database, or workspace. Export a project ZIP from LCP first. If a project package cannot be exported, there is no supported automatic migration path.
- **Application settings and state:** Legacy shortcuts, preferences, AI model configuration and keys, Agent history, and task state are not automatically migrated through project packages and need to be configured again in the new app.
- **Legacy clipboard content:** Data copied in the old app cannot be pasted directly as clipboard data in the new app.
- **Content outside the package:** Local assets not included in the package, external database data, external services, and dependencies are not automatically restored.
- **Reverse migration:** DataVisool project exports are not guaranteed to import into LCP, and new server components are not guaranteed to run in the old host.

## Checks After Import

1. Compare the canvas layout, layers, text, tables, loop containers, and maps with the original project.
2. Check images, videos, fonts, and 3D models. If assets are missing, restore them in the original project and export again.
3. Test data connections such as shared APIs, databases, and WebSockets. Verify that addresses, credentials, and permissions work on the target machine.
4. Test blueprint events, subprojects, screen references, dynamic scripts, and custom server components individually.
5. Once verified, export a new project package from DataVisool and retain the original LCP package as a backup.

## Troubleshooting

| Symptom | What to do |
| --- | --- |
| `manifest.json` is missing, or the format or version is unsupported | Confirm that you selected an LCP project ZIP, not an individual component ZIP, backup directory, or repackaged file. Check the manifest marker and version and re-export from LCP if needed. A new export is not guaranteed to use a supported format. |
| A passphrase is required or decryption fails | Check the passphrase used for export and the integrity of the package. An unknown passphrase or unsupported encryption algorithm cannot be recovered automatically through this importer. |
| A file is missing or fails validation | Keep the original package, check the assets in LCP, and export again. Do not modify package files to bypass validation. |
| Project structure or a referenced project is missing | Check that subprojects and screen references are included in the LCP export, then re-export. Version 2 packages do not provide the version 3 project-graph restoration. |
| A server component cannot be converted statically | Inspect the named component and interface, then update and rebuild it using the [package migration guide](./package-migration.md). If needed, remove it from the old project before export and manually restore its configuration and blueprints after import. |
| Import succeeds but data sources must be selected again | Re-select the shared API or database in DataVisool, check its address and credentials, and test the connection. Import success does not mean those references were automatically restored. |
| Import succeeds but display or interactions differ from LCP | Check assets, component settings, maps, scripts, and blueprints individually, and adjust them to current component capabilities. Retain the original project for comparison; not all legacy behavior can be repaired automatically. |

An import success message is not a substitute for these checks.
