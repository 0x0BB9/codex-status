# Windows 支持说明

## 支持范围

Windows 10/11 x64 版本支持以下功能：

- 系统托盘、窗口显示/隐藏和关闭时驻留
- `Ctrl + Alt + Space` 全局快捷键
- 透明置顶浮窗、拖拽吸边和右侧自动收起
- Codex 账号、额度和最近任务读取
- 本地账号快照、添加账号和账号切换
- 切换后重启官方 Codex/ChatGPT 桌面客户端
- NSIS `.exe` 和 MSI `.msi` 安装包

## Codex app-server 探测

浮窗需要启动官方 `codex app-server`。Windows 会依次尝试：

1. `%LOCALAPPDATA%\Programs\Codex\resources\codex.exe`
2. `%LOCALAPPDATA%\Programs\ChatGPT\resources\codex.exe`
3. Microsoft Store 中名称匹配 OpenAI、ChatGPT 或 Codex 的应用包
4. PATH 中的 `codex app-server`

官方桌面应用内部结构不是稳定的公共接口。如果桌面应用升级后无法找到内置 `codex.exe`，PATH 中安装的 Codex CLI 会作为兼容回退。

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

多账号快照依赖文件形式的 `auth.json`。如果 Windows Codex 把凭据存入 Credential Manager，请在 `%USERPROFILE%\.codex\config.toml` 中设置：

```toml
cli_auth_credentials_store = "file"
```

设置后重新登录一次，再使用“保存当前账号”或“登录添加账号”。这只改变 Codex 的本机凭据存储方式，不会把 Token 上传到本工具。

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
- Windows Store 应用内部路径可能发生变化，需要在真实 Windows 环境持续验证。
- 切换账号后重启客户端会关闭现有 Codex 窗口。
- WSL 有独立的 Linux home；除非设置 `CODEX_HOME`，否则 WSL CLI 不会自动共享 `%USERPROFILE%\.codex`。
