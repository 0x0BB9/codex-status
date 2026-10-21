# 本地账号切换说明

这个工具不依赖 `codex-auth`。账号切换由 Tauri 后端直接管理本机 Codex 认证文件完成。

## 文件结构

默认使用 Codex 的本地状态目录；macOS/Linux 为 `~/.codex`，Windows 为 `%USERPROFILE%\.codex`：

```text
~/.codex/
  auth.json
  config.toml
  accounts/
    registry.json
    <base64url(account_key)>.auth.json
  status-floater/
    chatgpt-config.toml
    model-catalogs/
      <provider_id>.json
    connection-backups/
      switch-<timestamp>/
```

如果设置了 `CODEX_HOME`，并且该目录存在，工具会优先使用 `CODEX_HOME`。

多账号切换依赖文件形式的 `auth.json`。执行添加、保存或切换账号时，工具会自动在 `config.toml` 中设置 `cli_auth_credentials_store = "file"`；如果需要修改已有配置，会先备份为 `config.toml.bak.status-floater`。用户不需要手动编辑配置。

## 各文件用途

- `~/.codex/auth.json` 是当前 Codex CLI / app-server 会读取的认证文件。
- `~/.codex/accounts/*.auth.json` 是保存下来的账号快照。
- `~/.codex/accounts/registry.json` 是账号索引，记录当前激活账号和账号展示信息。
- `~/.codex/status-floater/chatgpt-config.toml` 是进入 API 模式前的 Plus 配置快照。
- `~/.codex/status-floater/model-catalogs/*.json` 是应用生成的自定义模型目录，不包含 API Key。
- `~/.codex/status-floater/connection-backups/switch-*/` 是切换前的配置和账号索引备份；从 Plus 切换时还包含认证备份。

## 添加 API 连接

1. 点击“添加 API 连接”，选择两列 CSV 或手动填写连接名称、Base URL、模型 ID 和 API Key。
2. CSV 只在当前窗口读取，不会复制进项目或账号目录。
3. 应用把 API Key 保存到 macOS 钥匙串或 Windows 凭据管理器；`registry.json` 只记录连接参数和 Key 末四位。
4. “保存并测试”只请求 `<Base URL>/models` 并确认模型 ID，不调用推理接口。
5. 激活连接时，应用生成私有模型目录并同时设置 `model`、`model_provider` 和 `model_catalog_json`。

API Key 长期只保存在系统凭据库。API 连接激活期间，工具会把 Key 临时写入自定义 provider 的 `experimental_bearer_token`，并把 `config.toml` 权限限制为仅当前用户可读；标准 `auth.json` 继续保留 Plus 登录。切回 Plus 后会恢复原配置，连接备份也会自动剔除 bearer token。

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
2. 工具把当前配置和账号索引备份到 `status-floater/connection-backups/switch-<timestamp>/`；当前连接是 Plus 时同时备份认证。
3. 工具用目标账号快照替换 `~/.codex/auth.json`，并更新 `registry.json`。
4. 浮窗重启自己的 `codex app-server`，读取并验证目标账号登录状态，但不会仅为切换而强制轮换 refresh token。
5. 验证成功后，再把 Codex 当前使用的认证回写到目标账号快照；Codex 在真正需要时仍会自动刷新 token。
6. 如果验证失败，工具会自动切回并验证原账号，避免停留在未登录状态。
7. 目标账号会标记为“需要重新登录”，记录失败时间和精简错误原因，但不会改写认证快照。

## Plus 与 API 连接切换

- 切到 API 连接时，工具从系统凭据库读取 Key，生成模型目录，并基于保存的 Plus 配置写入自定义 provider；Plus 的 `auth.json` 保持不变。
- app-server 加载配置后，工具会核对 `model` 和 `model_provider`，再创建一个显式绑定该 provider 的独立新任务，并通过 `codex://threads/<id>` 请求官方客户端打开。
- 切回 Plus 时恢复原 GPT 模型配置和目标认证快照，同时保留已保存 API 连接的无 Key 服务商定义，避免旧聊天打开时报 `Model provider not found`。
- 浮窗分别使用 `modelProviders: []`、`archived: false/true` 分页读取全部服务商的交互任务，切换后及官方客户端重启成功后按 ID 检查记录缺失、服务商变化和新增归档。重启前复查失败、记录缺失或服务商变化会停止后续步骤并尝试恢复原连接，不标记账号失效；新增归档独立提示。重启后复查仅报告，不改写任务。
- 从工具重启官方客户端后，原生兼容桥继续调用官方客户端自带运行时；将默认列表查询改为全部服务商，并将恢复、创建任务和新一轮请求的模型设置为当前连接的配置模型。旧 API 聊天可以使用当前 GPT 连接继续，任务 ID 和已有上下文保留。后续请求按当前连接计费，不按历史来源计费。
- 工具不批量修改 `state_5.sqlite` 或 rollout 的 provider 字段。运行时自身可能在继续任务时更新元数据；兼容查询不依赖这种更新。
- 首次跨服务商恢复某个任务时，桥接层会先备份原 rollout，再只保留可跨服务商重放的用户消息和助手可见回答。reasoning、function call、tool output、web search 等供应商私有对象不会发给目标接口，避免 `reasoning.content` 和第三方 `msg_` 调用 ID 被 GPT 拒绝；任务 ID不变。原文件备份到 `CODEX_HOME/status-floater/history-compat-backups/<任务ID>.before-cross-provider.jsonl`，写入失败则停止恢复。
- 兼容桥启用期间，悬浮工具顶部显示官方客户端任务的准备、处理、重试、失败和完成状态。该状态来自 app-server 通知；若 GPT 返回 429，界面会明确显示服务端繁忙和当前重试次数。
- 关闭自动重启后，必须点击“立即重启 Codex 客户端”再继续旧聊天。兼容桥检测到连接改变会拒绝请求，避免已加载任务沿用旧服务商。应用内选模型可能被连接的配置模型覆盖；模型选择以浮窗连接配置为准。
- 兼容桥只通过该次启动的进程环境启用，不修改官方安装文件或系统全局环境。完全退出客户端后从系统图标启动不会自动启用，需要再次从工具重启。若提示未确认兼容连接，不要继续旧 API 聊天。

## 找回历史与聊天备份

日常页面没有独立历史模块，自动备份和复查在切换流程中执行。发现缺失、服务商变化、新增归档、备份不完整或查询失败时，显示“聊天记录需要检查”；提示保存在本机，重启后仍保留，直到用户关闭。新的正常检查不会自动清除之前尚未确认的异常提示。

1. 出现异常提示后，连接成功并点击“找回记录”，再点击“读取 / 刷新历史”，选择“已归档”或搜索标题、服务商和任务 ID。
2. 点击“查看正文”可读取用户和助手消息；“导出 JSON”保存接口返回的任务内容到 `~/.codex/status-floater/history-exports/`，界面显示实际路径。
3. 需要恢复时点击“取消归档”并确认。从工具重启官方客户端启用兼容连接后，在官方侧栏重新打开原聊天。

切换前会把接口列出的本地聊天文件保存到 `~/.codex/status-floater/history-backups/`，每条任务保留首次备份 `.initial.jsonl` 和最近备份 `.latest.jsonl`；`last-switch.json` 保存最近切换前的 ID、路径、服务商和归档状态。自定义 `CODEX_HOME` 时使用对应目录。

历史查询或备份写入失败会阻止切换，并显示具体错误。单条记录的源文件缺失、不可读或不在允许目录内时，跳过该文件并继续备份其他记录；无本地路径同样跳过。这些记录仍保留在 `last-switch.json`，跳过的 ID 写入 `last-skipped.json`，界面提示备份不完整但允许切换。备份只覆盖接口可见的交互任务和已有 JSONL 内容，不包含外部附件，也不是持续同步。保留首次和最近两份，不自动删除首次备份；记录数量增长时目录会增长。聊天包含私人内容，文件仅保存在本机，沿用应用的私有文件写入逻辑（Unix 目录 0700、文件 0600；Windows 继承目录 ACL）。

该入口依赖 app-server 仍能读取记录；文件实际丢失、损坏或运行时不兼容时会报错。重启后的复查是一次采样，不保证阻止之后发生的归档。工具不修改官方客户端安装文件或任务数据库，也不自动恢复已归档记录。

## 重新登录失效账号

点击失效账号卡片里的“重新登录”后，工具会复用官方 Codex app-server 登录流程。请在浏览器中选择卡片所对应的同一 ChatGPT 账号。登录完成后，工具会核对账号 ID；只有目标账号验证成功时才刷新它的认证快照并清除失效标记。若登录成其他账号，新账号会正常保存，但原目标账号仍保持失效状态。

## 删除账号

- 只有非当前账号提供删除入口，并需要二次确认。
- 删除会同时移除 `registry.json` 中的账号记录和对应的 `accounts/*.auth.json` 快照。
- 删除 API 连接会同时删除系统凭据库中的 Key 和本地模型目录。
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
- API 服务必须支持 HTTPS、Bearer API Key、`GET /models` 和 OpenAI Responses 兼容请求；兼容程度仍取决于服务端实现。
- 更新当前 API 连接的 Key 后，浮窗会重新启动自己的 app-server 以加载新凭据。
- 重启官方 Codex 桌面客户端会关闭当前 Codex App 窗口；正在进行的客户端会话可能需要重新打开。
- 编辑账号别名、导入外部 auth 文件还没有做成 UI。
