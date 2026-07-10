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

export async function listLocalCodexAccounts() {
  return invoke<StoredCodexRegistry>("list_local_codex_accounts");
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

export async function restartCodexDesktopClient() {
  return invoke<void>("restart_codex_desktop_client");
}
