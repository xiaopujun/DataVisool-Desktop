# Building DataVisool

[简体中文](../cn/build.md) | English

This guide covers running and building the DataVisool desktop app from source, as well as creating desktop release packages with the repository's release script. Unless otherwise noted, run commands from the repository root.

## Prerequisites

- Install Node.js, pnpm, and the Rust toolchain. The repository does not currently specify a single required Node.js or pnpm version.
- Install the dependencies for your operating system using the [Tauri prerequisites guide](https://v2.tauri.app/start/prerequisites/). macOS requires Xcode Command Line Tools; Windows requires the Microsoft C++ build tools and WebView2.
- Make sure the dependency sources are accessible. Installing dependencies and preparing build tools for the first time may require a network connection.

Check that the tools are available:

```bash
node --version
pnpm --version
rustc --version
cargo --version
```

## Install dependencies

```bash
pnpm install --frozen-lockfile
```

This installs dependencies for the entire pnpm workspace using the repository's lockfile.

The desktop app also needs the configured ripgrep binaries as sidecars (helper programs bundled with the app). Development and build commands verify the files and their SHA-256 hashes for all configured targets. If a file is missing or fails verification, run:

```bash
pnpm -F datavisool-app sidecar:prepare
```

This downloads and verifies the ripgrep binaries for each target and requires a network connection and the `tar` command. To check existing files without downloading them, run `pnpm -F datavisool-app sidecar:verify`.

## Local development

Start the desktop app in development mode:

```bash
pnpm -F datavisool-app tauri:dev
```

To start only the frontend development server:

```bash
pnpm -F datavisool-app dev
```

The frontend server does not include the Tauri desktop backend. Use `tauri:dev` when desktop capabilities are needed. Both commands first build the workspace packages required by the app.

## Local builds

Build only the frontend. The output is written to `datavisool-app/dist/`:

```bash
pnpm -F datavisool-app build
```

Build the desktop app and installer for the current machine:

```bash
pnpm -F datavisool-app tauri:build
```

Tauri verifies the sidecars and builds the frontend, then compiles the Rust backend and creates the bundle. Desktop artifacts are normally placed under `datavisool-app/src-tauri/target/release/bundle/`; consult the build output for the exact formats and paths.

For a build with internal debugging tools enabled, use `pnpm -F datavisool-app tauri:build:internal`. This build is intended for internal debugging.

## Checks and tests

Choose checks appropriate to your changes:

```bash
# Frontend unit and component tests
pnpm -F datavisool-app test

# Desktop automation tests
pnpm -F datavisool-app test:desktop

# Public package boundaries and build artifacts
pnpm run check:packages
pnpm run test:package-artifacts

# Desktop Rust tests
cargo test --manifest-path datavisool-app/src-tauri/Cargo.toml
```

See the [desktop test guide](../../datavisool-app/tests/desktop/README.md) for coverage and usage requirements. See the [documentation overview](../README.md#7-测试与验证入口) for test commands for other modules (in Chinese).

## Component CLI

View the CLI commands and build its component template from the repository:

```bash
pnpm -F datavisool-cli cli --help
pnpm -F datavisool-cli template:build
```

Component packages built by the CLI include an entry script and associated assets. The ZIP can be uploaded to the app's server component management page.

## Desktop release packages

The repository's `release:desktop` script runs only on a macOS host. By default, it builds macOS Apple Silicon, macOS Intel, and Windows x64 in sequence. It stages `.app`, `.dmg`, and Windows NSIS `.exe` artifacts under `datavisool-app/dist/desktop-release/releases/<version>/` by default.

Release builds that include macOS targets require an Apple Developer ID Application signing identity and notarization credentials. Building the Windows target from macOS also requires `cargo-xwin`, LLVM's `llvm-rc`, and NSIS. See [`datavisool-app/.env.release.example`](../../datavisool-app/.env.release.example) for configuration examples. You can inject these settings as environment variables or copy the example to `datavisool-app/.env.release` and fill it in. Do not commit credentials to the repository.

First, check the build environment for the selected targets:

```bash
pnpm -F datavisool-app release:desktop -- --doctor --targets darwin-aarch64,darwin-x86_64,windows-x86_64
```

Install any missing tools and Rust targets and complete the signing configuration as indicated by the check, then run:

```bash
# Build all three targets
pnpm -F datavisool-app release:desktop

# Build only the selected target
pnpm -F datavisool-app release:desktop -- --targets darwin-aarch64

# After an interruption, try to reuse existing progress and artifacts;
# targets and build options must match the original command
pnpm -F datavisool-app release:desktop -- --resume
```

Before releasing, check the versions in `datavisool-app/package.json` and `datavisool-app/src-tauri/Cargo.toml`. Without `--resume`, the script clears the staging directory and clears each target's bundle directory before rebuilding it. Move any artifacts you want to keep elsewhere first. For more options, run `pnpm -F datavisool-app release:desktop -- --help`.
