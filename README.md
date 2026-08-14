# DSH Desktop

使用 Tauri 2 封装的 DeepSeek Harness 桌面客户端。应用启动时会在本机运行：

```bash
npx --yes @deepseek-ai/dsh@0.1.0-rc.6 web --host 127.0.0.1 --port 3080
```

服务就绪后，同一个桌面窗口会自动打开 `http://127.0.0.1:3080`。关闭桌面应用时，由它启动的 dsh 子进程也会一并退出。

## 环境要求

- Node.js 20 或更高版本（需要包含 `npx`）
- Rust stable
- 当前平台的 Tauri 2 系统依赖

首次运行时，`npx` 需要联网下载 `@deepseek-ai/dsh` 及依赖。后续启动会复用 npm 缓存。

## 开发运行

```bash
npm install
npm run tauri:dev
```

## 构建安装包

```bash
npm run tauri:build
```

产物位于 `src-tauri/target/release/bundle/`。

## GitHub Actions 安装包

推送到 `main`、创建 Pull Request 或手动运行 `Build installers` workflow 时，会分别在原生 runner 上构建：

- macOS DMG：Artifact `dsh-desktop-macos-dmg`
- Windows NSIS EXE：Artifact `dsh-desktop-windows-exe`

构建完成后，可在对应 GitHub Actions 运行详情页的 `Artifacts` 区域下载，产物保留 14 天。

## 可选环境变量

- `DSH_NPX_PATH`：指定 `npx` 可执行文件的绝对路径。
- `DSH_WORKSPACE`：指定 dsh 启动时的默认工作目录；未设置时使用当前用户主目录。

应用固定监听 `127.0.0.1:3080`。若端口已被占用，启动页会显示错误，避免误连到未知本地服务。

界面与应用图标使用 `@deepseek-ai/dsh-web-frontend` 随附的 DeepSeek Harness 黑鲸鱼标识。
