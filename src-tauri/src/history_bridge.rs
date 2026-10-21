use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

static BRIDGE_STATUS_WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActiveHistoryRoute {
    pub model: String,
    pub model_provider: String,
    #[serde(default)]
    pub connection_key: Option<String>,
}

pub fn rewrite_follow_current_request(line: &str, route: &ActiveHistoryRoute) -> String {
    let normalized = rewrite_history_request(line);
    let Ok(mut request) = serde_json::from_str::<serde_json::Value>(&normalized) else {
        return normalized;
    };
    let method = request
        .get("method")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let resume = matches!(method, "thread/resume" | "thread/start" | "thread/fork");
    let turn = method == "turn/start";
    if !resume && !turn {
        return normalized;
    }
    let Some(params) = request
        .get_mut("params")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return normalized;
    };
    params.insert("model".into(), serde_json::json!(route.model));
    if resume {
        params.insert(
            "modelProvider".into(),
            serde_json::json!(route.model_provider),
        );
    }
    if let Some(settings) = params
        .get_mut("collaborationMode")
        .and_then(|mode| mode.get_mut("settings"))
        .and_then(serde_json::Value::as_object_mut)
    {
        settings.insert("model".into(), serde_json::json!(route.model));
    }
    request.to_string()
}

fn request_thread_id(line: &str) -> Option<String> {
    let request = serde_json::from_str::<serde_json::Value>(line).ok()?;
    if !matches!(
        request.get("method").and_then(|value| value.as_str()),
        Some("thread/resume" | "thread/fork")
    ) {
        return None;
    }
    request
        .get("params")?
        .get("threadId")?
        .as_str()
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 128
                && id
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
        .map(str::to_owned)
}

fn find_thread_rollout(root: &Path, thread_id: &str, depth: usize) -> Option<PathBuf> {
    if depth == 0 || !root.is_dir() {
        return None;
    }
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_thread_rollout(&path, thread_id, depth - 1) {
                return Some(found);
            }
        } else if path.extension().and_then(|value| value.to_str()) == Some("jsonl")
            && path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.contains(thread_id))
        {
            return Some(path);
        }
    }
    None
}

struct RolloutScan {
    provider: Option<String>,
    sanitized: String,
    removed: usize,
    foreign_tool_ids: bool,
}

// Third-party providers reuse the OpenAI wire format but emit `msg_`/`fco_` ids
// on tool and reasoning items. OpenAI's Responses API rejects those on resume,
// so detect them from the item content instead of trusting the recorded
// provider. That keeps a thread whose `session_meta` still says `openai` but
// whose tail was written by a compatible-mode provider from breaking GPT.
fn has_foreign_tool_ids(payload: &serde_json::Value) -> bool {
    if payload.get("type").and_then(|value| value.as_str()) == Some("message") {
        return false;
    }
    payload
        .get("id")
        .and_then(|value| value.as_str())
        .is_some_and(|id| id.starts_with("msg_") || id.starts_with("fco_"))
}

fn scan_rollout(content: &str) -> RolloutScan {
    let mut provider = None;
    let mut removed = 0;
    let mut foreign_tool_ids = false;
    let mut kept = Vec::new();
    for line in content.lines() {
        let value = serde_json::from_str::<serde_json::Value>(line).ok();
        if provider.is_none()
            && value
                .as_ref()
                .and_then(|value| value.get("type"))
                .and_then(|value| value.as_str())
                == Some("session_meta")
        {
            provider = value
                .as_ref()
                .and_then(|value| value.get("payload"))
                .and_then(|value| value.get("model_provider"))
                .and_then(|value| value.as_str())
                .map(str::to_owned);
        }
        let payload = value
            .as_ref()
            .filter(|value| {
                value.get("type").and_then(|value| value.as_str()) == Some("response_item")
            })
            .and_then(|value| value.get("payload"));
        if payload.is_some_and(has_foreign_tool_ids) {
            foreign_tool_ids = true;
        }
        let non_portable_response_item = payload.is_some_and(|payload| {
            payload.get("type").and_then(|value| value.as_str()) != Some("message")
                || !matches!(
                    payload.get("role").and_then(|value| value.as_str()),
                    Some("user" | "assistant")
                )
        }) || value.as_ref().is_some_and(|value| {
            value.get("type").and_then(|value| value.as_str()) == Some("response_item")
                && value.get("payload").is_none()
        });
        if non_portable_response_item {
            removed += 1;
        } else {
            kept.push(line);
        }
    }
    let mut sanitized = kept.join("\n");
    if content.ends_with('\n') {
        sanitized.push('\n');
    }
    RolloutScan {
        provider,
        sanitized,
        removed,
        foreign_tool_ids,
    }
}

fn now_unix_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn write_bridge_status(path: Option<&std::ffi::OsStr>, status: &serde_json::Value) {
    let Some(path) = path else { return };
    let Ok(_guard) = BRIDGE_STATUS_WRITE_LOCK.lock() else {
        return;
    };
    let path = PathBuf::from(path);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::write(&path, status.to_string()).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

fn request_bridge_status(line: &str) -> Option<serde_json::Value> {
    let request = serde_json::from_str::<serde_json::Value>(line).ok()?;
    let method = request.get("method")?.as_str()?;
    let state = match method {
        "thread/resume" | "thread/fork" => "preparing",
        "turn/start" => "working",
        _ => return None,
    };
    let params = request.get("params")?;
    Some(serde_json::json!({
        "state": state,
        "threadId": params.get("threadId").and_then(|value| value.as_str()),
        "message": if state == "preparing" { "正在转换旧聊天并准备当前连接。" } else { "请求已发送，正在等待模型响应。" },
        "updatedAtMs": now_unix_millis(),
    }))
}

fn redact_bridge_message(message: &str) -> String {
    let mut redacted = String::with_capacity(message.len());
    let mut remaining = message;
    while let Some(start) = remaining.find("sk-") {
        redacted.push_str(&remaining[..start]);
        let secret = &remaining[start..];
        let end = secret
            .char_indices()
            .skip(3)
            .find(|(_, character)| {
                !(character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
            })
            .map(|(index, _)| index)
            .unwrap_or(secret.len());
        redacted.push_str("sk-***");
        remaining = &secret[end..];
    }
    redacted.push_str(remaining);
    redacted
}

fn server_bridge_status(
    line: &str,
    retry_counts: &mut HashMap<String, u64>,
) -> Option<serde_json::Value> {
    let notification = serde_json::from_str::<serde_json::Value>(line).ok()?;
    let method = notification.get("method")?.as_str()?;
    let params = notification.get("params")?;
    let thread_id = params.get("threadId").and_then(|value| value.as_str());
    match method {
        "turn/started" => {
            let turn_id = params
                .get("turn")
                .and_then(|turn| turn.get("id"))
                .and_then(|value| value.as_str());
            if let Some(turn_id) = turn_id {
                retry_counts.remove(turn_id);
            }
            Some(serde_json::json!({
                "state": "working",
                "threadId": thread_id,
                "turnId": turn_id,
                "message": "模型已开始处理。",
                "updatedAtMs": now_unix_millis(),
            }))
        }
        "error" => {
            let error = params.get("error")?;
            let turn_id = params.get("turnId").and_then(|value| value.as_str());
            let will_retry = params
                .get("willRetry")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let retry_count = if will_retry {
                let count = retry_counts
                    .entry(turn_id.unwrap_or("").to_owned())
                    .or_default();
                *count += 1;
                Some(*count)
            } else {
                None
            };
            let message = redact_bridge_message(
                error
                    .get("message")
                    .and_then(|value| value.as_str())
                    .unwrap_or("模型请求失败。"),
            );
            Some(serde_json::json!({
                "state": if will_retry { "retrying" } else { "failed" },
                "threadId": thread_id,
                "turnId": turn_id,
                "message": message,
                "codexErrorInfo": error.get("codexErrorInfo"),
                "willRetry": will_retry,
                "retryCount": retry_count,
                "updatedAtMs": now_unix_millis(),
            }))
        }
        "turn/completed" => {
            let turn = params.get("turn")?;
            let turn_id = turn.get("id").and_then(|value| value.as_str());
            if let Some(turn_id) = turn_id {
                retry_counts.remove(turn_id);
            }
            let failed = turn.get("status").and_then(|value| value.as_str()) == Some("failed");
            let message = redact_bridge_message(
                turn.get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(|value| value.as_str())
                    .unwrap_or(if failed {
                        "任务处理失败。"
                    } else {
                        "任务处理完成。"
                    }),
            );
            Some(serde_json::json!({
                "state": if failed { "failed" } else { "completed" },
                "threadId": thread_id,
                "turnId": turn_id,
                "message": message,
                "updatedAtMs": now_unix_millis(),
            }))
        }
        _ => None,
    }
}

fn write_recoverable_replacement(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("聊天文件路径无效。")?;
    let temporary = parent.join(format!(
        ".status-floater-sanitized-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    let displaced = parent.join(format!(
        ".status-floater-original-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    std::fs::write(&temporary, content).map_err(|error| error.to_string())?;
    if let Ok(metadata) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&temporary, metadata.permissions());
    }
    std::fs::rename(path, &displaced).map_err(|error| error.to_string())?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::rename(&displaced, path);
        let _ = std::fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    std::fs::remove_file(displaced).map_err(|error| error.to_string())
}

fn sanitize_cross_provider_history(
    line: &str,
    route: &ActiveHistoryRoute,
) -> Result<usize, String> {
    let Some(thread_id) = request_thread_id(line) else {
        return Ok(0);
    };
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .or_else(|| std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".codex")))
        .ok_or("找不到 Codex 数据目录。")?;
    let active_root = std::fs::canonicalize(home.join("sessions")).ok();
    let archived_root = std::fs::canonicalize(home.join("archived_sessions")).ok();
    let path = find_thread_rollout(&home.join("sessions"), &thread_id, 5)
        .or_else(|| find_thread_rollout(&home.join("archived_sessions"), &thread_id, 2))
        .ok_or("找不到原聊天文件，已停止跨服务商恢复。")?;
    let path = std::fs::canonicalize(path).map_err(|error| error.to_string())?;
    if !active_root
        .as_ref()
        .is_some_and(|root| path.starts_with(root))
        && !archived_root
            .as_ref()
            .is_some_and(|root| path.starts_with(root))
    {
        return Err("聊天文件不在允许的数据目录内。".into());
    }
    let content = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let scan = scan_rollout(&content);
    let same_provider = scan.provider.as_deref() == Some(route.model_provider.as_str());
    // A thread that still records the target provider can nevertheless carry
    // foreign tool/reasoning ids from a compatible-mode provider, so only skip
    // when the provider matches *and* the content is actually portable.
    if scan.removed == 0 || (same_provider && !scan.foreign_tool_ids) {
        return Ok(0);
    }
    let sanitized = scan.sanitized;
    let removed = scan.removed;

    let backup_dir = home.join("status-floater/history-compat-backups");
    std::fs::create_dir_all(&backup_dir).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&backup_dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    let backup = backup_dir.join(format!("{thread_id}.before-cross-provider.jsonl"));
    if !backup.exists() {
        std::fs::write(&backup, content.as_bytes()).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o600))
                .map_err(|error| error.to_string())?;
        }
    }
    write_recoverable_replacement(&path, sanitized.as_bytes())?;
    Ok(removed)
}

// Only rewrite the implicit provider filter. Keep explicit filters and all other RPCs intact.
pub fn rewrite_history_request(line: &str) -> String {
    let Ok(mut request) = serde_json::from_str::<serde_json::Value>(line) else {
        return line.to_string();
    };
    if request.get("method").and_then(|value| value.as_str()) != Some("thread/list") {
        return line.to_string();
    }
    let Some(object) = request.as_object_mut() else {
        return line.to_string();
    };
    let params = object.entry("params").or_insert(serde_json::json!({}));
    if params.is_null() {
        *params = serde_json::json!({});
    }
    let Some(params) = params.as_object_mut() else {
        return line.to_string();
    };
    if params
        .get("modelProviders")
        .is_none_or(serde_json::Value::is_null)
    {
        params.insert("modelProviders".to_string(), serde_json::json!([]));
        return request.to_string();
    }
    line.to_string()
}

#[allow(dead_code)]
pub fn run() -> Result<i32, Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--upstream")) {
        return Err(
            "Usage: codex-history-bridge --upstream <codex-binary> [codex arguments]".into(),
        );
    }
    let upstream = arguments.next().ok_or("Missing upstream binary")?;
    run_upstream(upstream, arguments.collect())
}

pub fn run_upstream(
    upstream: std::ffi::OsString,
    arguments: Vec<std::ffi::OsString>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let upstream = std::fs::canonicalize(upstream)?;
    if upstream == std::fs::canonicalize(std::env::current_exe()?)? {
        return Err("Bridge cannot use itself as the upstream binary".into());
    }
    let is_app_server = arguments.iter().any(|arg| arg == "app-server");
    if !is_app_server {
        return Ok(Command::new(upstream)
            .args(arguments)
            .status()?
            .code()
            .unwrap_or(1));
    }
    // The caller chooses the original runtime. Do not substitute the floater's older sidecar.
    let mut child = Command::new(upstream)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let mut upstream_stdin = child.stdin.take().ok_or("Missing child stdin")?;
    let upstream_stdout = child.stdout.take().ok_or("Missing child stdout")?;
    if let Some(path) = std::env::var_os("STATUS_FLOATER_BRIDGE_ACK") {
        std::fs::write(
            path,
            serde_json::json!({"pid":std::process::id(),"active":true}).to_string(),
        )?;
    }
    let route_path = std::env::var_os("STATUS_FLOATER_HISTORY_ROUTE");
    let status_path = std::env::var_os("STATUS_FLOATER_BRIDGE_STATUS");
    let output_status_path = status_path.clone();
    std::thread::spawn(move || {
        let mut retry_counts = HashMap::new();
        for line in BufReader::new(upstream_stdout).lines() {
            let Ok(line) = line else { break };
            if let Some(status) = server_bridge_status(&line, &mut retry_counts) {
                write_bridge_status(output_status_path.as_deref(), &status);
            }
            let mut stdout = io::stdout().lock();
            if writeln!(stdout, "{line}").is_err() || stdout.flush().is_err() {
                break;
            }
        }
    });
    let initial_route = route_path.as_ref().and_then(|path| {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ActiveHistoryRoute>(&bytes).ok())
    });
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if let Some(status) = request_bridge_status(&line) {
                write_bridge_status(status_path.as_deref(), &status);
            }
            let rewritten = if let Some(path) = &route_path {
                let route = std::fs::read(path)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<ActiveHistoryRoute>(&bytes).ok());
                let Some(route) = route.filter(|route| {
                    !route.model.is_empty()
                        && !route.model_provider.is_empty()
                        && Some(route) == initial_route.as_ref()
                }) else {
                    // Fail closed instead of resuming with stale credentials or an old model.
                    if let Some(id) = serde_json::from_str::<serde_json::Value>(&line)
                        .ok()
                        .and_then(|value| value.get("id").cloned())
                    {
                        let _ = writeln!(
                            io::stdout().lock(),
                            "{}",
                            serde_json::json!({"id":id,"error":{"code":-32603,"message":"连接已变更或配置不可用，请在悬浮工具点击“立即重启 Codex 客户端”后再继续。"}})
                        );
                    }
                    continue;
                };
                if let Err(error) = sanitize_cross_provider_history(&line, &route) {
                    if let Some(id) = serde_json::from_str::<serde_json::Value>(&line)
                        .ok()
                        .and_then(|value| value.get("id").cloned())
                    {
                        let _ = writeln!(
                            io::stdout().lock(),
                            "{}",
                            serde_json::json!({"id":id,"error":{"code":-32603,"message":format!("旧聊天兼容转换失败，未继续打开：{error}")}})
                        );
                    }
                    continue;
                }
                rewrite_follow_current_request(&line, &route)
            } else {
                rewrite_history_request(&line)
            };
            if writeln!(upstream_stdin, "{}", rewritten).is_err() {
                break;
            }
            if upstream_stdin.flush().is_err() {
                break;
            }
        }
    });
    Ok(child.wait()?.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::rewrite_history_request;
    use serde_json::{json, Value};

    #[test]
    fn resume_and_turn_follow_the_current_connection_without_changing_history() {
        let route = super::ActiveHistoryRoute {
            model: "gpt-test".into(),
            model_provider: "openai".into(),
            connection_key: None,
        };
        let request = json!({"id":7,"method":"thread/resume","params":{"threadId":"original-task","model":"deepseek-test","modelProvider":"status_floater_api_test"}});
        let result: Value = serde_json::from_str(&super::rewrite_follow_current_request(
            &request.to_string(),
            &route,
        ))
        .unwrap();
        assert_eq!(result["params"]["threadId"], "original-task");
        assert_eq!(result["params"]["model"], "gpt-test");
        assert_eq!(result["params"]["modelProvider"], "openai");
        let turn = json!({"id":8,"method":"turn/start","params":{"threadId":"original-task","input":[{"type":"text","text":"Continue"}],"collaborationMode":{"settings":{"model":"deepseek-test"}}}});
        let result: Value = serde_json::from_str(&super::rewrite_follow_current_request(
            &turn.to_string(),
            &route,
        ))
        .unwrap();
        assert_eq!(result["params"]["input"], turn["params"]["input"]);
        assert_eq!(
            result["params"]["collaborationMode"]["settings"]["model"],
            "gpt-test"
        );
    }

    #[test]
    fn cross_provider_history_keeps_only_visible_user_and_assistant_messages() {
        let content = [
            json!({"type":"session_meta","payload":{"model_provider":"custom"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Question"}]}}),
            json!({"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"Provider instructions"}]}}),
            json!({"type":"response_item","payload":{"type":"reasoning","content":[{"type":"reasoning_text","text":"private"}],"encrypted_content":null}}),
            json!({"type":"response_item","payload":{"type":"reasoning","content":[],"encrypted_content":"ciphertext"}}),
            json!({"type":"response_item","payload":{"type":"function_call","id":"msg_third_party","call_id":"call_1","name":"exec_command","arguments":"{}"}}),
            json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"call_1","output":"private tool output"}}),
            json!({"type":"response_item","payload":{"type":"web_search_call","id":"msg_search"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Visible answer"}]}}),
        ]
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        let scan = super::scan_rollout(&content);
        assert_eq!(scan.provider.as_deref(), Some("custom"));
        assert_eq!(scan.removed, 6);
        assert!(scan.foreign_tool_ids);
        assert!(!scan.sanitized.contains("private"));
        assert!(!scan.sanitized.contains("ciphertext"));
        assert!(!scan.sanitized.contains("msg_third_party"));
        assert!(!scan.sanitized.contains("Provider instructions"));
        assert!(scan.sanitized.contains("Question"));
        assert!(scan.sanitized.contains("Visible answer"));
    }

    #[test]
    fn provider_mislabelled_thread_with_foreign_tool_ids_is_still_normalized() {
        // Mirrors the reported failure: `session_meta` says `openai` but the
        // tail carries `msg_`/`fco_` tool ids from a compatible-mode provider,
        // which GPT rejects with "Expected an ID that begins with 'fc'".
        let content = [
            json!({"type":"session_meta","payload":{"model_provider":"openai"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}),
            json!({"type":"response_item","payload":{"type":"reasoning","id":"msg_386fe015","content":[],"encrypted_content":"ciphertext"}}),
            json!({"type":"response_item","payload":{"type":"function_call","id":"msg_93dfe382","call_id":"call_1554331f","name":"exec_command","arguments":"{}"}}),
            json!({"type":"response_item","payload":{"type":"function_call_output","id":"fco_01a15daa","call_id":"call_1554331f","output":"tool output"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}}),
        ]
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        let scan = super::scan_rollout(&content);
        assert_eq!(scan.provider.as_deref(), Some("openai"));
        assert_eq!(scan.removed, 3);
        assert!(scan.foreign_tool_ids);
        assert!(!scan.sanitized.contains("msg_93dfe382"));
        assert!(!scan.sanitized.contains("fco_01a15daa"));
        assert!(!scan.sanitized.contains("ciphertext"));
        assert!(scan.sanitized.contains("hello"));
        assert!(scan.sanitized.contains("ok"));
    }

    #[test]
    fn genuine_openai_thread_with_fc_ids_is_left_untouched() {
        // A real OpenAI thread keeps `fc_`/`rs_` ids and must not be rewritten
        // just because it contains tool calls.
        let content = [
            json!({"type":"session_meta","payload":{"model_provider":"openai"}}),
            json!({"type":"response_item","payload":{"type":"function_call","id":"fc_abc","call_id":"call_1","name":"exec_command","arguments":"{}"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}}),
        ]
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        let scan = super::scan_rollout(&content);
        assert!(!scan.foreign_tool_ids);
    }

    #[test]
    fn bridge_status_reports_retry_progress_and_completion() {
        let mut retries = std::collections::HashMap::new();
        let retry = json!({
            "method":"error",
            "params":{
                "threadId":"thread-1",
                "turnId":"turn-1",
                "willRetry":true,
                "error":{"message":"high demand","codexErrorInfo":"serverOverloaded"}
            }
        });
        let status = super::server_bridge_status(&retry.to_string(), &mut retries).unwrap();
        assert_eq!(status["state"], "retrying");
        assert_eq!(status["retryCount"], 1);
        assert_eq!(status["message"], "high demand");

        assert_eq!(
            super::redact_bridge_message("Incorrect API key: sk-secret_value, retry"),
            "Incorrect API key: sk-***, retry"
        );

        let completed = json!({
            "method":"turn/completed",
            "params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"completed","error":null}}
        });
        let status = super::server_bridge_status(&completed.to_string(), &mut retries).unwrap();
        assert_eq!(status["state"], "completed");
        assert_eq!(status["message"], "任务处理完成。");
        assert!(!retries.contains_key("turn-1"));
    }

    #[test]
    fn default_provider_filters_include_all_without_changing_archive_or_pagination() {
        for params in [
            json!({"archived":true,"cursor":"page2","modelProviders":null}),
            json!({"archived":false}),
            Value::Null,
        ] {
            let original = json!({"id":11,"method":"thread/list","params":params});
            let rewritten: Value =
                serde_json::from_str(&rewrite_history_request(&original.to_string())).unwrap();
            assert_eq!(rewritten["id"], 11);
            assert_eq!(rewritten["params"]["modelProviders"], json!([]));
            if let Some(params) = original["params"].as_object() {
                for (key, value) in params {
                    if key != "modelProviders" {
                        assert_eq!(&rewritten["params"][key], value);
                    }
                }
            }
        }
    }

    #[test]
    fn explicit_filters_and_non_list_requests_remain_byte_for_byte_unchanged() {
        for request in [
            json!({"id":3,"method":"thread/list","params":{"modelProviders":["openai"]}}),
            json!({"id":3,"method":"thread/list","params":{"modelProviders":[]}}),
            json!({"id":3,"method":"thread/archive","params":{"threadId":"test"}}),
            json!({"id":3,"method":"turn/start","params":{"threadId":"test","input":[]}}),
            json!({"id":3,"method":"account/login/start","params":{"type":"chatgpt"}}),
        ] {
            let line = request.to_string();
            assert_eq!(rewrite_history_request(&line), line);
        }
        assert_eq!(rewrite_history_request("not-json"), "not-json");
    }
}
