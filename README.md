# DSH Desktop

English | [简体中文](README.zh-CN.md)

A Tauri 2 desktop client for DeepSeek Harness. On startup, the application runs the following command locally:

```bash
npx --yes @deepseek-ai/dsh@0.1.0-rc.6 web --host 127.0.0.1 --port 3080
```

When the service is ready, the same desktop window opens `http://127.0.0.1:3080`. Closing the application also stops the dsh child process that it started.

## Requirements

- Node.js 20 or later, including `npx`
- Rust stable
- The Tauri 2 system dependencies for your platform

On the first run, `npx` requires network access to download `@deepseek-ai/dsh` and its dependencies. Later launches reuse the npm cache.

## Development

```bash
npm install
npm run tauri:dev
```

## Build Installers

```bash
npm run tauri:build
```

Build outputs are written to `src-tauri/target/release/bundle/`.

## GitHub Actions Artifacts

Pushing to `main`, opening a pull request, or manually running the `Build installers` workflow builds installers on native runners:

- macOS DMG: artifact `dsh-desktop-macos-dmg`
- Windows NSIS EXE: artifact `dsh-desktop-windows-exe`

After a successful build, download the installers from the `Artifacts` section of the corresponding GitHub Actions run. Artifacts are retained for 14 days.

## Optional Environment Variables

- `DSH_NPX_PATH`: absolute path to the `npx` executable.
- `DSH_WORKSPACE`: default working directory for dsh. The current user's home directory is used when this variable is not set.

The application always listens on `127.0.0.1:3080`. If the port is already occupied, the startup screen reports an error instead of connecting to an unknown local service.

## Branding

The interface and application icons use the DeepSeek Harness black whale mark included with `@deepseek-ai/dsh-web-frontend`.
