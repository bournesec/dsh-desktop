# DSH Desktop

[English](README.md) | 简体中文

使用 Tauri 2 封装的 DeepSeek Harness 桌面客户端。通常可以在终端中通过以下命令启动 DeepSeek Harness：

```bash
npx @deepseek-ai/dsh web
```

DSH Desktop 会自动完成这套流程。应用依次查找 PATH 中的 `dsh` 命令、应用托管版本以及 npx 已下载的缓存；如果都不存在，则通过 npm 将最新版 `@deepseek-ai/dsh` 安装到用户级应用数据目录。找到可用 CLI 后，应用使用 `web --host 127.0.0.1 --port 3080` 参数启动服务。后续启动直接复用检测到的版本，不会每次联网检查更新。服务就绪后，同一个桌面窗口会自动打开 `http://127.0.0.1:3080`。关闭桌面应用时，由它启动的 npm 或 dsh 子进程也会一并退出。

## 环境要求

- Node.js 20 或更高版本，需要包含 `npm` 和 `npx`
- Rust stable
- 当前平台的 Tauri 2 系统依赖

首次自动安装需要联网。应用使用自身数据目录中的独立 npm 缓存，不会修改全局 npm 安装。

## 开发运行

```bash
npm install
npm run tauri:dev
```

## 构建安装包

```bash
npm run tauri:build
```

构建产物位于 `src-tauri/target/release/bundle/`。

## GitHub Actions 安装包

推送到 `main`、创建 Pull Request 或手动运行 `Build installers` workflow 时，会分别在原生 runner 上构建安装包：

- macOS DMG：Artifact `dsh-desktop-macos-dmg`
- Windows NSIS EXE：Artifact `dsh-desktop-windows-exe`

构建成功后，可以从对应 GitHub Actions 运行详情页的 `Artifacts` 区域下载安装包。Artifact 保留 14 天。

### macOS 签名与公证

工作流始终至少应用完整的 ad-hoc 签名，避免从 GitHub 下载的 Apple Silicon 应用被 macOS 报告为“已损坏”。ad-hoc 构建仍可能需要在 macOS“隐私与安全性”中明确允许打开。

如需发布可直接打开且不显示 Gatekeeper 警告的 DMG，请配置以下全部 GitHub Actions Secrets。工作流会自动导入 Developer ID Application 证书、签名、提交 Apple 公证、装订公证票据并验证结果：

- `APPLE_CERTIFICATE`：Developer ID Application `.p12` 的 Base64 内容
- `APPLE_CERTIFICATE_PASSWORD`：导出 `.p12` 时设置的密码
- `KEYCHAIN_PASSWORD`：CI 临时钥匙串密码
- `APPLE_ID`：Apple Developer 账户邮箱
- `APPLE_PASSWORD`：Apple ID 专用密码
- `APPLE_TEAM_ID`：Apple Developer Team ID

## 可选环境变量

- `DSH_EXECUTABLE_PATH`：指定现有 `dsh` 可执行文件的绝对路径，优先级高于自动查找。
- `DSH_NPM_PATH`：指定自动安装时使用的 `npm` 可执行文件绝对路径。
- `DSH_NODE_PATH`：指定加入子进程 `PATH` 的 `node` 可执行文件绝对路径。
- `DSH_RUNTIME_DIR`：指定应用托管 dsh 和独立 npm 缓存所在的绝对目录。
- `DSH_NPM_CACHE_PATH`：指定用于检测既有 npx 下载包的 npm 缓存绝对路径。
- `DSH_NPX_PATH`：已弃用的兼容选项；应用会在其所在目录查找同级 `npm`。
- `DSH_WORKSPACE`：指定 dsh 启动时的默认工作目录；未设置时使用当前用户主目录。

应用固定监听 `127.0.0.1:3080`。若端口已被占用，启动页会显示错误，避免误连到未知本地服务。

## 品牌资源

界面与应用图标使用 `@deepseek-ai/dsh-web-frontend` 随附的 DeepSeek Harness 黑鲸鱼标识。
