use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime, WindowEvent,
};
use tauri_plugin_global_shortcut::ShortcutState;

const MAIN_WINDOW_LABEL: &str = "main";
const CODEX_DESKTOP_BUNDLE_ID: &str = "com.openai.codex";
const TRAY_MENU_TOGGLE: &str = "toggle-window";
const TRAY_MENU_QUIT: &str = "quit";

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
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
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

fn floater_state_dir() -> Result<PathBuf, String> {
    Ok(codex_home()?.join("status-floater"))
}

fn thread_board_path() -> Result<PathBuf, String> {
    Ok(floater_state_dir()?.join("thread-board.json"))
}

fn read_registry() -> Result<StoredCodexRegistry, String> {
    let path = registry_path()?;
    if !path.exists() {
        return Ok(StoredCodexRegistry {
            schema_version: Some(3),
            ..StoredCodexRegistry::default()
        });
    }

    let content =
        fs::read_to_string(&path).map_err(|error| format!("Unable to read {}: {error}", path.display()))?;

    serde_json::from_str(&content)
        .map_err(|error| format!("Unable to parse {}: {error}", path.display()))
}

fn write_json_file<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Unable to create {}: {error}", parent.display()))?;
    }

    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Invalid file path: {}", path.display()))?;
    let tmp_path = path.with_file_name(format!(".{filename}.{}.tmp", now_unix_millis()));
    let content = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("Unable to serialize {}: {error}", path.display()))?;

    fs::write(&tmp_path, content)
        .map_err(|error| format!("Unable to write {}: {error}", tmp_path.display()))?;
    fs::rename(&tmp_path, path)
        .map_err(|error| format!("Unable to replace {}: {error}", path.display()))
}

fn read_auth_file(path: &Path) -> Result<Value, String> {
    let content =
        fs::read_to_string(path).map_err(|error| format!("Unable to read {}: {error}", path.display()))?;

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
            output.push(ALPHABET[(((b1 & 0b0000_1111) << 2) | b2.unwrap_or(0) >> 6) as usize] as char);
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

fn stable_hash(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }

    format!("{hash:016x}")
}

fn backup_active_auth() -> Result<Option<PathBuf>, String> {
    let auth_path = active_auth_path()?;
    if !auth_path.exists() {
        return Ok(None);
    }

    let backup_path = accounts_dir()?.join(format!("auth.json.bak.{}", now_unix_millis()));
    fs::create_dir_all(
        backup_path
            .parent()
            .ok_or_else(|| format!("Invalid backup path: {}", backup_path.display()))?,
    )
    .map_err(|error| format!("Unable to create backup directory: {error}"))?;
    fs::copy(&auth_path, &backup_path).map_err(|error| {
        format!(
            "Unable to back up {} to {}: {error}",
            auth_path.display(),
            backup_path.display()
        )
    })?;

    Ok(Some(backup_path))
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
    account.last_used_at = Some(now);

    if let Some(alias) = alias.map(|value| value.trim().to_string()).filter(|value| !value.is_empty()) {
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

fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
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
        Some("Alt+Space"),
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

    let content =
        fs::read_to_string(&path).map_err(|error| format!("Unable to read {}: {error}", path.display()))?;

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

#[tauri::command]
fn save_current_codex_account(params: Option<SaveCurrentAccountParams>) -> Result<StoredCodexRegistry, String> {
    let mut registry = read_registry()?;
    registry.schema_version = registry.schema_version.or(Some(3));

    let auth_path = active_auth_path()?;
    let auth = read_auth_file(&auth_path)?;
    let account = derive_account_from_auth(&auth, &registry, params.and_then(|params| params.alias))?;
    let account_key = account.account_key.clone();
    let account_path = account_auth_path(&account_key)?;

    write_json_file(&account_path, &auth)?;
    upsert_account(&mut registry, account);
    registry.active_account_key = Some(account_key);
    registry.active_account_activated_at_ms = Some(now_unix_millis());
    write_registry(&registry)?;

    Ok(registry)
}

#[tauri::command]
fn switch_local_codex_account(account_key: String) -> Result<AccountSwitchResult, String> {
    let mut registry = read_registry()?;
    let account_index = registry
        .accounts
        .iter()
        .position(|account| account.account_key == account_key)
        .ok_or_else(|| "Account is not saved in the local registry.".to_string())?;
    let account_path = account_auth_path(&account_key)?;

    if !account_path.exists() {
        return Err(format!("Saved auth snapshot is missing: {}", account_path.display()));
    }

    let auth = read_auth_file(&account_path)?;
    let backup_path = backup_active_auth()?;
    write_json_file(&active_auth_path()?, &auth)?;

    registry.active_account_key = Some(account_key.clone());
    registry.active_account_activated_at_ms = Some(now_unix_millis());
    registry.accounts[account_index].last_used_at = Some(now_unix_seconds());
    write_registry(&registry)?;

    Ok(AccountSwitchResult {
        backup_path: backup_path.map(|path| path.display().to_string()),
        registry,
    })
}

#[tauri::command]
fn restart_codex_desktop_client() -> Result<(), String> {
    let quit_status = Command::new("osascript")
        .arg("-e")
        .arg(format!(
            r#"tell application id "{CODEX_DESKTOP_BUNDLE_ID}" to quit"#
        ))
        .status();

    if !matches!(quit_status, Ok(status) if status.success()) {
        let _ = Command::new("pkill").args(["-TERM", "-x", "Codex"]).status();
    }

    thread::sleep(Duration::from_millis(1_500));

    let open_status = Command::new("open")
        .args(["-b", CODEX_DESKTOP_BUNDLE_ID])
        .status()
        .map_err(|error| format!("Unable to open Codex desktop client: {error}"))?;

    if !open_status.success() {
        return Err(format!(
            "Unable to open Codex desktop client with bundle id {CODEX_DESKTOP_BUNDLE_ID}."
        ));
    }

    Ok(())
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
                .with_shortcut("Alt+Space")
                .expect("failed to configure global shortcut")
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        toggle_main_window(app);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            setup_tray(app.handle())?;
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
            save_current_codex_account,
            switch_local_codex_account,
            restart_codex_desktop_client,
            list_thread_board_metadata,
            set_thread_board_metadata
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
