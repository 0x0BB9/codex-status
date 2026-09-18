use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "macos")]
use std::time::Instant;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
#[cfg(target_os = "windows")]
use tauri::PhysicalPosition;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime, WebviewWindow, WindowEvent,
};
use tauri_plugin_global_shortcut::ShortcutState;
use toml_edit::{value, DocumentMut, Item, Table};

const MAIN_WINDOW_LABEL: &str = "main";
#[cfg(target_os = "macos")]
const CODEX_DESKTOP_BUNDLE_ID: &str = "com.openai.codex";
#[cfg(target_os = "macos")]
const CODEX_DESKTOP_PROCESS_NAMES: [&str; 2] = ["ChatGPT", "Codex"];
const TRAY_MENU_TOGGLE: &str = "toggle-window";
const TRAY_MENU_QUIT: &str = "quit";
const RESET_CREDITS_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
const API_KEYRING_SERVICE: &str = "io.github.codexstatus.floater.api-connections";
const API_PROVIDER_PREFIX: &str = "status_floater_api_";
const MAX_AUTH_BYTES: u64 = 256 * 1024;
const MAX_QUOTA_RESPONSE_BYTES: u64 = 1024 * 1024;
#[cfg(target_os = "windows")]
const GLOBAL_SHORTCUT: &str = "Ctrl+Alt+Space";
#[cfg(not(target_os = "windows"))]
const GLOBAL_SHORTCUT: &str = "Alt+Space";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct StoredCodexAccount {
    account_key: String,
    chatgpt_account_id: Option<String>,
    chatgpt_user_id: Option<String>,
    email: Option<String>,
    alias: Option<String>,
    account_name: Option<String>,
    plan: Option<String>,
    auth_mode: Option<String>,
    created_at: Option<i64>,
    last_used_at: Option<i64>,
    invalid_at: Option<i64>,
    invalid_reason: Option<String>,
    connection_kind: Option<String>,
    provider_id: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    workspace_name: Option<String>,
    workspace_id: Option<String>,
    key_hint: Option<String>,
    secret_id: Option<String>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct StoredCodexRegistry {
    schema_version: Option<u32>,
    active_account_key: Option<String>,
    active_account_activated_at_ms: Option<i64>,
    auto_switch: Option<Value>,
    api: Option<Value>,
    #[serde(default)]
    accounts: Vec<StoredCodexAccount>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
struct SaveCurrentAccountParams {
    alias: Option<String>,
}

#[derive(Debug, Serialize)]
struct AccountSwitchResult {
    backup_path: Option<String>,
    registry: StoredCodexRegistry,
}

#[derive(Debug, Serialize)]
struct AuthStorageConfigResult {
    backup_path: Option<String>,
    changed: bool,
    config_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveApiConnectionParams {
    profile_id: Option<String>,
    label: String,
    api_key: String,
    base_url: String,
    model: String,
    workspace_name: Option<String>,
    workspace_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiConnectionTestResult {
    model_count: usize,
    model_found: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResetCreditsSnapshot {
    available_count: Option<u64>,
    expires_at: Vec<String>,
    updated_at_ms: i64,
}

struct ResetCreditsAuth {
    access_token: String,
    account_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadBoardMetadata {
    pinned: bool,
    note: Option<String>,
    project: Option<String>,
    priority: Option<String>,
    stage: Option<String>,
    updated_at_ms: Option<i64>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadBoardState {
    schema_version: u32,
    #[serde(default)]
    threads: BTreeMap<String, ThreadBoardMetadata>,
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn now_unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn codex_home() -> Result<PathBuf, String> {
    if let Ok(custom_home) = std::env::var("CODEX_HOME") {
        if !custom_home.trim().is_empty() {
            let path = PathBuf::from(custom_home);
            if path.exists() {
                return Ok(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Unable to resolve Windows user profile directory.".to_string())?;

    #[cfg(not(target_os = "windows"))]
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "Unable to resolve home directory.".to_string())?;

    Ok(PathBuf::from(home).join(".codex"))
}

fn accounts_dir() -> Result<PathBuf, String> {
    Ok(codex_home()?.join("accounts"))
}

fn registry_path() -> Result<PathBuf, String> {
    Ok(accounts_dir()?.join("registry.json"))
}

fn active_auth_path() -> Result<PathBuf, String> {
    Ok(codex_home()?.join("auth.json"))
}

fn codex_config_path() -> Result<PathBuf, String> {
    Ok(codex_home()?.join("config.toml"))
}

fn floater_state_dir() -> Result<PathBuf, String> {
    Ok(codex_home()?.join("status-floater"))
}

fn thread_board_path() -> Result<PathBuf, String> {
    Ok(floater_state_dir()?.join("thread-board.json"))
}

fn chatgpt_config_snapshot_path() -> Result<PathBuf, String> {
    Ok(floater_state_dir()?.join("chatgpt-config.toml"))
}

fn connection_backups_dir() -> Result<PathBuf, String> {
    Ok(floater_state_dir()?.join("connection-backups"))
}

fn api_model_catalogs_dir() -> Result<PathBuf, String> {
    Ok(floater_state_dir()?.join("model-catalogs"))
}

fn read_registry() -> Result<StoredCodexRegistry, String> {
    let path = registry_path()?;
    if !path.exists() {
        return Ok(StoredCodexRegistry {
            schema_version: Some(4),
            ..StoredCodexRegistry::default()
        });
    }

    if let Some(parent) = path.parent() {
        ensure_private_directory(parent)?;
    }
    harden_private_file(&path)?;
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("Unable to read {}: {error}", path.display()))?;

    serde_json::from_str(&content)
        .map_err(|error| format!("Unable to parse {}: {error}", path.display()))
}

fn ensure_private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|error| format!("Unable to create {}: {error}", path.display()))?;

    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("Unable to secure {}: {error}", path.display()))?;

    Ok(())
}

fn harden_private_file(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("Unable to secure {}: {error}", path.display()))?;

    Ok(())
}

fn replace_json_file(tmp_path: &Path, path: &Path) -> Result<(), String> {
    // Windows cannot rename over an existing destination with std::fs::rename.
    #[cfg(target_os = "windows")]
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("Unable to replace {}: {error}", path.display()))?;
    }

    fs::rename(tmp_path, path)
        .map_err(|error| format!("Unable to replace {}: {error}", path.display()))
}

fn write_private_file(path: &Path, content: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        ensure_private_directory(parent)?;
    }

    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Invalid file path: {}", path.display()))?;
    let tmp_path = path.with_file_name(format!(".{filename}.{}.tmp", now_unix_millis()));

    fs::write(&tmp_path, content)
        .map_err(|error| format!("Unable to write {}: {error}", tmp_path.display()))?;
    harden_private_file(&tmp_path)?;
    replace_json_file(&tmp_path, path)?;
    harden_private_file(path)
}

fn write_json_file<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let content = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("Unable to serialize {}: {error}", path.display()))?;
    write_private_file(path, &content)
}

fn read_auth_file(path: &Path) -> Result<Value, String> {
    harden_private_file(path)?;
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Unable to inspect {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_AUTH_BYTES {
        return Err("Codex auth file is unavailable or unexpectedly large.".to_string());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Unable to read {}: {error}", path.display()))?;

    serde_json::from_str(&content)
        .map_err(|error| format!("Unable to parse {}: {error}", path.display()))
}

fn base64url_encode(input: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let bytes = input.as_bytes();
    let mut output = String::new();
    let mut index = 0;

    while index < bytes.len() {
        let b0 = bytes[index];
        let b1 = bytes.get(index + 1).copied();
        let b2 = bytes.get(index + 2).copied();

        output.push(ALPHABET[(b0 >> 2) as usize] as char);
        output.push(ALPHABET[(((b0 & 0b0000_0011) << 4) | b1.unwrap_or(0) >> 4) as usize] as char);

        if let Some(b1) = b1 {
            output.push(
                ALPHABET[(((b1 & 0b0000_1111) << 2) | b2.unwrap_or(0) >> 6) as usize] as char,
            );
        }

        if let Some(b2) = b2 {
            output.push(ALPHABET[(b2 & 0b0011_1111) as usize] as char);
        }

        index += 3;
    }

    output
}

fn base64url_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0u8;

    for byte in input.bytes().filter(|byte| *byte != b'=') {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return Err("Invalid base64url character.".to_string()),
        } as u32;

        buffer = (buffer << 6) | value;
        bits += 6;

        if bits >= 8 {
            bits -= 8;
            output.push(((buffer >> bits) & 0xff) as u8);
        }
    }

    Ok(output)
}

fn account_auth_path(account_key: &str) -> Result<PathBuf, String> {
    Ok(accounts_dir()?.join(format!("{}.auth.json", base64url_encode(account_key))))
}

fn jwt_payload(token: Option<&str>) -> Option<Value> {
    let token = token?;
    let payload = token.split('.').nth(1)?;
    let decoded = base64url_decode(payload).ok()?;
    serde_json::from_slice(&decoded).ok()
}

fn value_string<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for segment in path {
        current = current.get(segment)?;
    }

    current.as_str()
}

fn pick_string<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| value.get(*key)?.as_str())
}

fn reset_credits_auth(auth: &Value) -> Result<ResetCreditsAuth, String> {
    let tokens = auth.get("tokens").unwrap_or(auth);
    let access_token = pick_string(tokens, &["access_token", "accessToken"])
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "请先登录 ChatGPT/Codex 账号。".to_string())?
        .to_string();
    let account_id = pick_string(tokens, &["account_id", "accountId"])
        .map(str::to_string)
        .or_else(|| {
            jwt_payload(Some(&access_token)).and_then(|payload| {
                pick_string(
                    &payload,
                    &[
                        "https://api.openai.com/auth.chatgpt_account_id",
                        "chatgpt_account_id",
                    ],
                )
                .map(str::to_string)
            })
        });

    Ok(ResetCreditsAuth {
        access_token,
        account_id,
    })
}

fn reset_credits_headers(auth: &ResetCreditsAuth) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    let mut bearer = HeaderValue::from_str(&format!("Bearer {}", auth.access_token))
        .map_err(|_| "Codex 登录信息无效。".to_string())?;
    bearer.set_sensitive(true);
    headers.insert(AUTHORIZATION, bearer);
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert("originator", HeaderValue::from_static("Codex Desktop"));
    headers.insert("OAI-Product-Sku", HeaderValue::from_static("CODEX"));

    if let Some(account_id) = &auth.account_id {
        let mut value =
            HeaderValue::from_str(account_id).map_err(|_| "Codex 账号标识无效。".to_string())?;
        value.set_sensitive(true);
        headers.insert("ChatGPT-Account-Id", value);
    }

    Ok(headers)
}

fn unsigned_integer(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        let value = value.get(*key)?;
        value
            .as_u64()
            .or_else(|| value.as_i64().and_then(|item| u64::try_from(item).ok()))
    })
}

fn reset_credit_expiration(value: &Value) -> Option<String> {
    for key in [
        "expires_at",
        "expiresAt",
        "expiration_time",
        "expirationTime",
        "expires",
    ] {
        let Some(item) = value.get(key) else {
            continue;
        };
        if let Some(text) = item.as_str() {
            return Some(text.to_string());
        }
        if let Some(seconds) = item.as_i64() {
            return Some(seconds.to_string());
        }
        if let Some(seconds) = item.as_u64() {
            return Some(seconds.to_string());
        }
    }

    None
}

fn collect_reset_credit_expirations(value: &Value) -> Vec<String> {
    fn visit(value: &Value, output: &mut Vec<String>) {
        match value {
            Value::Array(items) => {
                for item in items {
                    visit(item, output);
                }
            }
            Value::Object(map) => {
                if let Some(expiration) = reset_credit_expiration(value) {
                    output.push(expiration);
                }
                for key in [
                    "credits",
                    "reset_credits",
                    "resetCredits",
                    "available",
                    "items",
                    "grants",
                ] {
                    if let Some(child) = map.get(key) {
                        visit(child, output);
                    }
                }
            }
            _ => {}
        }
    }

    let mut expirations = Vec::new();
    visit(value, &mut expirations);
    expirations.sort();
    expirations.dedup();
    expirations
}

fn parse_reset_credits(value: &Value) -> ResetCreditsSnapshot {
    let nested = value
        .get("rate_limit_reset_credits")
        .or_else(|| value.get("rateLimitResetCredits"));
    let available_count = unsigned_integer(
        value,
        &[
            "available_count",
            "availableCount",
            "remaining",
            "count",
            "quantity",
        ],
    )
    .or_else(|| {
        nested.and_then(|value| {
            unsigned_integer(
                value,
                &[
                    "available_count",
                    "availableCount",
                    "remaining",
                    "count",
                    "quantity",
                ],
            )
        })
    });

    ResetCreditsSnapshot {
        available_count,
        expires_at: collect_reset_credit_expirations(nested.unwrap_or(value)),
        updated_at_ms: now_unix_millis(),
    }
}

async fn limited_json(mut response: reqwest::Response) -> Result<Value, String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_QUOTA_RESPONSE_BYTES)
    {
        return Err("重置次数响应过大。".to_string());
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "读取重置次数响应失败。".to_string())?
    {
        if bytes.len().saturating_add(chunk.len()) as u64 > MAX_QUOTA_RESPONSE_BYTES {
            return Err("重置次数响应过大。".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }

    serde_json::from_slice(&bytes).map_err(|_| "重置次数响应格式已变化。".to_string())
}

#[tauri::command]
async fn read_codex_reset_credits() -> Result<ResetCreditsSnapshot, String> {
    let auth_path = active_auth_path()?;
    let auth_value = read_auth_file(&auth_path).map_err(|_| "请先登录 ChatGPT/Codex 账号。")?;
    let auth = reset_credits_auth(&auth_value)?;
    let headers = reset_credits_headers(&auth)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("CodexStatusFloater/0.1")
        .build()
        .map_err(|_| "无法初始化重置次数请求。".to_string())?;
    let response = client
        .get(RESET_CREDITS_URL)
        .headers(headers)
        .send()
        .await
        .map_err(|_| "暂时无法连接额度服务。".to_string())?;

    let snapshot = match response.status().as_u16() {
        200..=299 => parse_reset_credits(&limited_json(response).await?),
        401 | 403 => return Err("Codex 登录已失效，请重新登录。".to_string()),
        429 => return Err("额度服务请求频繁，稍后会自动重试。".to_string()),
        _ => return Err("额度服务暂时不可用。".to_string()),
    };

    Ok(snapshot)
}

fn stable_hash(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }

    format!("{hash:016x}")
}

fn is_api_connection(account: &StoredCodexAccount) -> bool {
    account.connection_kind.as_deref() == Some("api")
}

fn active_account_is_api(registry: &StoredCodexRegistry) -> bool {
    registry
        .active_account_key
        .as_deref()
        .and_then(|active_key| {
            registry
                .accounts
                .iter()
                .find(|account| account.account_key == active_key)
        })
        .is_some_and(is_api_connection)
}

fn api_secret_entry(secret_id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(API_KEYRING_SERVICE, secret_id)
        .map_err(|_| "无法访问系统凭据库。".to_string())
}

fn store_api_secret(secret_id: &str, api_key: &str) -> Result<(), String> {
    api_secret_entry(secret_id)?
        .set_password(api_key)
        .map_err(|_| "无法将 API Key 保存到系统凭据库。".to_string())
}

fn read_api_secret(secret_id: &str) -> Result<String, String> {
    api_secret_entry(secret_id)?
        .get_password()
        .map_err(|_| "系统凭据库中找不到该连接的 API Key，请重新导入。".to_string())
}

fn delete_api_secret(secret_id: &str) -> Result<(), String> {
    match api_secret_entry(secret_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("无法从系统凭据库删除 API Key。".to_string()),
    }
}

fn normalize_optional_text(value: Option<String>, max_chars: usize) -> Option<String> {
    value
        .map(|value| value.trim().chars().take(max_chars).collect::<String>())
        .filter(|value| !value.is_empty())
}

fn normalize_api_connection_params(
    params: SaveApiConnectionParams,
) -> Result<(SaveApiConnectionParams, String), String> {
    let label = params.label.trim().chars().take(80).collect::<String>();
    if label.is_empty() {
        return Err("请填写连接名称。".to_string());
    }

    let api_key = params.api_key.trim().to_string();
    if api_key.len() < 8 || api_key.len() > 4096 || api_key.chars().any(char::is_whitespace) {
        return Err("API Key 格式不正确。".to_string());
    }

    let mut base_url = reqwest::Url::parse(params.base_url.trim())
        .map_err(|_| "Base URL 格式不正确。".to_string())?;
    if base_url.scheme() != "https"
        || base_url.host_str().is_none()
        || !base_url.username().is_empty()
        || base_url.password().is_some()
        || base_url.query().is_some()
        || base_url.fragment().is_some()
    {
        return Err("Base URL 必须是不含账号、查询参数和片段的 HTTPS 地址。".to_string());
    }
    let normalized_path = base_url.path().trim_end_matches('/').to_string();
    base_url.set_path(if normalized_path.is_empty() {
        "/"
    } else {
        &normalized_path
    });
    let base_url = base_url.as_str().trim_end_matches('/').to_string();

    let model = params.model.trim().to_string();
    if model.is_empty()
        || model.len() > 160
        || !model
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-._/:".contains(character))
    {
        return Err("模型 ID 格式不正确。".to_string());
    }

    let profile_seed = format!(
        "{}|{}",
        base_url,
        params.workspace_id.as_deref().unwrap_or_default().trim()
    );
    let generated_profile_id = format!("api-profile::{}", stable_hash(&profile_seed));
    let normalized = SaveApiConnectionParams {
        profile_id: params.profile_id,
        label,
        api_key,
        base_url,
        model,
        workspace_name: normalize_optional_text(params.workspace_name, 120),
        workspace_id: normalize_optional_text(params.workspace_id, 160),
    };

    Ok((normalized, generated_profile_id))
}

fn configure_api_provider(
    existing_content: &str,
    account: &StoredCodexAccount,
    model_catalog_path: &Path,
    api_key: &str,
) -> Result<String, String> {
    let provider_id = account
        .provider_id
        .as_deref()
        .ok_or_else(|| "API 连接缺少 provider 配置。".to_string())?;
    let base_url = account
        .base_url
        .as_deref()
        .ok_or_else(|| "API 连接缺少 Base URL。".to_string())?;
    let model = account
        .model
        .as_deref()
        .ok_or_else(|| "API 连接缺少模型 ID。".to_string())?;
    let mut document = if existing_content.trim().is_empty() {
        DocumentMut::new()
    } else {
        existing_content
            .parse::<DocumentMut>()
            .map_err(|error| format!("Invalid Codex config TOML: {error}"))?
    };

    document["model"] = value(model);
    document["model_provider"] = value(provider_id);
    document["model_catalog_json"] = value(model_catalog_path.display().to_string());
    document["cli_auth_credentials_store"] = value("file");

    if !document.contains_key("model_providers") {
        document["model_providers"] = Item::Table(Table::new());
    }
    let providers = document["model_providers"]
        .as_table_mut()
        .ok_or_else(|| "Codex config 中的 model_providers 不是有效表。".to_string())?;
    let mut provider = Table::new();
    provider["name"] = value(
        account
            .alias
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or("API connection"),
    );
    provider["base_url"] = value(base_url);
    provider["wire_api"] = value("responses");
    provider["requires_openai_auth"] = value(false);
    provider["experimental_bearer_token"] = value(api_key);
    providers.insert(provider_id, Item::Table(provider));

    Ok(document.to_string())
}

fn redact_api_provider_secrets(existing_content: &str) -> Result<String, String> {
    if existing_content.trim().is_empty() {
        return Ok(String::new());
    }

    let mut document = existing_content
        .parse::<DocumentMut>()
        .map_err(|error| format!("Invalid Codex config TOML: {error}"))?;
    if let Some(providers) = document
        .get_mut("model_providers")
        .and_then(Item::as_table_mut)
    {
        for (_, provider) in providers.iter_mut() {
            if let Some(provider) = provider.as_table_mut() {
                provider.remove("experimental_bearer_token");
            }
        }
    }

    Ok(document.to_string())
}

fn api_model_catalog_path(account: &StoredCodexAccount) -> Result<PathBuf, String> {
    let provider_id = account
        .provider_id
        .as_deref()
        .ok_or_else(|| "API 连接缺少 provider 配置。".to_string())?;
    if !provider_id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err("API 连接的 provider 标识无效。".to_string());
    }

    Ok(api_model_catalogs_dir()?.join(format!("{provider_id}.json")))
}

fn build_api_model_catalog(account: &StoredCodexAccount) -> Result<Value, String> {
    let model = account
        .model
        .as_deref()
        .ok_or_else(|| "API 连接缺少模型 ID。".to_string())?;
    let display_name = account
        .alias
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or(model);

    Ok(serde_json::json!({
        "models": [{
            "slug": model,
            "display_name": display_name,
            "description": "Custom Responses API model managed by Codex Status Floater.",
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": [],
            "shell_type": "shell_command",
            "visibility": "list",
            "supported_in_api": true,
            "priority": 1,
            "base_instructions": "You are a coding assistant. Use the available tools carefully and follow the user's instructions.",
            "supports_parallel_tool_calls": false,
            "support_verbosity": false,
            "truncation_policy": {
                "mode": "tokens",
                "limit": 10000
            },
            "experimental_supported_tools": []
        }]
    }))
}

fn write_api_model_catalog(account: &StoredCodexAccount) -> Result<PathBuf, String> {
    let path = api_model_catalog_path(account)?;
    write_json_file(&path, &build_api_model_catalog(account)?)?;
    Ok(path)
}

fn restore_file(path: &Path, content: Option<&[u8]>) -> Result<(), String> {
    match content {
        Some(content) => write_private_file(path, content),
        None if path.exists() => fs::remove_file(path)
            .map_err(|error| format!("Unable to remove {}: {error}", path.display())),
        None => Ok(()),
    }
}

fn backup_connection_state(skip_auth: bool) -> Result<PathBuf, String> {
    let backup_root = connection_backups_dir()?.join(format!("switch-{}", now_unix_millis()));
    ensure_private_directory(&backup_root)?;

    for (source, filename) in [
        (codex_config_path()?, "config.toml"),
        (registry_path()?, "registry.json"),
    ] {
        if source.exists() {
            let mut content = fs::read(&source)
                .map_err(|error| format!("Unable to read {}: {error}", source.display()))?;
            if filename == "config.toml" {
                content =
                    redact_api_provider_secrets(&String::from_utf8_lossy(&content))?.into_bytes();
            }
            write_private_file(&backup_root.join(filename), &content)?;
        }
    }

    let auth_path = active_auth_path()?;
    if !skip_auth && auth_path.exists() {
        let content = fs::read(&auth_path)
            .map_err(|error| format!("Unable to read {}: {error}", auth_path.display()))?;
        write_private_file(&backup_root.join("auth.json"), &content)?;
    }

    Ok(backup_root)
}

fn auth_contains_chatgpt_tokens(content: &[u8]) -> bool {
    serde_json::from_slice::<Value>(content)
        .ok()
        .and_then(|auth| auth.get("tokens").cloned())
        .is_some_and(|tokens| tokens.is_object())
}

fn chatgpt_auth_for_api_switch(
    registry: &StoredCodexRegistry,
    current_auth: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    if let Some(current_auth) = current_auth.filter(|auth| auth_contains_chatgpt_tokens(auth)) {
        return Ok(current_auth.to_vec());
    }

    let mut candidates = registry
        .accounts
        .iter()
        .filter(|account| !is_api_connection(account))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|account| std::cmp::Reverse(account.last_used_at.unwrap_or_default()));

    for account in candidates {
        let path = account_auth_path(&account.account_key)?;
        let Ok(content) = fs::read(path) else {
            continue;
        };
        if auth_contains_chatgpt_tokens(&content) {
            return Ok(content);
        }
    }

    current_auth.map(ToOwned::to_owned).ok_or_else(|| {
        "找不到可保留的 Codex 登录状态，请先登录一个 Plus 账号后再切换 API 连接。".to_string()
    })
}

fn derive_account_from_auth(
    auth: &Value,
    registry: &StoredCodexRegistry,
    alias: Option<String>,
) -> Result<StoredCodexAccount, String> {
    let now = now_unix_seconds();
    let auth_mode = value_string(auth, &["auth_mode"])
        .or_else(|| {
            if auth.get("OPENAI_API_KEY").is_some() {
                Some("apiKey")
            } else {
                None
            }
        })
        .unwrap_or("chatgpt")
        .to_string();
    let account_id = value_string(auth, &["tokens", "account_id"]).map(str::to_string);
    let id_payload = jwt_payload(value_string(auth, &["tokens", "id_token"]));
    let email = id_payload
        .as_ref()
        .and_then(|payload| payload.get("email"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let account_name = id_payload
        .as_ref()
        .and_then(|payload| payload.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let jwt_user_id = id_payload
        .as_ref()
        .and_then(|payload| payload.get("sub"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let existing = account_id.as_ref().and_then(|id| {
        registry
            .accounts
            .iter()
            .find(|account| account.chatgpt_account_id.as_deref() == Some(id.as_str()))
            .cloned()
    });

    let openai_api_key = auth
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty());

    let account_key = existing
        .as_ref()
        .map(|account| account.account_key.clone())
        .or_else(|| {
            account_id
                .as_ref()
                .map(|id| format!("{}::{id}", jwt_user_id.as_deref().unwrap_or("local")))
        })
        .or_else(|| openai_api_key.map(|key| format!("api-key::{}", stable_hash(key))))
        .ok_or_else(|| {
            "Current ~/.codex/auth.json does not contain ChatGPT tokens or an API key.".to_string()
        })?;

    let mut account = existing.unwrap_or_else(|| StoredCodexAccount {
        account_key,
        created_at: Some(now),
        ..StoredCodexAccount::default()
    });

    account.chatgpt_account_id = account_id.or(account.chatgpt_account_id);
    account.chatgpt_user_id = jwt_user_id.or(account.chatgpt_user_id);
    account.email = email.or(account.email);
    account.account_name = account_name.or(account.account_name);
    account.auth_mode = Some(auth_mode);
    account.connection_kind = Some("chatgpt".to_string());
    account.last_used_at = Some(now);

    if let Some(alias) = alias
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        account.alias = Some(alias);
    }

    if account.email.is_none() && openai_api_key.is_some() {
        account.email = Some("API Key".to_string());
    }

    Ok(account)
}

fn upsert_account(registry: &mut StoredCodexRegistry, account: StoredCodexAccount) {
    if let Some(existing) = registry
        .accounts
        .iter_mut()
        .find(|existing| existing.account_key == account.account_key)
    {
        *existing = account;
    } else {
        registry.accounts.push(account);
    }
}

fn write_registry(registry: &StoredCodexRegistry) -> Result<(), String> {
    write_json_file(&registry_path()?, registry)
}

#[cfg(target_os = "windows")]
fn position_windows_floating_window<R: Runtime>(window: &WebviewWindow<R>) {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let (Some(monitor), Ok(size)) = (monitor, window.outer_size()) else {
        return;
    };
    let work_area = monitor.work_area();
    let margin = (12.0 * monitor.scale_factor()).round().max(12.0) as i32;
    let available_x = work_area.size.width.saturating_sub(size.width) as i32;
    let position = PhysicalPosition::new(
        work_area
            .position
            .x
            .saturating_add((available_x - margin).max(0)),
        work_area.position.y.saturating_add(margin),
    );

    let _ = window.set_position(position);
}

fn configure_main_window<R: Runtime>(window: &WebviewWindow<R>, position_window: bool) {
    let _ = window.set_always_on_top(true);

    #[cfg(not(target_os = "windows"))]
    let _ = position_window;

    #[cfg(target_os = "windows")]
    {
        let _ = window.set_skip_taskbar(true);
        if position_window {
            position_windows_floating_window(window);
        }
    }
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        configure_main_window(&window, false);
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn toggle_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            show_main_window(app);
        }
    }
}

fn setup_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(
        app,
        TRAY_MENU_TOGGLE,
        "显示/隐藏浮窗",
        true,
        Some(GLOBAL_SHORTCUT),
    )?;
    let quit = MenuItem::with_id(app, TRAY_MENU_QUIT, "退出", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&toggle, &separator, &quit])?;

    let mut tray = TrayIconBuilder::with_id("codex-status-floater")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Codex 状态浮窗")
        .on_menu_event(|app, event| {
            if event.id() == TRAY_MENU_TOGGLE {
                toggle_main_window(app);
            } else if event.id() == TRAY_MENU_QUIT {
                app.exit(0);
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button,
                button_state,
                ..
            } = event
            {
                if button == MouseButton::Left && button_state == MouseButtonState::Up {
                    toggle_main_window(tray.app_handle());
                }
            }
        });

    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }

    tray.build(app)?;
    Ok(())
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_optional_enum(value: Option<String>) -> Option<String> {
    normalize_optional_string(value).filter(|value| value != "none")
}

fn normalize_thread_board_metadata(mut metadata: ThreadBoardMetadata) -> ThreadBoardMetadata {
    metadata.note = normalize_optional_string(metadata.note);
    metadata.project = normalize_optional_string(metadata.project);
    metadata.priority = normalize_optional_enum(metadata.priority);
    metadata.stage = normalize_optional_enum(metadata.stage);
    metadata.updated_at_ms = Some(now_unix_millis());
    metadata
}

fn is_empty_thread_board_metadata(metadata: &ThreadBoardMetadata) -> bool {
    !metadata.pinned
        && metadata.note.is_none()
        && metadata.project.is_none()
        && metadata.priority.is_none()
        && metadata.stage.is_none()
}

fn read_thread_board_state() -> Result<ThreadBoardState, String> {
    let path = thread_board_path()?;
    if !path.exists() {
        return Ok(ThreadBoardState {
            schema_version: 1,
            threads: BTreeMap::new(),
        });
    }

    let content = fs::read_to_string(&path)
        .map_err(|error| format!("Unable to read {}: {error}", path.display()))?;

    serde_json::from_str(&content)
        .map_err(|error| format!("Unable to parse {}: {error}", path.display()))
}

fn write_thread_board_state(state: &ThreadBoardState) -> Result<(), String> {
    write_json_file(&thread_board_path()?, state)
}

#[tauri::command]
fn list_local_codex_accounts() -> Result<StoredCodexRegistry, String> {
    read_registry()
}

fn set_account_invalid_state(
    registry: &mut StoredCodexRegistry,
    account_key: &str,
    reason: Option<&str>,
) -> Result<(), String> {
    let account = registry
        .accounts
        .iter_mut()
        .find(|account| account.account_key == account_key)
        .ok_or_else(|| "Account is not saved in the local registry.".to_string())?;

    if let Some(reason) = reason {
        account.invalid_at = Some(now_unix_seconds());
        account.invalid_reason = Some(reason.trim().chars().take(240).collect());
    } else {
        account.invalid_at = None;
        account.invalid_reason = None;
    }
    registry.schema_version = Some(registry.schema_version.unwrap_or(4).max(4));
    Ok(())
}

#[tauri::command]
fn mark_local_codex_account_invalid(
    account_key: String,
    reason: String,
) -> Result<StoredCodexRegistry, String> {
    let mut registry = read_registry()?;
    set_account_invalid_state(&mut registry, &account_key, Some(&reason))?;
    write_registry(&registry)?;

    Ok(registry)
}

#[tauri::command]
fn clear_local_codex_account_invalid(account_key: String) -> Result<StoredCodexRegistry, String> {
    let mut registry = read_registry()?;
    set_account_invalid_state(&mut registry, &account_key, None)?;
    write_registry(&registry)?;

    Ok(registry)
}

fn remove_inactive_account(
    registry: &mut StoredCodexRegistry,
    account_key: &str,
) -> Result<(), String> {
    if registry.active_account_key.as_deref() == Some(account_key) {
        return Err("The active account cannot be deleted.".to_string());
    }

    let account_index = registry
        .accounts
        .iter()
        .position(|account| account.account_key == account_key)
        .ok_or_else(|| "Account is not saved in the local registry.".to_string())?;
    registry.accounts.remove(account_index);
    registry.schema_version = Some(registry.schema_version.unwrap_or(4).max(4));
    Ok(())
}

#[tauri::command]
fn delete_local_codex_account(account_key: String) -> Result<StoredCodexRegistry, String> {
    let mut registry = read_registry()?;
    let previous_registry = registry.clone();
    let removed_account = registry
        .accounts
        .iter()
        .find(|account| account.account_key == account_key)
        .cloned()
        .ok_or_else(|| "Account is not saved in the local registry.".to_string())?;
    remove_inactive_account(&mut registry, &account_key)?;
    write_registry(&registry)?;

    if let Some(secret_id) = removed_account
        .secret_id
        .as_deref()
        .filter(|_| is_api_connection(&removed_account))
    {
        if let Err(error) = delete_api_secret(secret_id) {
            let rollback_error = write_registry(&previous_registry).err();
            return Err(match rollback_error {
                Some(rollback_error) => {
                    format!("{error} 账号列表回滚也失败：{rollback_error}")
                }
                None => error,
            });
        }
    }

    let account_path = account_auth_path(&account_key)?;
    if account_path.exists() {
        if let Err(error) = fs::remove_file(&account_path) {
            let rollback_error = write_registry(&previous_registry).err();
            return Err(match rollback_error {
                Some(rollback_error) => format!(
                    "Unable to delete {}: {error}; registry rollback also failed: {rollback_error}",
                    account_path.display()
                ),
                None => format!("Unable to delete {}: {error}", account_path.display()),
            });
        }
    }

    if is_api_connection(&removed_account) {
        if let Ok(catalog_path) = api_model_catalog_path(&removed_account) {
            let _ = fs::remove_file(catalog_path);
        }
    }

    Ok(registry)
}

async fn test_api_connection_values(
    base_url: &str,
    model: &str,
    api_key: &str,
) -> Result<ApiConnectionTestResult, String> {
    let mut bearer = HeaderValue::from_str(&format!("Bearer {api_key}"))
        .map_err(|_| "API Key 格式不正确。".to_string())?;
    bearer.set_sensitive(true);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("CodexStatusFloater/0.1")
        .build()
        .map_err(|_| "无法初始化 API 连接测试。".to_string())?;
    let response = client
        .get(format!("{base_url}/models"))
        .header(AUTHORIZATION, bearer)
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|_| "无法连接 API 服务，请检查网络和 Base URL。".to_string())?;

    match response.status().as_u16() {
        200..=299 => {}
        401 | 403 => return Err("API Key 无效或无权访问该工作空间。".to_string()),
        429 => return Err("API 服务请求频繁或额度不足。".to_string()),
        status => return Err(format!("API 服务返回 HTTP {status}。")),
    }

    if response
        .content_length()
        .is_some_and(|length| length > MAX_QUOTA_RESPONSE_BYTES)
    {
        return Err("模型列表响应过大。".to_string());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "读取模型列表失败。".to_string())?;
    if bytes.len() as u64 > MAX_QUOTA_RESPONSE_BYTES {
        return Err("模型列表响应过大。".to_string());
    }
    let payload: Value =
        serde_json::from_slice(&bytes).map_err(|_| "模型列表响应格式不兼容。".to_string())?;
    let models = payload
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "模型列表响应缺少 data 字段。".to_string())?;
    let model_found = models
        .iter()
        .any(|item| item.get("id").and_then(Value::as_str) == Some(model));

    Ok(ApiConnectionTestResult {
        model_count: models.len(),
        model_found,
    })
}

#[tauri::command]
async fn save_api_connection_profile(
    params: SaveApiConnectionParams,
) -> Result<StoredCodexRegistry, String> {
    let (params, generated_profile_id) = normalize_api_connection_params(params)?;
    let test_result =
        test_api_connection_values(&params.base_url, &params.model, &params.api_key).await?;
    if !test_result.model_found {
        return Err(format!(
            "连接正常并读取到 {} 个模型，但没有找到目标模型 {}。",
            test_result.model_count, params.model
        ));
    }
    let mut registry = read_registry()?;
    let existing_index = if let Some(profile_id) = params.profile_id.as_deref() {
        let index = registry
            .accounts
            .iter()
            .position(|account| account.account_key == profile_id)
            .ok_or_else(|| "API 连接不存在。".to_string())?;
        if !is_api_connection(&registry.accounts[index]) {
            return Err("只能用此入口编辑 API 连接。".to_string());
        }
        Some(index)
    } else {
        registry.accounts.iter().position(|account| {
            is_api_connection(account)
                && account.base_url.as_deref() == Some(params.base_url.as_str())
                && account.workspace_id.as_deref() == params.workspace_id.as_deref()
        })
    };
    let account_key = existing_index
        .map(|index| registry.accounts[index].account_key.clone())
        .unwrap_or(generated_profile_id);
    let secret_id = existing_index
        .and_then(|index| registry.accounts[index].secret_id.clone())
        .unwrap_or_else(|| account_key.clone());
    let provider_id = existing_index
        .and_then(|index| registry.accounts[index].provider_id.clone())
        .unwrap_or_else(|| format!("{API_PROVIDER_PREFIX}{}", &stable_hash(&account_key)[..12]));
    let previous_secret = read_api_secret(&secret_id).ok();
    store_api_secret(&secret_id, &params.api_key)?;

    let key_suffix = params
        .api_key
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    let previous = existing_index.map(|index| registry.accounts[index].clone());
    let now = now_unix_seconds();
    let account = StoredCodexAccount {
        account_key: account_key.clone(),
        alias: Some(params.label),
        email: Some("API Key".to_string()),
        plan: Some("api".to_string()),
        auth_mode: Some("apikey".to_string()),
        connection_kind: Some("api".to_string()),
        provider_id: Some(provider_id),
        base_url: Some(params.base_url),
        model: Some(params.model),
        workspace_name: params.workspace_name,
        workspace_id: params.workspace_id,
        key_hint: Some(format!("...{key_suffix}")),
        secret_id: Some(secret_id.clone()),
        created_at: previous
            .as_ref()
            .and_then(|account| account.created_at)
            .or(Some(now)),
        last_used_at: previous.as_ref().and_then(|account| account.last_used_at),
        invalid_at: None,
        invalid_reason: None,
        ..StoredCodexAccount::default()
    };
    upsert_account(&mut registry, account);
    registry.schema_version = Some(registry.schema_version.unwrap_or(5).max(5));

    if let Err(error) = write_registry(&registry) {
        let _ = match previous_secret {
            Some(secret) => store_api_secret(&secret_id, &secret),
            None => delete_api_secret(&secret_id),
        };
        return Err(error);
    }

    Ok(registry)
}

#[tauri::command]
async fn test_api_connection_profile(
    account_key: String,
) -> Result<ApiConnectionTestResult, String> {
    let registry = read_registry()?;
    let account = registry
        .accounts
        .iter()
        .find(|account| account.account_key == account_key && is_api_connection(account))
        .ok_or_else(|| "API 连接不存在。".to_string())?;
    let secret_id = account
        .secret_id
        .as_deref()
        .ok_or_else(|| "API 连接缺少凭据引用。".to_string())?;
    let api_key = read_api_secret(secret_id)?;
    let base_url = account
        .base_url
        .as_deref()
        .ok_or_else(|| "API 连接缺少 Base URL。".to_string())?;
    let model = account.model.as_deref().unwrap_or_default();
    test_api_connection_values(base_url, model, &api_key).await
}

fn configure_file_auth_credentials_store(existing_content: &str) -> Result<(String, bool), String> {
    let mut document = if existing_content.trim().is_empty() {
        DocumentMut::new()
    } else {
        existing_content
            .parse::<DocumentMut>()
            .map_err(|error| format!("Invalid Codex config TOML: {error}"))?
    };

    if document
        .get("cli_auth_credentials_store")
        .and_then(|item| item.as_str())
        == Some("file")
    {
        return Ok((existing_content.to_string(), false));
    }

    document["cli_auth_credentials_store"] = value("file");
    Ok((document.to_string(), true))
}

#[tauri::command]
fn ensure_file_auth_credentials_store() -> Result<AuthStorageConfigResult, String> {
    let config_path = codex_config_path()?;
    let existing_content = if config_path.exists() {
        fs::read_to_string(&config_path)
            .map_err(|error| format!("Unable to read {}: {error}", config_path.display()))?
    } else {
        String::new()
    };
    let (updated_content, changed) = configure_file_auth_credentials_store(&existing_content)
        .map_err(|error| format!("Unable to update {}: {error}", config_path.display()))?;

    if !changed {
        return Ok(AuthStorageConfigResult {
            backup_path: None,
            changed: false,
            config_path: config_path.display().to_string(),
        });
    }

    let backup_path = if config_path.exists() && !existing_content.is_empty() {
        let backup_path = config_path.with_file_name("config.toml.bak.status-floater");
        if !backup_path.exists() {
            write_private_file(&backup_path, existing_content.as_bytes())?;
        }
        Some(backup_path)
    } else {
        None
    };

    write_private_file(&config_path, updated_content.as_bytes())?;

    Ok(AuthStorageConfigResult {
        backup_path: backup_path.map(|path| path.display().to_string()),
        changed: true,
        config_path: config_path.display().to_string(),
    })
}

#[tauri::command]
fn save_current_codex_account(
    params: Option<SaveCurrentAccountParams>,
) -> Result<StoredCodexRegistry, String> {
    let mut registry = read_registry()?;
    registry.schema_version = Some(registry.schema_version.unwrap_or(4).max(4));

    if let Some(active_index) = registry
        .active_account_key
        .as_deref()
        .and_then(|active_key| {
            registry
                .accounts
                .iter()
                .position(|account| account.account_key == active_key && is_api_connection(account))
        })
    {
        registry.accounts[active_index].last_used_at = Some(now_unix_seconds());
        write_registry(&registry)?;
        return Ok(registry);
    }

    let auth_path = active_auth_path()?;
    let auth = read_auth_file(&auth_path)?;
    let account =
        derive_account_from_auth(&auth, &registry, params.and_then(|params| params.alias))?;
    let account_key = account.account_key.clone();
    let account_path = account_auth_path(&account_key)?;

    write_json_file(&account_path, &auth)?;
    upsert_account(&mut registry, account);
    registry.active_account_key = Some(account_key);
    registry.active_account_activated_at_ms = Some(now_unix_millis());
    write_registry(&registry)?;

    Ok(registry)
}

fn sync_active_account_snapshot(registry: &mut StoredCodexRegistry) -> Result<(), String> {
    let Some(active_account_key) = registry.active_account_key.clone() else {
        return Ok(());
    };
    if registry
        .accounts
        .iter()
        .find(|account| account.account_key == active_account_key)
        .is_some_and(is_api_connection)
    {
        return Ok(());
    }
    let auth_path = active_auth_path()?;
    if !auth_path.exists() {
        return Ok(());
    }

    let auth = read_auth_file(&auth_path)?;
    let Ok(account) = derive_account_from_auth(&auth, registry, None) else {
        return Ok(());
    };

    if account.account_key != active_account_key
        || !registry
            .accounts
            .iter()
            .any(|saved| saved.account_key == active_account_key)
    {
        return Ok(());
    }

    write_json_file(&account_auth_path(&active_account_key)?, &auth)?;
    upsert_account(registry, account);
    Ok(())
}

#[tauri::command]
fn switch_local_codex_account(account_key: String) -> Result<AccountSwitchResult, String> {
    let mut registry = read_registry()?;
    let previous_registry = registry.clone();
    let account_index = registry
        .accounts
        .iter()
        .position(|account| account.account_key == account_key)
        .ok_or_else(|| "Account is not saved in the local registry.".to_string())?;
    let target_account = registry.accounts[account_index].clone();
    let current_is_api = active_account_is_api(&registry);
    sync_active_account_snapshot(&mut registry)?;

    let auth_path = active_auth_path()?;
    let config_path = codex_config_path()?;
    let previous_auth = fs::read(&auth_path).ok();
    let previous_config = fs::read(&config_path).ok();
    let backup_path = backup_connection_state(current_is_api)?;
    let target_catalog_path = is_api_connection(&target_account)
        .then(|| api_model_catalog_path(&target_account))
        .transpose()?;
    let previous_target_catalog = target_catalog_path
        .as_ref()
        .and_then(|path| fs::read(path).ok());

    let (next_auth, next_config) = if is_api_connection(&target_account) {
        let secret_id = target_account
            .secret_id
            .as_deref()
            .ok_or_else(|| "API 连接缺少凭据引用。".to_string())?;
        let api_key = read_api_secret(secret_id)?;
        let baseline_path = chatgpt_config_snapshot_path()?;
        if !current_is_api {
            write_private_file(
                &baseline_path,
                previous_config.as_deref().unwrap_or_default(),
            )?;
        }
        let baseline = fs::read_to_string(&baseline_path)
            .map_err(|_| "找不到 ChatGPT 配置快照，已停止切换。".to_string())?;
        let catalog_path = write_api_model_catalog(&target_account)?;
        let config = configure_api_provider(&baseline, &target_account, &catalog_path, &api_key)?;
        let auth = chatgpt_auth_for_api_switch(&registry, previous_auth.as_deref())?;
        (auth, config.into_bytes())
    } else {
        let account_path = account_auth_path(&account_key)?;
        if !account_path.exists() {
            return Err(format!(
                "Saved auth snapshot is missing: {}",
                account_path.display()
            ));
        }
        let auth = fs::read(&account_path)
            .map_err(|error| format!("Unable to read {}: {error}", account_path.display()))?;
        let config = if current_is_api {
            fs::read(chatgpt_config_snapshot_path()?)
                .map_err(|_| "找不到 ChatGPT 配置快照，已停止切换。".to_string())?
        } else {
            previous_config.clone().unwrap_or_default()
        };
        (auth, config)
    };

    if let Err(error) = write_private_file(&config_path, &next_config)
        .and_then(|_| write_private_file(&auth_path, &next_auth))
    {
        let _ = restore_file(&config_path, previous_config.as_deref());
        let _ = restore_file(&auth_path, previous_auth.as_deref());
        if let Some(catalog_path) = target_catalog_path.as_deref() {
            let _ = restore_file(catalog_path, previous_target_catalog.as_deref());
        }
        return Err(error);
    }

    registry.active_account_key = Some(account_key.clone());
    registry.active_account_activated_at_ms = Some(now_unix_millis());
    registry.accounts[account_index].last_used_at = Some(now_unix_seconds());
    if let Err(error) = write_registry(&registry) {
        let config_rollback = restore_file(&config_path, previous_config.as_deref()).err();
        let auth_rollback = restore_file(&auth_path, previous_auth.as_deref()).err();
        let catalog_rollback = target_catalog_path.as_deref().and_then(|catalog_path| {
            restore_file(catalog_path, previous_target_catalog.as_deref()).err()
        });
        let registry_rollback = write_registry(&previous_registry).err();
        return Err(format!(
            "{error}{}{}{}{}",
            config_rollback
                .map(|error| format!("；配置回滚失败：{error}"))
                .unwrap_or_default(),
            auth_rollback
                .map(|error| format!("；认证回滚失败：{error}"))
                .unwrap_or_default(),
            catalog_rollback
                .map(|error| format!("；模型目录回滚失败：{error}"))
                .unwrap_or_default(),
            registry_rollback
                .map(|error| format!("；账号列表回滚失败：{error}"))
                .unwrap_or_default(),
        ));
    }

    Ok(AccountSwitchResult {
        backup_path: Some(backup_path.display().to_string()),
        registry,
    })
}

#[tauri::command]
fn restart_codex_desktop_client() -> Result<(), String> {
    restart_codex_desktop_client_for_platform()
}

#[cfg(target_os = "macos")]
fn macos_app_bundle_from_executable(executable: &Path) -> Option<PathBuf> {
    executable
        .ancestors()
        .find(|path| path.extension().and_then(|extension| extension.to_str()) == Some("app"))
        .map(Path::to_path_buf)
}

#[cfg(target_os = "macos")]
fn running_macos_codex_app_path() -> Option<PathBuf> {
    for process_name in CODEX_DESKTOP_PROCESS_NAMES {
        let Ok(pgrep_output) = Command::new("pgrep").args(["-x", process_name]).output() else {
            continue;
        };

        for pid in String::from_utf8_lossy(&pgrep_output.stdout).lines() {
            let Ok(ps_output) = Command::new("ps")
                .args(["-p", pid.trim(), "-o", "comm="])
                .output()
            else {
                continue;
            };

            let executable = PathBuf::from(String::from_utf8_lossy(&ps_output.stdout).trim());
            if let Some(app_path) = macos_app_bundle_from_executable(&executable) {
                if app_path.is_dir() {
                    return Some(app_path);
                }
            }
        }
    }

    None
}

#[cfg(target_os = "macos")]
fn installed_macos_codex_app_path() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(PathBuf::from(&home).join("Applications/Codex.app"));
        candidates.push(PathBuf::from(home).join("Applications/ChatGPT.app"));
    }
    candidates.push(PathBuf::from("/Applications/Codex.app"));
    candidates.push(PathBuf::from("/Applications/ChatGPT.app"));

    candidates.into_iter().find(|path| path.is_dir())
}

#[cfg(target_os = "macos")]
fn macos_codex_desktop_running() -> Option<bool> {
    let mut probe_available = false;

    for process_name in CODEX_DESKTOP_PROCESS_NAMES {
        if let Ok(output) = Command::new("pgrep").args(["-x", process_name]).output() {
            probe_available = true;
            if output.status.success() {
                return Some(true);
            }
        }
    }

    probe_available.then_some(false)
}

#[cfg(target_os = "macos")]
fn wait_for_macos_codex_desktop_state(expected_running: bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;

    loop {
        match macos_codex_desktop_running() {
            Some(running) if running == expected_running => return true,
            // If process inspection is unavailable, rely on the open/quit command status.
            None => return true,
            _ => {}
        }

        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(target_os = "macos")]
fn terminate_macos_codex_desktop() {
    for process_name in CODEX_DESKTOP_PROCESS_NAMES {
        let _ = Command::new("pkill")
            .args(["-TERM", "-x", process_name])
            .status();
    }
}

#[cfg(target_os = "macos")]
fn restart_codex_desktop_client_for_platform() -> Result<(), String> {
    let app_path = running_macos_codex_app_path().or_else(installed_macos_codex_app_path);
    let was_running = macos_codex_desktop_running().unwrap_or(true);

    if was_running {
        let quit_succeeded = Command::new("osascript")
            .arg("-e")
            .arg(format!(
                r#"tell application id "{CODEX_DESKTOP_BUNDLE_ID}" to quit"#
            ))
            .status()
            .is_ok_and(|status| status.success());

        if !quit_succeeded {
            terminate_macos_codex_desktop();
        }

        if !wait_for_macos_codex_desktop_state(false, Duration::from_secs(8)) {
            terminate_macos_codex_desktop();
            if !wait_for_macos_codex_desktop_state(false, Duration::from_secs(4)) {
                return Err("Codex desktop client did not finish quitting on macOS.".to_string());
            }
        }
    }

    let mut open_command = Command::new("open");
    open_command.arg("-n");
    let launch_target = if let Some(path) = &app_path {
        open_command.arg(path);
        path.display().to_string()
    } else {
        open_command.args(["-b", CODEX_DESKTOP_BUNDLE_ID]);
        format!("bundle id {CODEX_DESKTOP_BUNDLE_ID}")
    };

    let open_output = open_command
        .output()
        .map_err(|error| format!("Unable to open Codex desktop client: {error}"))?;

    if !open_output.status.success() {
        let stderr = String::from_utf8_lossy(&open_output.stderr)
            .trim()
            .to_string();
        return Err(format!(
            "Unable to open Codex desktop client from {launch_target}: {stderr}"
        ));
    }

    if !wait_for_macos_codex_desktop_state(true, Duration::from_secs(15)) {
        return Err(format!(
            "macOS accepted the launch request for {launch_target}, but the Codex desktop process did not start."
        ));
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn restart_codex_desktop_client_for_platform() -> Result<(), String> {
    const RESTART_SCRIPT: &str = r#"$app = Get-StartApps | Where-Object { $_.Name -in @('Codex', 'ChatGPT') } | Sort-Object { if ($_.Name -eq 'Codex') { 0 } else { 1 } } | Select-Object -First 1; if (-not $app) { Write-Error 'Unable to find Codex or ChatGPT in the Windows Start menu.'; exit 1 }; $processName = if ($app.Name -eq 'Codex') { 'Codex' } else { 'ChatGPT' }; Get-Process -Name $processName -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep -Milliseconds 1500; Start-Process explorer.exe "shell:AppsFolder\$($app.AppID)""#;

    let status = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            RESTART_SCRIPT,
        ])
        .status()
        .map_err(|error| format!("Unable to restart Codex desktop client: {error}"))?;

    if !status.success() {
        return Err("Unable to restart the Codex desktop client on Windows.".to_string());
    }

    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn restart_codex_desktop_client_for_platform() -> Result<(), String> {
    Err(
        "Restarting the Codex desktop client is currently supported on macOS and Windows."
            .to_string(),
    )
}

#[tauri::command]
fn list_thread_board_metadata() -> Result<ThreadBoardState, String> {
    read_thread_board_state()
}

#[tauri::command]
fn set_thread_board_metadata(
    thread_id: String,
    metadata: ThreadBoardMetadata,
) -> Result<ThreadBoardState, String> {
    let mut state = read_thread_board_state()?;
    state.schema_version = 1;
    let metadata = normalize_thread_board_metadata(metadata);

    if is_empty_thread_board_metadata(&metadata) {
        state.threads.remove(&thread_id);
    } else {
        state.threads.insert(thread_id, metadata);
    }

    write_thread_board_state(&state)?;
    Ok(state)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcut(GLOBAL_SHORTCUT)
                .expect("failed to configure global shortcut")
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        toggle_main_window(app);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            setup_tray(app.handle())?;
            if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
                configure_main_window(&window, true);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == MAIN_WINDOW_LABEL {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_local_codex_accounts,
            mark_local_codex_account_invalid,
            clear_local_codex_account_invalid,
            delete_local_codex_account,
            save_api_connection_profile,
            test_api_connection_profile,
            ensure_file_auth_credentials_store,
            save_current_codex_account,
            switch_local_codex_account,
            restart_codex_desktop_client,
            read_codex_reset_credits,
            list_thread_board_metadata,
            set_thread_board_metadata
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::macos_app_bundle_from_executable;
    use super::{
        build_api_model_catalog, configure_api_provider, configure_file_auth_credentials_store,
        normalize_api_connection_params, parse_reset_credits, redact_api_provider_secrets,
        remove_inactive_account, reset_credits_auth, set_account_invalid_state,
        SaveApiConnectionParams, StoredCodexAccount, StoredCodexRegistry,
    };
    use std::path::Path;
    #[cfg(target_os = "macos")]
    use std::path::PathBuf;

    #[test]
    fn adds_file_auth_store_without_removing_existing_config() {
        let existing = "# keep this comment\nmodel = \"gpt-5\"\n";
        let (updated, changed) =
            configure_file_auth_credentials_store(existing).expect("config should update");

        assert!(changed);
        assert!(updated.contains("# keep this comment"));
        assert!(updated.contains("model = \"gpt-5\""));
        assert!(updated.contains("cli_auth_credentials_store = \"file\""));
    }

    #[test]
    fn leaves_file_auth_store_unchanged() {
        let existing = "cli_auth_credentials_store = \"file\"\nmodel = \"gpt-5\"\n";
        let (updated, changed) =
            configure_file_auth_credentials_store(existing).expect("config should parse");

        assert!(!changed);
        assert_eq!(updated, existing);
    }

    #[test]
    fn replaces_non_file_auth_store() {
        let existing = "cli_auth_credentials_store = \"keyring\"\n";
        let (updated, changed) =
            configure_file_auth_credentials_store(existing).expect("config should update");

        assert!(changed);
        assert_eq!(updated, "cli_auth_credentials_store = \"file\"\n");
    }

    #[test]
    fn configures_api_provider_with_isolated_bearer_token() {
        let account = StoredCodexAccount {
            account_key: "api-profile::test".to_string(),
            alias: Some("DeepSeek API".to_string()),
            connection_kind: Some("api".to_string()),
            provider_id: Some("status_floater_api_test".to_string()),
            base_url: Some("https://example.com/compatible-mode/v1".to_string()),
            model: Some("deepseek-v4.1-flash".to_string()),
            ..StoredCodexAccount::default()
        };
        let configured = configure_api_provider(
            "model = \"gpt-5\"\n",
            &account,
            Path::new("/tmp/status-floater-api.json"),
            "test-bearer-token",
        )
        .expect("provider config should update");

        assert!(configured.contains("model = \"deepseek-v4.1-flash\""));
        assert!(configured.contains("model_provider = \"status_floater_api_test\""));
        assert!(configured.contains("wire_api = \"responses\""));
        assert!(configured.contains("requires_openai_auth = false"));
        assert!(configured.contains("experimental_bearer_token = \"test-bearer-token\""));
        assert!(configured.contains("model_catalog_json"));

        let redacted = redact_api_provider_secrets(&configured).expect("config should redact");
        assert!(!redacted.contains("test-bearer-token"));
        assert!(!redacted.contains("experimental_bearer_token"));
    }

    #[test]
    fn builds_a_codex_compatible_api_model_catalog() {
        let account = StoredCodexAccount {
            account_key: "api-profile::test".to_string(),
            alias: Some("DeepSeek API".to_string()),
            connection_kind: Some("api".to_string()),
            provider_id: Some("status_floater_api_test".to_string()),
            model: Some("deepseek-v4.1-flash".to_string()),
            ..StoredCodexAccount::default()
        };
        let catalog = build_api_model_catalog(&account).expect("catalog should build");
        let model = &catalog["models"][0];

        assert_eq!(model["slug"], "deepseek-v4.1-flash");
        assert_eq!(model["visibility"], "list");
        assert_eq!(model["supports_parallel_tool_calls"], false);
        assert!(model.get("base_instructions").is_some());
        assert!(!catalog.to_string().contains("api_key"));
    }

    #[test]
    fn normalizes_secure_api_connection_params() {
        let (params, profile_id) = normalize_api_connection_params(SaveApiConnectionParams {
            profile_id: None,
            label: " DeepSeek ".to_string(),
            api_key: "sk-test-value".to_string(),
            base_url: "https://example.com/compatible-mode/v1/".to_string(),
            model: "deepseek-v4.1-flash".to_string(),
            workspace_name: Some(" Workspace ".to_string()),
            workspace_id: Some(" workspace-1 ".to_string()),
        })
        .expect("params should normalize");

        assert_eq!(params.label, "DeepSeek");
        assert_eq!(params.base_url, "https://example.com/compatible-mode/v1");
        assert_eq!(params.workspace_name.as_deref(), Some("Workspace"));
        assert!(profile_id.starts_with("api-profile::"));
    }

    #[test]
    fn rejects_insecure_api_base_url() {
        let result = normalize_api_connection_params(SaveApiConnectionParams {
            profile_id: None,
            label: "DeepSeek".to_string(),
            api_key: "sk-test-value".to_string(),
            base_url: "http://example.com/v1".to_string(),
            model: "deepseek-v4.1-flash".to_string(),
            workspace_name: None,
            workspace_id: None,
        });

        assert!(result.is_err());
    }

    #[test]
    fn parses_reset_credit_count_and_unique_expirations() {
        let value = serde_json::json!({
            "available_count": 2,
            "credits": [
                { "expires_at": "2026-08-20T00:00:00Z" },
                { "expirationTime": 1787270400 },
                { "expires_at": "2026-08-20T00:00:00Z" }
            ]
        });
        let snapshot = parse_reset_credits(&value);

        assert_eq!(snapshot.available_count, Some(2));
        assert_eq!(
            snapshot.expires_at,
            vec!["1787270400".to_string(), "2026-08-20T00:00:00Z".to_string()]
        );
    }

    #[test]
    fn reads_reset_credit_auth_without_persisting_it() {
        let auth = serde_json::json!({
            "tokens": {
                "access_token": "header.payload.signature",
                "account_id": "account-123"
            }
        });
        let parsed = reset_credits_auth(&auth).expect("auth should parse");

        assert_eq!(parsed.access_token, "header.payload.signature");
        assert_eq!(parsed.account_id.as_deref(), Some("account-123"));
    }

    #[test]
    fn removes_only_inactive_saved_accounts() {
        let mut registry = StoredCodexRegistry {
            active_account_key: Some("active".to_string()),
            accounts: vec![
                StoredCodexAccount {
                    account_key: "active".to_string(),
                    ..StoredCodexAccount::default()
                },
                StoredCodexAccount {
                    account_key: "inactive".to_string(),
                    ..StoredCodexAccount::default()
                },
            ],
            ..StoredCodexRegistry::default()
        };

        assert!(remove_inactive_account(&mut registry, "active").is_err());
        remove_inactive_account(&mut registry, "inactive")
            .expect("inactive account should be removed");
        assert_eq!(registry.accounts.len(), 1);
        assert_eq!(registry.accounts[0].account_key, "active");
        assert_eq!(registry.schema_version, Some(4));
    }

    #[test]
    fn marks_and_clears_invalid_account_state() {
        let mut registry = StoredCodexRegistry {
            accounts: vec![StoredCodexAccount {
                account_key: "target".to_string(),
                ..StoredCodexAccount::default()
            }],
            ..StoredCodexRegistry::default()
        };

        set_account_invalid_state(&mut registry, "target", Some("  token expired  "))
            .expect("account should be marked invalid");
        assert!(registry.accounts[0].invalid_at.is_some());
        assert_eq!(
            registry.accounts[0].invalid_reason.as_deref(),
            Some("token expired")
        );

        set_account_invalid_state(&mut registry, "target", None)
            .expect("invalid state should clear");
        assert_eq!(registry.accounts[0].invalid_at, None);
        assert_eq!(registry.accounts[0].invalid_reason, None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolves_macos_app_bundle_from_executable() {
        let executable = Path::new("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT");
        assert_eq!(
            macos_app_bundle_from_executable(executable),
            Some(PathBuf::from("/Applications/ChatGPT.app"))
        );
    }
}
