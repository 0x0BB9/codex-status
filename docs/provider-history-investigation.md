# 跨服务商聊天可见性排查

日期：2026-10-21。状态：已实现配置修复和原生兼容桥；隔离协议测试通过，官方 GUI 和 Windows 实际切换待验证。

## 验收标准

GPT / API 切换完成后，原聊天应能在官方客户端正常显示、打开和继续使用。浮窗里的备份、异常提示及找回入口不替代这一标准。

## 已验证

使用独立临时 `CODEX_HOME`，生成 OpenAI、自定义服务商各一条未归档和已归档记录，未访问真实账号或聊天，未发起模型推理。

| 运行时 | `null` / 不传筛选 | `[]` | 范围 |
| --- | --- | --- | --- |
| 0.146.0（工具内置） | 只返回当前 OpenAI 服务商 | 返回两个服务商 | 未归档、已归档均复现 |
| 0.163.0-alpha.10.2（本机官方客户端） | 只返回当前 OpenAI 服务商 | 返回两个服务商 | 未归档、已归档均复现 |

本机官方客户端版本为 26.1013.40934。只读检查应用代码，最近聊天的 `fetchRecentThreads` 和归档页的 `listArchivedThreads` 均传 `modelProviders: null`。因此升级浮窗内置运行时或修改浮窗自己的查询不能修复官方列表；Mac 上未观察到真实历史消失也不代表此行为仅存在于 Windows。

当前[官方接口文档](https://learn.chatgpt.com/docs/app-server#list-threads-with-pagination--filters)称 null 或未传时包含全部服务商，与上述两个本机二进制的实测结果不同。此处以具体运行时结果为准。

额外隔离实验：对自定义服务商任务调用 `thread/resume`，显式设置 `modelProvider: openai` 和 GPT 模型，但不开始推理。0.146.0 下任务仍被默认列表过滤；0.163.0-alpha.10.2 下数据库查询一度可见，扫描修复元数据后再次不可见。不能把这个调用当成可靠的历史迁移方案。

## 未确定

- 对方 Windows 客户端及内置运行时的准确版本和安装方式。
- 用户记录被归档的具体调用方和触发动作；筛选实验没有证明账号切换会归档任务。
- Windows 客户端是否支持稳定的兼容接入方式，使官方列表显式查询全部服务商。
- Windows 官方 GUI 中的实际启动、恢复和推理结果。

用户补充对方“应该是最新版本”，尚不能据此推断准确构建号。后续以最新版作为验证基线，不把版本差异当成已经确认的原因。

只读检查本机应用发现 CLI / app-server 地址覆盖能力及 `codex_cli_command` 启动覆盖，但尚未验证 Windows 的安装包、启动入口及版本兼容性，不将其作为已支持方案。不得在未验证前替换官方应用文件或批量改写用户数据库。

## 修复实现

原生兼容桥已嵌入应用主程序，独立测试程序仍可按 feature 构建。从浮窗重启官方客户端时，通过进程环境将 CLI 入口指向兼容桥，再调用官方客户端原运行时，不降级到浮窗的内置版本、不改官方安装文件。

- 默认 `thread/list` 的 null / 缺省筛选改为 `[]`，显式筛选、归档筛选和分页不变。
- 切回 GPT 时保留已保存 API 连接的无 Key 定义，修复官方前端的 `Model provider not found` 校验失败。
- `thread/resume`、`thread/start`、`thread/fork` 和 `turn/start` 使用当前连接的配置模型；恢复请求显式传当前服务商，原任务 ID和可见问答不变。
- 跨服务商恢复前先备份原 JSONL，再只保留用户消息和助手可见回答。reasoning、function call、tool output、web search 等供应商私有对象不能安全重放，其中第三方 `msg_` function-call ID 会被 GPT 要求的 `fc_` 格式拒绝。替换失败时恢复原文件并中止请求。
- 兼容桥透传 app-server 输出的同时记录任务准备、处理、重试、失败和完成通知，悬浮工具读取私有状态文件显示当前进度。
- 路由保存在私有 `status-floater/active-history-route.json`，只有模型、服务商和连接标识，没有 Key。路由变化后旧桥拒绝请求，要求重启客户端，避免已加载任务沿用旧连接。完全退出客户端后从系统图标启动不会自动启用兼容桥。
- 启动确认文件只证明桥接进程已启动，不替代官方 GUI 功能验收。未确认时界面报错，不宣称切换完全成功。

用 0.146.0 和本机后来更新到的 0.163.0-alpha.11 运行时进行隔离测试：默认及归档列表均包含两个服务商；旧服务商定义不存在时，可用 OpenAI / GPT 模型恢复同一任务。另用显式本机模拟服务接收含自定义 reasoning、function call 和 tool output 的续聊请求，确认可见问答保留、供应商私有对象未发送、原始备份已创建、桥接状态可读，并核对配置模型与目标测试凭据。原生 OpenAI 测试只调用恢复接口，不发起付费推理。本机模拟服务结果不能替代真实 Plus 推理验收。

```sh
cargo test --offline --manifest-path src-tauri/Cargo.toml --features history-bridge-prototype --bin codex-history-bridge
cargo build --offline --manifest-path src-tauri/Cargo.toml --features history-bridge-prototype --bin codex-history-bridge
node scripts/check-provider-history.mjs --bridge <built-bridge> --follow-current <codex-binary>
node scripts/check-provider-history.mjs --bridge <built-app-executable> --embedded-bridge --follow-current <codex-binary>
```

仍需验收：从浮窗重启官方客户端后，旧 DeepSeek 聊天在官方列表可见、能打开，并用当前 GPT 连接回答。Windows 必须单独验证，不能以 Mac 的协议测试代替。工具不能保证阻止客户端发出归档操作，也不自动取消归档。

## 复测工具

```sh
node scripts/check-provider-history.mjs <codex-binary> [<another-codex-binary>]
```

脚本只创建并清理自己的临时测试目录，输出筛选结果及恢复接口实验结果。无需真实 API Key；结果不能替代对应 Windows 客户端的实际切换验证。
