import { invoke } from "@tauri-apps/api/core";

export type ThreadPriority = "none" | "high" | "medium" | "low";
export type ThreadStage = "none" | "todo" | "doing" | "waiting" | "done";

export type ThreadBoardMetadata = {
  note?: string | null;
  pinned: boolean;
  priority?: ThreadPriority | null;
  project?: string | null;
  stage?: ThreadStage | null;
  updatedAtMs?: number | null;
};

export type ThreadBoardState = {
  schemaVersion: number;
  threads: Record<string, ThreadBoardMetadata>;
};

export const EMPTY_THREAD_BOARD_METADATA: ThreadBoardMetadata = {
  note: "",
  pinned: false,
  priority: "none",
  project: "",
  stage: "none",
};

export async function listThreadBoardMetadata() {
  return invoke<ThreadBoardState>("list_thread_board_metadata");
}

export async function setThreadBoardMetadata(
  threadId: string,
  metadata: ThreadBoardMetadata,
) {
  return invoke<ThreadBoardState>("set_thread_board_metadata", {
    threadId,
    metadata,
  });
}
