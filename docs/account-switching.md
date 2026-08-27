# 本地账号切换说明

这个工具不依赖 `codex-auth`。账号切换由 Tauri 后端直接管理本机 Codex 认证文件完成。

## 文件结构

默认使用 Codex 的本地状态目录；macOS/Linux 为 `~/.codex`，Windows 为 `%USERPROFILE%\.codex`：

```text
~/.codex/
  auth.json
  accounts/
    registry.json
    <base64url(account_key)>.auth.json
    auth.json.bak.<timestamp>
```

如果设置了 `CODEX_HOME`，并且该目录存在，工具会优先使用 `CODEX_HOME`。

多账号切换依赖文件形式的 `auth.json`。执行添加、保存或切换账号时，工具会自动在 `config.toml` 中设置 `cli_auth_credentials_store = "file"`；如果需要修改已有配置，会先备份为 `config.toml.bak.status-floater`。用户不需要手动编辑配置。

## 各文件用途

- `~/.codex/auth.json` 是当前 Codex CLI / app-server 会读取的认证文件。
- `~/.codex/accounts/*.auth.json` 是保存下来的账号快照。
- `~/.codex/accounts/registry.json` 是账号索引，记录当前激活账号和账号展示信息。
- `~/.codex/accounts/auth.json.bak.<timestamp>` 是切换账号前自动生成的备份。

## 登录添加账号

点击“登录添加账号”后：

1. 工具自动启用文件形式的本机 Codex 凭据存储。
2. 工具通过安装包内置的官方 Codex app-server 调用官方登录流程。
3. 如果当前已有登录账号，工具会先把最新认证回写到它的账号快照。
4. 工具优先打开标准浏览器 OAuth 登录，让用户选择要添加的 ChatGPT 账号。
5. 只有浏览器登录无法启动时才回退到设备码；设备码登录需要在 ChatGPT 安全设置中启用相关授权。
6. 登录完成后，Codex 会写入当前 `~/.codex/auth.json`。
7. 浮窗收到 `account/login/completed` 通知后，会读取新的 `auth.json`。
8. 工具会把完整认证内容保存为 `accounts/<base64url(account_key)>.auth.json`，并更新 `registry.json`。
9. 如果打开了“账号变更后自动重启 Codex 客户端”，工具会在账号保存成功后重启官方桌面客户端。

这个流程不依赖 `codex-auth`，也不会在本应用里复刻 OAuth token exchange。

## 保存当前账号

1. 先用官方 Codex 完成登录，让 Codex 写入当前 `~/.codex/auth.json`。
2. 打开浮窗，点击“保存当前账号”。
3. 工具会读取当前 `auth.json`，提取账号 ID、邮箱等非敏感展示信息。
4. 工具会把完整认证内容保存为 `accounts/<base64url(account_key)>.auth.json`。
5. 工具会更新 `registry.json`。

工具不会把 token 上传到任何服务。

## 切换账号

点击账号卡片里的“切换”后：

1. 工具把当前 `~/.codex/auth.json` 回写到当前账号快照，保存 Codex 自动刷新后的最新 token。
2. 工具把当前认证备份到 `accounts/auth.json.bak.<timestamp>`。
3. 工具用目标账号快照替换 `~/.codex/auth.json`，并更新 `registry.json`。
4. 浮窗重启自己的 `codex app-server`，读取并验证目标账号登录状态，但不会仅为切换而强制轮换 refresh token。
5. 验证成功后，再把 Codex 当前使用的认证回写到目标账号快照；Codex 在真正需要时仍会自动刷新 token。
6. 如果验证失败，工具会自动切回并验证原账号，避免停留在未登录状态。
7. 目标账号会标记为“需要重新登录”，记录失败时间和精简错误原因，但不会改写认证快照。

## 重新登录失效账号

点击失效账号卡片里的“重新登录”后，工具会复用官方 Codex app-server 登录流程。请在浏览器中选择卡片所对应的同一 ChatGPT 账号。登录完成后，工具会核对账号 ID；只有目标账号验证成功时才刷新它的认证快照并清除失效标记。若登录成其他账号，新账号会正常保存，但原目标账号仍保持失效状态。

## 删除账号

- 只有非当前账号提供删除入口，并需要二次确认。
- 删除会同时移除 `registry.json` 中的账号记录和对应的 `accounts/*.auth.json` 快照。
- 当前账号不能删除；需要先切换到另一个已验证账号。
- 删除失败时工具会尝试恢复账号索引，避免列表与本地文件状态不一致。

如果打开了“账号变更后自动重启 Codex 客户端”，工具会在目标账号验证成功后重启官方 Codex 桌面客户端。也可以在账号面板里点击“立即重启 Codex 客户端”手动触发。

macOS 会通过 bundle id `com.openai.codex` 请求官方客户端退出，等待 `ChatGPT`/`Codex` 主进程真正结束后，再从实际 App 路径启动一个新实例并验证进程已经恢复。Windows 会从开始菜单应用注册信息中查找 Codex/ChatGPT，终止对应进程后重新启动注册的桌面应用。

已经运行中的其他 Codex CLI 或 VS Code 扩展不会自动切换账号；它们通常需要重启或重新连接后才会读取新的 `auth.json`。

新版 ChatGPT 桌面客户端可能使用独立于 `~/.codex/auth.json` 的宿主登录会话。工具可以保证自己的 app-server 和之后启动的 CLI 使用已验证账号，但不能修改官方客户端内部的 Cookie、钥匙串或宿主会话；重启后如果账号没有同步，需要在官方客户端内确认登录。

## 兼容性

当前实现兼容 `codex-auth` 使用过的目录结构：

- 已存在的 `~/.codex/accounts/registry.json` 会被直接读取。
- 已存在的 `accounts/*.auth.json` 快照会继续可用。
- 新保存的账号也使用相同的 `base64url(account_key).auth.json` 命名方式。

区别是：切换时不再调用 `codex-auth switch`，所有文件读写都由本应用完成。

## 限制

- 这个工具调用 Codex app-server 的官方登录流程，不直接实现 OAuth token exchange。
- refresh token 可能被其他 Codex 客户端、CLI 或扩展轮换或撤销；已经永久失效的账号仍需要重新登录一次。
- 发布安装包内置官方 Codex app-server，因此接收方不需要安装 Codex CLI。
- 工具只管理本机文件，不跨设备同步账号。
- 认证文件包含敏感 token，不要把 `~/.codex/accounts` 提交到 Git 或发给别人。
- 重启官方 Codex 桌面客户端会关闭当前 Codex App 窗口；正在进行的客户端会话可能需要重新打开。
- 编辑账号别名、导入外部 auth 文件还没有做成 UI。
