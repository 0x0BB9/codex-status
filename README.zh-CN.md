# Codex Status Floater

[English](README.md) | 简体中文

Codex Status Floater 是一个基于 Tauri 的轻量悬浮状态看板，主要解决两个问题：

- 随时查看当前 ChatGPT / Codex 使用额度
- 集中关注多个项目中最近任务的状态变化

应用通过官方 `codex app-server` 接口读取数据，可以固定在主工作区旁边使用。发布版安装包内置官方 Codex 原生二进制作为本地 sidecar，接收方不需要额外安装 Node.js、npm、pnpm 或 Codex CLI。

## 主要功能

- 展示当前账号、登录方式和套餐信息
- 保存多个本地 Codex 账号并一键切换
- 展示五小时和一周额度的使用比例与重置时间
- 根据最近活动时间排列任务
- 提供本地任务看板：关注置顶、备注、项目、优先级和阶段
- 接收 `thread/status/changed` 等实时任务状态通知
- 支持系统托盘、全局快捷键、透明悬浮和拖拽吸边
- 停靠到屏幕右侧后自动收起为额度胶囊；鼠标经过不会打扰，点击胶囊才恢复完整看板
- 提供多套浅色主题，并记住用户选择
- 启动本地 `codex app-server` 时显示精简运行日志

## 账号切换

应用内置本地账号管理逻辑，不依赖 `codex-auth`。

- 已保存账号位于 `~/.codex/accounts/`。
- 当前生效账号仍然使用 Codex 标准文件 `~/.codex/auth.json`。
- 添加、保存或切换账号前，应用会自动启用文件形式的 Codex 凭据存储，并在修改配置前创建备份。
- 点击“登录添加账号”后，应用会启动官方 Codex app-server 登录流程，并在登录完成后保存账号快照。
- 点击“保存当前账号”后，应用会把当前 Codex 登录凭据保存为本地账号快照。
- 点击“切换”后，应用会先保存当前账号的最新 token，再替换 `auth.json`，然后刷新 Codex 并验证目标账号。
- 如果验证失败，应用会自动恢复原账号，避免 Codex 停留在退出登录状态。
- 切换验证成功后，可以在账号面板中重启 macOS 或 Windows 上的官方 Codex 客户端。新版 ChatGPT 桌面端可能保留独立宿主会话，仍需在官方客户端中进行一次确认。

完整的文件结构、切换流程、备份、兼容性和限制说明见 [账号切换文档](docs/account-switching.md)。

## 本地任务看板

任务看板中的标记只保存在本机，不会修改 Codex 的真实任务状态。

- 将重要任务标记为关注，使其置顶并高亮显示。
- 添加“等接口”“等设计”或“今日处理”等本地备注。
- 按项目、优先级和阶段分类，并按照这些字段对看板分组。

存储位置和字段说明见 [任务看板文档](docs/thread-board.md)。

## 打包与分发

macOS App/DMG 和 Windows NSIS/MSI 的构建、签名及分发说明见 [打包分发文档](docs/distribution.md)。Windows 平台的行为差异和构建步骤见 [Windows 文档](docs/windows.md)。

版本变化见 [更新日志](CHANGELOG.md)。

## 本地网页预览

运行：

```bash
pnpm dev
```

网页模式仅用于预览界面。实时 Codex 数据需要通过 Tauri 的 stdio 桥接读取，因为本地 app-server 不接受浏览器发起的 WebSocket 握手。

## 运行 Tauri 桌面版

在项目目录中运行：

```bash
pnpm install
pnpm tauri dev
```

Tauri 桌面外壳提供以下能力：

- 窗口始终置顶
- 以透明悬浮工具窗口显示
- 通过系统托盘显示、隐藏或退出应用
- macOS 使用 `Option + Space`，Windows 使用 `Ctrl + Alt + Space` 全局显示或隐藏窗口
- 窗口拖到屏幕边缘附近时自动吸附
- 闲置后降低透明度，停靠右侧时自动收起为额度胶囊
- 以窄侧边工具窗口形式展示
- 优先通过 stdio 启动安装包内置的官方 `codex app-server` sidecar，并将已安装桌面客户端和系统 PATH 作为兼容回退方案
- 通过 Tauri 命令管理本地 Codex 账号快照

## 构建说明

- `.cargo/config.toml` 将 Cargo 指向 `rsproxy` sparse 索引，提高中国大陆网络环境下下载 Rust crate 的稳定性。
- `src-tauri/patches/` 包含 `cookie`、`tauri-utils` 和 `tauri` 的本地补丁。
- 这些补丁只收窄少数范围过宽的泛型 `From<T>` 实现，使项目可以在当前 Rust 工具链上正常编译，不改变应用所需的运行行为。

## 重新生成协议类型

Codex 更新 app-server 协议后，可以运行以下命令重新生成 TypeScript 类型：

```bash
pnpm protocol:generate
```
