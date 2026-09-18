import { invoke } from "@tauri-apps/api/core";

export type StoredCodexAccount = {
  account_key: string;
  account_name?: string | null;
  alias?: string | null;
  auth_mode?: string | null;
  chatgpt_account_id?: string | null;
  chatgpt_user_id?: string | null;
  created_at?: number | null;
  email?: string | null;
  invalid_at?: number | null;
  invalid_reason?: string | null;
  connection_kind?: "chatgpt" | "api" | null;
  provider_id?: string | null;
  base_url?: string | null;
  model?: string | null;
  workspace_name?: string | null;
  workspace_id?: string | null;
  key_hint?: string | null;
  secret_id?: string | null;
  last_used_at?: number | null;
  plan?: string | null;
};

export type StoredCodexRegistry = {
  accounts: StoredCodexAccount[];
  active_account_activated_at_ms?: number | null;
  active_account_key?: string | null;
  api?: unknown;
  auto_switch?: unknown;
  schema_version?: number | null;
};

export type AccountSwitchResult = {
  backup_path?: string | null;
  registry: StoredCodexRegistry;
};

export type AuthStorageConfigResult = {
  backup_path?: string | null;
  changed: boolean;
  config_path: string;
};

export type SaveApiConnectionParams = {
  profileId?: string | null;
  label: string;
  apiKey: string;
  baseUrl: string;
  model: string;
  workspaceName?: string | null;
  workspaceId?: string | null;
};

export type ApiConnectionTestResult = {
  modelCount: number;
  modelFound: boolean;
};

export async function listLocalCodexAccounts() {
  return invoke<StoredCodexRegistry>("list_local_codex_accounts");
}

export async function ensureFileAuthCredentialsStore() {
  return invoke<AuthStorageConfigResult>("ensure_file_auth_credentials_store");
}

export async function saveCurrentCodexAccount(alias?: string) {
  return invoke<StoredCodexRegistry>("save_current_codex_account", {
    params: { alias: alias?.trim() || null },
  });
}

export async function switchLocalCodexAccount(accountKey: string) {
  return invoke<AccountSwitchResult>("switch_local_codex_account", {
    accountKey,
  });
}

export async function markLocalCodexAccountInvalid(
  accountKey: string,
  reason: string,
) {
  return invoke<StoredCodexRegistry>("mark_local_codex_account_invalid", {
    accountKey,
    reason,
  });
}

export async function clearLocalCodexAccountInvalid(accountKey: string) {
  return invoke<StoredCodexRegistry>("clear_local_codex_account_invalid", {
    accountKey,
  });
}

export async function deleteLocalCodexAccount(accountKey: string) {
  return invoke<StoredCodexRegistry>("delete_local_codex_account", {
    accountKey,
  });
}

export async function restartCodexDesktopClient() {
  return invoke<void>("restart_codex_desktop_client");
}

export async function saveApiConnectionProfile(params: SaveApiConnectionParams) {
  return invoke<StoredCodexRegistry>("save_api_connection_profile", { params });
}

export async function testApiConnectionProfile(accountKey: string) {
  return invoke<ApiConnectionTestResult>("test_api_connection_profile", {
    accountKey,
  });
}
