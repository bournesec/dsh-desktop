# DSH Desktop

English | [简体中文](README.zh-CN.md)

A Tauri 2 desktop client for DeepSeek Harness. DeepSeek Harness can normally be started from a terminal with:

```bash
npx @deepseek-ai/dsh web
```

DSH Desktop handles this automatically. It first looks for a `dsh` command on `PATH`, an application-managed installation, or a copy already downloaded by npx. If none is available, it uses npm to install the latest `@deepseek-ai/dsh` release into its user-level application data directory. It then starts the resolved CLI with the `web --host 127.0.0.1 --port 3080` arguments. Later launches directly reuse the detected installation without checking for updates. When the service is ready, the same desktop window opens `http://127.0.0.1:3080`. Closing the application also stops any npm or dsh child process that it started.

## Requirements

- Node.js 20 or later, including `npm` and `npx`
- Rust stable
- The Tauri 2 system dependencies for your platform

The first automatic installation requires network access. The application uses an isolated npm cache inside its own data directory and does not modify the global npm installation.

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

### macOS signing and notarization

The workflow always applies at least a complete ad-hoc signature so that Apple Silicon builds downloaded from GitHub are not reported as damaged. Ad-hoc builds may still need to be explicitly allowed in macOS Privacy & Security.

For a public DMG that opens without a Gatekeeper warning, configure all of these GitHub Actions secrets. The workflow will then import the Developer ID Application certificate, sign the application, submit it to Apple for notarization, staple the ticket, and verify the result:

- `APPLE_CERTIFICATE`: base64-encoded Developer ID Application `.p12`
- `APPLE_CERTIFICATE_PASSWORD`: password used to export the `.p12`
- `KEYCHAIN_PASSWORD`: temporary CI keychain password
- `APPLE_ID`: Apple Developer account email
- `APPLE_PASSWORD`: app-specific Apple ID password
- `APPLE_TEAM_ID`: Apple Developer Team ID

## Optional Environment Variables

- `DSH_EXECUTABLE_PATH`: absolute path to an existing `dsh` executable. This takes priority over automatic discovery.
- `DSH_NPM_PATH`: absolute path to the `npm` executable used for automatic installation.
- `DSH_NODE_PATH`: absolute path to the `node` executable added to the child process `PATH`.
- `DSH_RUNTIME_DIR`: absolute path for the application-managed dsh installation and isolated npm cache.
- `DSH_NPM_CACHE_PATH`: absolute npm cache path used when detecting packages previously downloaded by npx.
- `DSH_NPX_PATH`: deprecated compatibility option; its directory is checked for a sibling `npm` executable.
- `DSH_WORKSPACE`: default working directory for dsh. The current user's home directory is used when this variable is not set.

The application always listens on `127.0.0.1:3080`. If the port is already occupied, the startup screen reports an error instead of connecting to an unknown local service.

## Branding

The interface and application icons use the DeepSeek Harness black whale mark included with `@deepseek-ai/dsh-web-frontend`.
