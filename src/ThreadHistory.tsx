import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Thread, ThreadReadResponse } from "./generated/v2";
import { listThreadHistory, type HistoryClient, type HistoryEntry } from "./lib/thread-history";
import { getErrorMessage } from "./lib/error-message";

type Props = {
  client: HistoryClient;
  busy: boolean;
  providerNames: Record<string, string>;
  onRestored: () => void;
  onBusyChange: (busy: boolean) => void;
};

export function ThreadHistory({ client, busy, providerNames, onRestored, onBusyChange }: Props) {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [scope, setScope] = useState("all");
  const [query, setQuery] = useState("");
  const [limit, setLimit] = useState(40);
  const [working, setWorking] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [selected, setSelected] = useState<Thread | null>(null);
  const [confirmRestore, setConfirmRestore] = useState<string | null>(null);
  const generation = useRef(0);
  const operation = useRef(false);

  useEffect(() => {
    generation.current += 1;
    operation.current = false;
    setEntries([]);
    setSelected(null);
    setConfirmRestore(null);
    setWorking(false);
    setMessage("");
    setError("");
    return () => { generation.current += 1; onBusyChange(false); };
  }, [client, onBusyChange]);

  async function run(action: () => Promise<void>) {
    if (busy || operation.current) return;
    operation.current = true;
    onBusyChange(true);
    setWorking(true);
    setError("");
    setMessage("");
    try { await action(); }
    catch (reason) { setError(getErrorMessage(reason, "读取历史记录失败。")); }
    finally { operation.current = false; setWorking(false); onBusyChange(false); }
  }

  const filtered = entries.filter(({ thread, archived }) =>
    (scope === "all" || archived === (scope === "archived")) &&
    [thread.name, thread.preview, thread.id, thread.modelProvider, providerNames[thread.modelProvider], thread.cwd]
      .join(" ").toLocaleLowerCase().includes(query.toLocaleLowerCase()),
  );

  return <section className="panel history-panel">
    <div className="section-header"><h2>找回聊天记录</h2>
      <button type="button" disabled={busy || working} onClick={() => void run(async () => {
        const current = generation.current;
        const result = await listThreadHistory(client);
        if (current !== generation.current) return;
        setEntries(result);
        setMessage(`已读取 ${result.length} 条记录，包含全部服务商及已归档任务。`);
      })}>{working ? "处理中..." : "读取 / 刷新历史"}</button>
    </div>
    <p className="section-note">跨服务商查询本机聊天历史。取消归档后，官方侧栏仍可能按服务商筛选；继续 API 对话前请切回对应连接。</p>
    <div className="toolbar">
      <input aria-label="搜索历史记录" placeholder="搜索标题、服务商或任务 ID" value={query}
        onChange={(event) => { setQuery(event.target.value); setLimit(40); }} />
      <select aria-label="历史归档状态" value={scope} onChange={(event) => { setScope(event.target.value); setLimit(40); }}>
        <option value="all">全部历史</option><option value="active">未归档</option><option value="archived">已归档</option>
      </select>
    </div>
    {message && <p role="status" className="section-note">{message}</p>}
    {error && <p role="alert" className="error-text">{error}</p>}
    <div className="thread-list">
      {filtered.slice(0, limit).map(({ thread, archived }) => <article className="thread-card" key={thread.id}>
        <h3>{thread.name || thread.preview.slice(0, 100) || "未命名任务"}</h3>
        <p className="thread-meta">{providerNames[thread.modelProvider] || thread.modelProvider} · {archived ? "已归档" : "未归档"}</p>
        <small className="history-id">{thread.id}</small>
        <div className="action-row">
          <button type="button" disabled={busy || working} onClick={() => void run(async () => {
            const result = await client.request<ThreadReadResponse>("thread/read", { threadId: thread.id, includeTurns: true });
            setSelected(result.thread);
          })}>查看正文</button>
          <button type="button" disabled={busy || working} onClick={() => void run(async () => {
            const result = await client.request<ThreadReadResponse>("thread/read", { threadId: thread.id, includeTurns: true });
            const path = await invoke<string>("export_thread_history", { threadId: thread.id, content: JSON.stringify({ archived, ...result }, null, 2) });
            setMessage(`已导出到本机：${path}`);
          })}>导出 JSON</button>
          {archived && <button type="button" disabled={busy || working} onClick={() => setConfirmRestore(thread.id)}>取消归档</button>}
        </div>
        {confirmRestore === thread.id && <div className="action-row">
          <span>确认恢复这条任务到未归档列表？</span>
          <button type="button" disabled={busy || working} onClick={() => void run(async () => {
            await client.request("thread/unarchive", { threadId: thread.id });
            setConfirmRestore(null);
            setEntries(await listThreadHistory(client));
            onRestored();
            setMessage("已取消归档。服务商筛选仍可能影响官方客户端中的可见性。");
          })}>确认恢复</button>
          <button type="button" onClick={() => setConfirmRestore(null)}>取消</button>
        </div>}
      </article>)}
    </div>
    {filtered.length > limit && <button type="button" onClick={() => setLimit(limit + 40)}>显示更多（共 {filtered.length} 条）</button>}
    {selected && <div className="history-reader">
      <div className="section-header"><h3>{selected.name || "聊天正文"}</h3><button type="button" onClick={() => setSelected(null)}>关闭正文</button></div>
      {selected.turns.length === 0 && <p>此记录未返回对话正文，可尝试导出或检查本地备份。</p>}
      {selected.turns.flatMap((turn) => turn.items).filter((item) => item.type === "userMessage" || item.type === "agentMessage").map((item, index) => <div key={`${item.id}-${index}`}>
        <strong>{item.type === "userMessage" ? "用户" : "助手"}</strong>
        <pre>{item.type === "agentMessage" ? item.text : item.type === "userMessage" ? item.content.map((part) => part.type === "text" ? part.text : `[${part.type}]`).join("\n") : ""}</pre>
      </div>)}
      <p className="section-note">此处展示用户和助手消息；导出 JSON 包含接口返回的完整任务内容。</p>
    </div>}
  </section>;
}
