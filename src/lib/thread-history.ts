import type { ClientRequest } from "../generated";
import type { Thread, ThreadListParams, ThreadListResponse } from "../generated/v2";

export type HistoryClient = {
  listThreads(params: ThreadListParams): Promise<ThreadListResponse>;
  request<T>(method: ClientRequest["method"], params?: unknown): Promise<T>;
};

export type HistoryEntry = { thread: Thread; archived: boolean };

export async function listThreadHistory(client: Pick<HistoryClient, "listThreads">) {
  const entries = new Map<string, HistoryEntry>();
  for (const archived of [false, true]) {
    let cursor: string | null = null;
    const cursors = new Set<string>();
    do {
      const page = await client.listThreads({
        cursor, limit: 100, modelProviders: [], archived,
        sortKey: "updated_at", sortDirection: "desc",
      });
      for (const thread of page.data) entries.set(thread.id, { thread, archived });
      cursor = page.nextCursor;
      if (cursor && cursors.has(cursor)) throw new Error("历史列表分页异常，请重新连接后重试。");
      if (cursor) cursors.add(cursor);
    } while (cursor);
  }
  return [...entries.values()].sort((a, b) => b.thread.updatedAt - a.thread.updatedAt);
}

export function compareThreadHistory(before: HistoryEntry[], after: HistoryEntry[]) {
  const current = new Map(after.map((entry) => [entry.thread.id, entry]));
  return {
    missing: before.filter((entry) => !current.has(entry.thread.id)),
    newlyArchived: before.filter((entry) => !entry.archived && current.get(entry.thread.id)?.archived),
    providerChanged: before.filter((entry) => {
      const next = current.get(entry.thread.id);
      return next && next.thread.modelProvider !== entry.thread.modelProvider;
    }),
  };
}

export function describeHistoryChanges(before: HistoryEntry[], after: HistoryEntry[]) {
  const changes = compareThreadHistory(before, after);
  const details = [
    changes.missing.length ? `${changes.missing.length} 条记录暂时无法查询` : "",
    changes.newlyArchived.length ? `${changes.newlyArchived.length} 条记录变为已归档` : "",
    changes.providerChanged.length ? `${changes.providerChanged.length} 条记录的服务商发生变化` : "",
  ].filter(Boolean);
  return details.length
    ? `${details.join("；")}。可通过“找回记录”检查，本工具未自动改写归档状态。`
    : `已按 ID 核对 ${before.length} 条历史记录，均可查询，服务商与归档状态未发现上述异常。`;
}
