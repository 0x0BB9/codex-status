# Windows 支持说明

## 给运营同事的使用步骤

安装包已经内置官方 Codex 登录服务。接收方不需要安装 Node.js、npm、pnpm 或 Codex CLI。

1. 安装并打开 Codex Status Floater。
2. 点击“登录添加账号”，在浏览器中登录第一个 ChatGPT/Codex 账号。
3. 再次点击“登录添加账号”，在浏览器中选择第二个账号。
4. 以后在账号卡片上点击“切换”即可。建议保持“账号变更后自动重启 Codex 客户端”开启。

账号、Token 和账号快照只保存在当前 Windows 用户目录，不会打进安装包，也不会上传到本项目的服务端。

## 支持范围

Windows 10/11 x64 版本支持以下功能：

- 系统托盘、窗口显示/隐藏和关闭时驻留
- `Ctrl + Alt + Space` 全局快捷键
- 透明置顶浮窗、拖拽吸边和右侧自动收起；鼠标经过胶囊不会展开，点击才展开
- Codex 账号、额度和最近任务读取
- 本地账号快照、添加账号和账号切换
- 切换后重启官方 Codex/ChatGPT 桌面客户端
- NSIS `.exe` 和 MSI `.msi` 安装包

Windows 版本使用无边框工具窗口：启动时自动停靠到主屏幕右上侧，不占用任务栏位置，并持续保持置顶。额度胶囊会锁定在其当前显示器内展开，不会因为靠近分屏边界而跳到相邻屏幕。点击右上角“−”会隐藏到系统托盘；托盘图标或 `Ctrl + Alt + Space` 可以再次显示。

## Codex app-server 探测

浮窗需要启动官方 `codex app-server`。发布安装包会内置与应用一起分发的官方 Codex 原生二进制，Windows 会依次尝试：

1. 安装包内置的 Codex sidecar
2. `%LOCALAPPDATA%\Programs\Codex\resources\codex.exe`
3. `%LOCALAPPDATA%\Programs\ChatGPT\resources\codex.exe`
4. Microsoft Store 中名称匹配 OpenAI、ChatGPT 或 Codex 的应用包
5. PATH 中的 `codex app-server`

因此接收方不需要额外安装 CLI。桌面应用路径和 PATH 只作为兼容回退，不是正常运行的前提。

## 本地数据

Windows 官方 Codex App 和本工具默认共用：

```text
%USERPROFILE%\.codex
```

账号快照位于：

```text
%USERPROFILE%\.codex\accounts
```

如果存在有效的 `CODEX_HOME`，应用会优先使用该目录。认证文件依赖当前 Windows 用户目录的 ACL，不会包含在安装包中。

多账号快照依赖文件形式的 `auth.json`。用户第一次执行“登录添加账号”“保存当前账号”或“切换”时，应用会自动在 `%USERPROFILE%\.codex\config.toml` 中设置：

```toml
cli_auth_credentials_store = "file"
```

如果原配置需要修改，应用会先生成：

```text
%USERPROFILE%\.codex\config.toml.bak.status-floater
```

用户不需要手动编辑配置。这个设置只改变 Codex 的本机凭据存储方式，不会上传 Token。

## 本地构建

准备 Node.js、pnpm、Rust stable 和 Visual Studio C++ Build Tools，然后在 PowerShell 中执行：

```powershell
pnpm install --frozen-lockfile
pnpm windows:build
```

输出目录：

```text
src-tauri\target\release\bundle\nsis
src-tauri\target\release\bundle\msi
```

也可以在 GitHub Actions 页面手动运行 `Build Windows`，下载生成的 `codex-status-floater-windows` artifact。

## 已知限制

- Windows 安装包目前没有代码签名，首次安装可能出现 SmartScreen 提示。
- 切换账号后重启客户端会关闭现有 Codex 窗口。
- 新版 ChatGPT/Codex 桌面客户端可能维护独立于 `.codex/auth.json` 的宿主登录会话。本工具能保证自己的 app-server 和之后启动的 CLI 使用目标账号，但实际官方客户端仍需在 Windows 真机确认；如果没有同步，需要在官方客户端内确认登录。
- WSL 有独立的 Linux home；除非设置 `CODEX_HOME`，否则 WSL CLI 不会自动共享 `%USERPROFILE%\.codex`。
