# 本地账号切换说明

这个工具不依赖 `codex-auth`。账号切换由 Tauri 后端直接管理本机 Codex 认证文件完成。

## 文件结构

默认使用 Codex 的本地状态目录：

```text
~/.codex/
  auth.json
  accounts/
    registry.json
    <base64url(account_key)>.auth.json
    auth.json.bak.<timestamp>
```

如果设置了 `CODEX_HOME`，并且该目录存在，工具会优先使用 `CODEX_HOME`。

## 各文件用途

- `~/.codex/auth.json` 是当前 Codex CLI / app-server 会读取的认证文件。
- `~/.codex/accounts/*.auth.json` 是保存下来的账号快照。
- `~/.codex/accounts/registry.json` 是账号索引，记录当前激活账号和账号展示信息。
- `~/.codex/accounts/auth.json.bak.<timestamp>` 是切换账号前自动生成的备份。

## 登录添加账号

点击“登录添加账号”后：

1. 工具通过 Codex app-server 调用官方登录流程。
2. 浏览器会打开 Codex 登录页面；如果返回设备码，浮窗会同时显示验证码。
3. 登录完成后，Codex 会写入当前 `~/.codex/auth.json`。
4. 浮窗收到 `account/login/completed` 通知后，会读取新的 `auth.json`。
5. 工具会把完整认证内容保存为 `accounts/<base64url(account_key)>.auth.json`，并更新 `registry.json`。

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

1. 工具读取目标账号的 `accounts/*.auth.json` 快照。
2. 工具先把当前 `~/.codex/auth.json` 备份到 `accounts/auth.json.bak.<timestamp>`。
3. 工具用目标账号快照替换 `~/.codex/auth.json`。
4. 工具更新 `registry.json` 的 `active_account_key` 和 `last_used_at`。
5. 浮窗重启自己的 `codex app-server`，让新账号立即生效。

如果打开了“切换后自动重启 Codex 客户端”，工具还会在切换成功后重启官方 Codex 桌面客户端。也可以在账号面板里点击“立即重启 Codex 客户端”手动触发。

桌面客户端重启只针对 macOS 上 bundle id 为 `com.openai.codex` 的官方 Codex App。实现方式是先请求 Codex 正常退出，如果失败则用 `pkill -TERM -x Codex` 兜底，然后通过 `open -b com.openai.codex` 重新打开。

已经运行中的其他 Codex CLI 或 VS Code 扩展不会自动切换账号；它们通常需要重启或重新连接后才会读取新的 `auth.json`。

## 兼容性

当前实现兼容 `codex-auth` 使用过的目录结构：

- 已存在的 `~/.codex/accounts/registry.json` 会被直接读取。
- 已存在的 `accounts/*.auth.json` 快照会继续可用。
- 新保存的账号也使用相同的 `base64url(account_key).auth.json` 命名方式。

区别是：切换时不再调用 `codex-auth switch`，所有文件读写都由本应用完成。

## 限制

- 这个工具调用 Codex app-server 的官方登录流程，不直接实现 OAuth token exchange。
- 工具只管理本机文件，不跨设备同步账号。
- 认证文件包含敏感 token，不要把 `~/.codex/accounts` 提交到 Git 或发给别人。
- 重启官方 Codex 桌面客户端会关闭当前 Codex App 窗口；正在进行的客户端会话可能需要重新打开。
- 删除账号、编辑别名、导入外部 auth 文件还没有做成 UI。
