# 打包分发说明

这个工具可以打成 macOS App/DMG 或 Windows NSIS/MSI 安装包发给别人使用。应用本身不包含 Codex 账号、token 或任何用户本机 `.codex` 内容。

## 使用前提

发布安装包内置官方 Codex 原生二进制，并以 Tauri sidecar 方式在本机启动 `codex app-server`。macOS 和 Windows 接收方都不需要额外安装 Node.js、npm、pnpm 或 Codex CLI。

macOS 启动顺序是：

1. 安装包内置的 Codex sidecar
2. `/Applications/ChatGPT.app` 或 `/Applications/Codex.app` 内的 Codex 二进制
3. 用户 Applications 目录中的官方客户端
4. 用户 shell 或 PATH 中的 Codex CLI

Windows 启动顺序是：

1. 安装包内置的 Codex sidecar
2. 官方 Codex/ChatGPT 桌面应用常见安装目录中的 `codex.exe`
3. Microsoft Store 应用包中的 `codex.exe`
4. PATH 中的 `codex app-server`

Windows 使用官方文档约定的 `%USERPROFILE%\.codex` 作为默认状态目录。详细说明见 [windows.md](windows.md)。

## 本地构建

```bash
pnpm install
pnpm tauri:build
```

常用产物位置：

```text
src-tauri/target/release/bundle/macos/Codex Status Floater.app
src-tauri/target/release/bundle/dmg/Codex Status Floater_0.1.0_aarch64.dmg
```

如果只是本机调试包：

```bash
pnpm tauri:build:debug
```

如果是小范围测试，并且暂时没有 Apple Developer ID，可以生成一个 ad-hoc deep signed 测试包：

```bash
pnpm mac:package:adhoc
```

对应产物：

```text
src-tauri/target/release/bundle/dmg/Codex Status Floater_0.1.0_aarch64_adhoc.dmg
```

## Windows 构建

请在 Windows 10/11 或仓库自带的 GitHub Actions Windows Runner 上执行：

```powershell
pnpm install --frozen-lockfile
pnpm windows:build
```

产物通常位于：

```text
src-tauri\target\release\bundle\nsis\*.exe
src-tauri\target\release\bundle\msi\*.msi
```

GitHub 仓库的 `Build Windows` 工作流支持手动触发，也会在推送 `v*` 标签时构建并上传安装包 artifact。

## 分发前检查

发布前至少确认：

```bash
pnpm build
pnpm tauri:build
git diff --check
```

不要把这些本地文件或目录放进发行包或 Git 仓库：

```text
~/.codex/auth.json
~/.codex/accounts/
.env
*.auth.json
registry.json
thread-board.json
```

项目 `.gitignore` 已经覆盖这些路径或文件名。

## 签名与安装提示

如果没有 Apple Developer ID 签名和 notarization，别人第一次打开时 macOS 可能提示“无法验证开发者”。小范围测试可以发 ad-hoc 包，让对方右键 App 后选择“打开”。

如果要公开分发，建议使用 Apple Developer ID 对 release 产物签名并 notarize。签名只影响 macOS 信任链，不会改变应用读取用户本机 Codex 状态的逻辑。

Windows 未签名安装包可能触发 Microsoft Defender SmartScreen。公开分发时应配置可信代码签名证书；GitHub Actions 默认只生成未签名测试包。

## 隐私边界

- 应用只在用户本机读取 Codex app-server、本机 `.codex/auth.json` 和本机任务元数据。
- 账号快照保存在用户自己的 `.codex/accounts/`。
- 本项目不内置任何服务端，也不会上传 token。
- 安装包会包含 Apache-2.0 许可的官方 Codex 原生二进制和第三方声明，不包含开发者账号数据。
- 打包产物不应包含开发者自己的 `~/.codex` 目录。
