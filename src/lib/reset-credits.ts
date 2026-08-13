import { invoke } from "@tauri-apps/api/core";

export type ResetCreditsSnapshot = {
  availableCount: number | null;
  expiresAt: string[];
  updatedAtMs: number;
};

export async function readCodexResetCredits() {
  return invoke<ResetCreditsSnapshot>("read_codex_reset_credits");
}
