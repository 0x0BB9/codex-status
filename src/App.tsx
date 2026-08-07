import {
  startTransition,
  useDeferredValue,
  useEffect,
  useEffectEvent,
  useRef,
  useState,
} from "react";
import {
  availableMonitors,
  getCurrentWindow,
  PhysicalPosition,
  PhysicalSize,
} from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { PlanType, ServerNotification } from "./generated";
import type {
  Account,
  AccountLoginCompletedNotification,
  GetAccountRateLimitsResponse,
  LoginAccountParams,
  LoginAccountResponse,
  RateLimitSnapshot,
  Thread,
} from "./generated/v2";
import { CodexAppServerClient, DEFAULT_CODEX_WS_URL } from "./lib/codex-app-server";
import {
  ensureFileAuthCredentialsStore,
  listLocalCodexAccounts,
  restartCodexDesktopClient,
  saveCurrentCodexAccount,
  switchLocalCodexAccount,
  type StoredCodexRegistry,
} from "./lib/local-codex-accounts";
import {
  EMPTY_THREAD_BOARD_METADATA,
  listThreadBoardMetadata,
  setThreadBoardMetadata,
  type ThreadBoardMetadata,
  type ThreadBoardState,
  type ThreadPriority,
  type ThreadStage,
} from "./lib/thread-board";
import {
  CodexAppServerShellClient,
  isTauriRuntime,
} from "./lib/tauri-shell";
import "./App.css";

type ConnectionState = "disconnected" | "connecting" | "connected" | "error";
type FloatingMode = "expanded" | "peek";
type ThreadFilter = "all" | "recent" | "idle" | "systemError";
type ThreadGroupBy = "none" | "project" | "priority" | "stage";
type AccountLoginFlow = {
  authUrl?: string;
  loginId: string;
  startedAt: number;
  type: "chatgpt" | "chatgptDeviceCode";
  userCode?: string;
  verificationUrl?: string;
};
type DashboardClient = {
  cancelAccountLogin: (loginId: string) => Promise<unknown>;
  connect: (target: string) => Promise<unknown>;
  disconnect: (reason?: string) => void;
  getAccount: (refreshToken?: boolean) => Promise<{ account: Account | null }>;
  getRateLimits: () => Promise<GetAccountRateLimitsResponse>;
  isConnected: () => boolean;
  listThreads: (params: {
    limit: number;
    sortDirection: "desc";
    sortKey: "updated_at";
  }) => Promise<{ data: Thread[] }>;
  startAccountLogin: (params: LoginAccountParams) => Promise<LoginAccountResponse>;
};

const MAX_LOG_LINES = 6;
const RESTART_CODEX_CLIENT_AFTER_SWITCH_KEY =
  "codex-status-floater.restartCodexClientAfterSwitch";
const PLAN_TYPE_LABELS: Record<PlanType, string> = {
  free: "免费版",
  go: "Go",
  plus: "Plus",
  pro: "Pro",
  prolite: "Pro Lite",
  team: "Team",
  self_serve_business_usage_based: "Business 按量版",
  business: "Business",
  enterprise_cbp_usage_based: "Enterprise 按量版",
  enterprise: "Enterprise",
  edu: "教育版",
  unknown: "未知套餐",
};
const THREAD_FILTER_LABELS: Record<ThreadFilter, string> = {
  all: "全部",
  recent: "最近活跃",
  idle: "空闲",
  systemError: "异常",
};
const THREAD_GROUP_LABELS: Record<ThreadGroupBy, string> = {
  none: "不分组",
  project: "按项目",
  priority: "按优先级",
  stage: "按阶段",
};
const PRIORITY_LABELS: Record<ThreadPriority, string> = {
  none: "未设优先级",
  high: "高优先级",
  medium: "中优先级",
  low: "低优先级",
};
const STAGE_LABELS: Record<ThreadStage, string> = {
  none: "未设阶段",
  todo: "待处理",
  doing: "进行中",
  waiting: "等待中",
  done: "已完成",
};
const PRIORITY_ORDER: ThreadPriority[] = ["high", "medium", "low", "none"];
const STAGE_ORDER: ThreadStage[] = ["todo", "doing", "waiting", "done", "none"];
const RECENT_THREAD_ACTIVITY_SECONDS = 10 * 60;
const SNAP_EDGE_DISTANCE = 28;
const SNAP_SETTLE_DELAY_MS = 160;
const FLOATING_DIM_DELAY_MS = 2_500;
const FLOATING_COLLAPSE_DELAY_MS = 7_000;
const FLOATING_RIGHT_DOCK_TOLERANCE = SNAP_EDGE_DISTANCE + 8;
const FLOATING_PEEK_LOGICAL_SIZE = { width: 82, height: 168 };
const FLOATING_EXPANDED_MIN_LOGICAL_SIZE = { width: 360, height: 620 };
const FLOATING_DEFAULT_EXPANDED_LOGICAL_SIZE = { width: 430, height: 860 };
const IS_WINDOWS =
  typeof navigator !== "undefined" && navigator.userAgent.includes("Windows");

type AppWindow = ReturnType<typeof getCurrentWindow>;
type MonitorInfo = Awaited<ReturnType<typeof availableMonitors>>[number];
type FloatingWindowSnapshot = {
  position: {
    x: number;
    y: number;
  };
  size: {
    height: number;
    width: number;
  };
};

function formatPlanType(planType: PlanType | null | undefined) {
  if (!planType) {
    return "未知套餐";
  }

  return PLAN_TYPE_LABELS[planType] ?? planType;
}

function formatPercent(value: number | null | undefined) {
  if (value === null || value === undefined || Number.isNaN(value)) {
    return "暂无";
  }

  return `${Math.round(value)}%`;
}

function getUsageTone(value: number | null | undefined) {
  if (value === null || value === undefined || Number.isNaN(value)) {
    return "unknown";
  }

  if (value >= 90) {
    return "danger";
  }

  if (value >= 70) {
    return "warning";
  }

  return "safe";
}

function getUsageToneText(
  value: number | null | undefined,
  label = "短周期用量",
) {
  switch (getUsageTone(value)) {
    case "danger":
      return `${label}紧张`;
    case "warning":
      return `${label}偏高`;
    case "safe":
      return `${label}正常`;
    default:
      return `${label}未知`;
  }
}

function getCompactUsageToneText(value: number | null | undefined) {
  switch (getUsageTone(value)) {
    case "danger":
      return "紧张";
    case "warning":
      return "偏高";
    case "safe":
      return "正常";
    default:
      return "未知";
  }
}

function formatRateLimitWindowSummaryLabel(
  window: { windowDurationMins: number | null } | null | undefined,
  fallback = "短周期用量",
) {
  const durationMins = window?.windowDurationMins;
  if (!durationMins) {
    return fallback;
  }

  if (durationMins === 300) {
    return "五小时用量";
  }

  if (durationMins === 10_080) {
    return "一周用量";
  }

  if (durationMins % 10_080 === 0) {
    return `${durationMins / 10_080} 周用量`;
  }

  if (durationMins % 1_440 === 0) {
    return `${durationMins / 1_440} 天用量`;
  }

  if (durationMins % 60 === 0) {
    return `${durationMins / 60} 小时用量`;
  }

  return `${durationMins} 分钟用量`;
}

function formatRateLimitWindowCompactLabel(
  window: { windowDurationMins: number | null } | null | undefined,
) {
  const durationMins = window?.windowDurationMins;
  if (!durationMins) {
    return "短周期";
  }

  if (durationMins === 300) {
    return "5小时";
  }

  if (durationMins === 10_080) {
    return "1周";
  }

  if (durationMins % 10_080 === 0) {
    return `${durationMins / 10_080}周`;
  }

  if (durationMins % 1_440 === 0) {
    return `${durationMins / 1_440}天`;
  }

  if (durationMins % 60 === 0) {
    return `${durationMins / 60}小时`;
  }

  return `${durationMins}分钟`;
}

function getGlobalPrimaryUsage(rateLimits: RateLimitSnapshot[]) {
  const primaryWindows = rateLimits
    .map((snapshot) => ({
      poolLabel: snapshot.limitName ?? snapshot.limitId ?? "默认额度池",
      window: snapshot.primary,
    }))
    .filter((entry) => entry.window?.usedPercent !== undefined);
  const fiveHourWindows = primaryWindows.filter(
    (entry) => entry.window?.windowDurationMins === 300,
  );
  const candidates = fiveHourWindows.length > 0 ? fiveHourWindows : primaryWindows;
  const tightest = candidates.reduce<(typeof candidates)[number] | null>(
    (current, entry) => {
      if (!current) {
        return entry;
      }

      return (entry.window?.usedPercent ?? -1) > (current.window?.usedPercent ?? -1)
        ? entry
        : current;
    },
    null,
  );
  const selectedWindow = tightest?.window ?? null;
  const usedPercent = selectedWindow?.usedPercent ?? null;

  return {
    compactLabel: formatRateLimitWindowCompactLabel(selectedWindow),
    label: formatRateLimitWindowSummaryLabel(selectedWindow),
    poolLabel: tightest?.poolLabel ?? null,
    resetsAt: selectedWindow?.resetsAt ?? null,
    tone: getUsageTone(usedPercent),
    usedPercent,
  };
}

function clamp(value: number, min: number, max: number) {
  if (max < min) {
    return min;
  }

  return Math.min(Math.max(value, min), max);
}

function getNearestMonitor(
  position: PhysicalPosition,
  width: number,
  height: number,
  monitors: MonitorInfo[],
) {
  const centerX = position.x + width / 2;
  const centerY = position.y + height / 2;

  return monitors.reduce<MonitorInfo | null>((nearest, monitor) => {
    if (!nearest) {
      return monitor;
    }

    const monitorCenterX = monitor.workArea.position.x + monitor.workArea.size.width / 2;
    const monitorCenterY = monitor.workArea.position.y + monitor.workArea.size.height / 2;
    const nearestCenterX = nearest.workArea.position.x + nearest.workArea.size.width / 2;
    const nearestCenterY = nearest.workArea.position.y + nearest.workArea.size.height / 2;
    const monitorDistance =
      (centerX - monitorCenterX) ** 2 + (centerY - monitorCenterY) ** 2;
    const nearestDistance =
      (centerX - nearestCenterX) ** 2 + (centerY - nearestCenterY) ** 2;

    return monitorDistance < nearestDistance ? monitor : nearest;
  }, null);
}

async function snapWindowToScreenEdge(appWindow: AppWindow) {
  const [position, size, monitors] = await Promise.all([
    appWindow.outerPosition(),
    appWindow.outerSize(),
    availableMonitors(),
  ]);
  const monitor = getNearestMonitor(position, size.width, size.height, monitors);

  if (!monitor) {
    return;
  }

  const minX = monitor.workArea.position.x;
  const minY = monitor.workArea.position.y;
  const maxX = minX + monitor.workArea.size.width - size.width;
  const maxY = minY + monitor.workArea.size.height - size.height;
  const rightEdge = minX + monitor.workArea.size.width;
  const bottomEdge = minY + monitor.workArea.size.height;
  let nextX = clamp(position.x, minX, maxX);
  let nextY = clamp(position.y, minY, maxY);

  if (Math.abs(position.x - minX) <= SNAP_EDGE_DISTANCE) {
    nextX = minX;
  } else if (Math.abs(position.x + size.width - rightEdge) <= SNAP_EDGE_DISTANCE) {
    nextX = maxX;
  }

  if (Math.abs(position.y - minY) <= SNAP_EDGE_DISTANCE) {
    nextY = minY;
  } else if (Math.abs(position.y + size.height - bottomEdge) <= SNAP_EDGE_DISTANCE) {
    nextY = maxY;
  }

  nextX = Math.round(nextX);
  nextY = Math.round(nextY);

  if (nextX !== position.x || nextY !== position.y) {
    await appWindow.setPosition(new PhysicalPosition(nextX, nextY));
  }
}

function getPhysicalSizeFromLogical(
  logicalSize: { height: number; width: number },
  monitor: MonitorInfo,
) {
  return new PhysicalSize(
    Math.round(logicalSize.width * monitor.scaleFactor),
    Math.round(logicalSize.height * monitor.scaleFactor),
  );
}

function getRightDockPosition(
  monitor: MonitorInfo,
  width: number,
  height: number,
  preferredY: number,
) {
  const minY = monitor.workArea.position.y;
  const maxY = minY + monitor.workArea.size.height - height;

  return new PhysicalPosition(
    Math.round(monitor.workArea.position.x + monitor.workArea.size.width - width),
    Math.round(clamp(preferredY, minY, maxY)),
  );
}

function isNearRightDock(
  position: PhysicalPosition,
  width: number,
  monitor: MonitorInfo,
) {
  const rightEdge = monitor.workArea.position.x + monitor.workArea.size.width;

  return Math.abs(position.x + width - rightEdge) <= FLOATING_RIGHT_DOCK_TOLERANCE;
}

function formatRelativeTime(timestampSeconds: number) {
  const formatter = new Intl.RelativeTimeFormat("zh-CN", { numeric: "auto" });
  const diffSeconds = Math.round(timestampSeconds - Date.now() / 1000);

  if (Math.abs(diffSeconds) < 60) {
    return formatter.format(diffSeconds, "second");
  }

  const diffMinutes = Math.round(diffSeconds / 60);
  if (Math.abs(diffMinutes) < 60) {
    return formatter.format(diffMinutes, "minute");
  }

  const diffHours = Math.round(diffMinutes / 60);
  if (Math.abs(diffHours) < 24) {
    return formatter.format(diffHours, "hour");
  }

  return formatter.format(Math.round(diffHours / 24), "day");
}

function formatResetTime(unixSeconds: number | null | undefined) {
  if (!unixSeconds) {
    return "暂无重置时间";
  }

  return new Intl.DateTimeFormat("zh-CN", {
    hour: "numeric",
    minute: "2-digit",
    month: "short",
    day: "numeric",
  }).format(unixSeconds * 1000);
}

function formatRateLimitWindowLabel(
  window: { windowDurationMins: number | null } | null | undefined,
  fallback: string,
) {
  const durationMins = window?.windowDurationMins;
  if (!durationMins) {
    return fallback;
  }

  if (durationMins === 300) {
    return "五小时使用量";
  }

  if (durationMins === 10_080) {
    return "一周使用量";
  }

  if (durationMins % 10_080 === 0) {
    return `${durationMins / 10_080} 周使用量`;
  }

  if (durationMins % 1_440 === 0) {
    return `${durationMins / 1_440} 天使用量`;
  }

  if (durationMins % 60 === 0) {
    return `${durationMins / 60} 小时使用量`;
  }

  return `${durationMins} 分钟使用量`;
}

function shortenPath(path: string) {
  const segments = path.split("/").filter(Boolean);
  return segments.slice(-2).join("/") || path;
}

function getThreadLabel(thread: Thread) {
  return thread.name?.trim() || thread.preview?.trim() || "未命名任务";
}

function isRecentlyUpdatedThread(thread: Thread) {
  const ageSeconds = Date.now() / 1000 - thread.updatedAt;
  return ageSeconds >= 0 && ageSeconds <= RECENT_THREAD_ACTIVITY_SECONDS;
}

function getThreadStatusTone(thread: Thread) {
  const { status } = thread;

  switch (status.type) {
    case "active":
      if (status.activeFlags.includes("waitingOnApproval")) {
        return "approval";
      }

      if (status.activeFlags.includes("waitingOnUserInput")) {
        return "input";
      }

      return "active";
    case "systemError":
      return "error";
    default:
      return isRecentlyUpdatedThread(thread) ? "recent" : status.type === "idle" ? "idle" : "not-loaded";
  }
}

function getThreadStatusText(thread: Thread) {
  const { status } = thread;

  switch (status.type) {
    case "active":
      if (status.activeFlags.includes("waitingOnApproval")) {
        return "等待审批";
      }

      if (status.activeFlags.includes("waitingOnUserInput")) {
        return "等待输入";
      }

      return "进行中";
    case "systemError":
      return "系统异常";
    case "idle":
      return isRecentlyUpdatedThread(thread) ? "最近活跃" : "空闲";
    case "notLoaded":
      return isRecentlyUpdatedThread(thread) ? "最近活跃" : "未加载";
    default:
      return "未知";
  }
}

function getThreadFilterKey(thread: Thread): ThreadFilter {
  if (thread.status.type === "systemError") {
    return "systemError";
  }

  if (thread.status.type === "active" || isRecentlyUpdatedThread(thread)) {
    return "recent";
  }

  return "idle";
}

function mergeRateLimitSnapshot(
  current: RateLimitSnapshot | null,
  incoming: RateLimitSnapshot,
) {
  if (!current) {
    return incoming;
  }

  return {
    limitId: incoming.limitId ?? current.limitId,
    limitName: incoming.limitName ?? current.limitName,
    primary: incoming.primary ?? current.primary,
    secondary: incoming.secondary ?? current.secondary,
    credits: incoming.credits ?? current.credits,
    individualLimit: incoming.individualLimit ?? current.individualLimit,
    planType: incoming.planType ?? current.planType,
    rateLimitReachedType:
      incoming.rateLimitReachedType ?? current.rateLimitReachedType,
  };
}

function normalizeRateLimits(payload: GetAccountRateLimitsResponse) {
  const entries = Object.entries(payload.rateLimitsByLimitId ?? {});
  if (entries.length === 0) {
    return [payload.rateLimits];
  }

  return entries
    .map(([, snapshot]) => snapshot)
    .filter((snapshot): snapshot is RateLimitSnapshot => Boolean(snapshot));
}

function describeAccount(account: Account | null) {
  if (!account) {
    return "未登录";
  }

  switch (account.type) {
    case "chatgpt":
      return `${account.email} · ${formatPlanType(account.planType)}`;
    case "apiKey":
      return "API Key 模式";
    case "amazonBedrock":
      return "Amazon Bedrock";
    default:
      return "已连接";
  }
}

function formatEventName(method: string) {
  switch (method) {
    case "account/updated":
      return "账号已更新";
    case "account/rateLimits/updated":
      return "用量已更新";
    case "account/switched":
      return "账号已切换";
    case "account/switch-verified":
      return "账号切换已验证";
    case "account/switch-rolled-back":
      return "账号切换已回滚";
    case "account/saved":
      return "当前账号已保存";
    case "account/login/completed":
      return "账号登录已完成";
    case "account/login/started":
      return "账号登录已开始";
    case "codex-client/restarted":
      return "Codex 客户端已重启";
    case "thread/status/changed":
      return "任务状态已更新";
    case "thread/started":
      return "任务已启动";
    case "thread/archived":
      return "任务已归档";
    case "thread/unarchived":
      return "任务已恢复";
    case "thread/name/updated":
      return "任务名称已更新";
    case "thread/goal/updated":
      return "目标已更新";
    case "thread/goal/cleared":
      return "目标已清除";
    default:
      return method;
  }
}

function formatConnectionError(message: string | null) {
  if (!message) {
    return null;
  }

  if (message.includes("timed out waiting for Codex app-server")) {
    return "等待 Codex app-server 响应超时。";
  }

  if (message.startsWith("Unable to start codex app-server.")) {
    return "无法启动内置 Codex 服务。请重新安装应用或查看下方日志。";
  }

  switch (message) {
    case "Unable to connect to Codex app-server.":
      return "无法连接到 Codex app-server。";
    case "Timed out while connecting to Codex app-server.":
      return "连接 Codex app-server 超时。";
    case "WebSocket refused the connection.":
      return "WebSocket 连接被拒绝。";
    case "Codex app-server encountered a websocket error.":
      return "Codex app-server 的 WebSocket 连接异常。";
    case "Codex app-server exited.":
      return "Codex app-server 已退出。";
    case "Codex app-server is not running.":
    case "Not connected to Codex app-server.":
      return "Codex app-server 未运行。";
    case "The stdio transport is only available inside Tauri.":
      return "stdio 连接只能在 Tauri 客户端内使用。";
    case "Disconnected":
      return "已断开连接。";
    case "Reconnecting":
      return "正在重连。";
    default:
      return message;
  }
}

function getErrorMessage(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}

function formatLocalAccountError(error: unknown, fallback: string) {
  const message = getErrorMessage(error, fallback);

  if (message.includes("Current ~/.codex/auth.json")) {
    return "当前 ~/.codex/auth.json 里没有可保存的 ChatGPT 登录状态或 API Key。";
  }

  if (message.includes("Saved auth snapshot is missing")) {
    return "目标账号的本地认证快照不存在，无法切换。";
  }

  if (message.includes("Account is not saved in the local registry")) {
    return "账号列表中找不到这个账号，请刷新账号列表后重试。";
  }

  if (message.includes("Unable to read") || message.includes("Unable to parse")) {
    return `本地 Codex 账号文件读取失败：${message}`;
  }

  return message;
}

function formatRateLimitError(message: string | null) {
  if (!message) {
    return null;
  }

  if (
    message.includes("failed to fetch codex rate limits") ||
    message.includes("/backend-api/wham/usage")
  ) {
    return "用量接口暂不可用，不影响账号和任务状态。可能是当前账号、网络或 Codex 服务端限制。";
  }

  if (message.includes("account/rateLimits/read timed out")) {
    return "读取用量超时，不影响账号和任务状态。";
  }

  return `用量暂不可用：${message}`;
}

function formatAuthPlan(plan: string | null | undefined) {
  if (!plan) {
    return "未知套餐";
  }

  return PLAN_TYPE_LABELS[plan as PlanType] ?? plan;
}

function formatUnixTime(unixSeconds: number | null | undefined) {
  if (!unixSeconds) {
    return "暂无使用记录";
  }

  return new Intl.DateTimeFormat("zh-CN", {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(unixSeconds * 1000);
}

function getAuthAccountTitle(account: StoredCodexRegistry["accounts"][number]) {
  return (
    account.alias?.trim() ||
    account.account_name?.trim() ||
    account.email?.trim() ||
    "未命名账号"
  );
}

function normalizeBoardText(value: string | null | undefined) {
  return value?.trim() ?? "";
}

function normalizeThreadBoardMetadata(metadata: ThreadBoardMetadata) {
  return {
    pinned: Boolean(metadata.pinned),
    note: normalizeBoardText(metadata.note),
    project: normalizeBoardText(metadata.project),
    priority: metadata.priority ?? "none",
    stage: metadata.stage ?? "none",
    updatedAtMs: metadata.updatedAtMs ?? null,
  } satisfies ThreadBoardMetadata;
}

function getThreadBoardMetadata(
  boardState: ThreadBoardState,
  threadId: string,
) {
  return normalizeThreadBoardMetadata({
    ...EMPTY_THREAD_BOARD_METADATA,
    ...(boardState.threads[threadId] ?? {}),
  });
}

function getBoardChips(metadata: ThreadBoardMetadata) {
  const chips: Array<{ className: string; label: string }> = [];

  if (metadata.pinned) {
    chips.push({ className: "board-chip pinned", label: "关注" });
  }

  if (metadata.project) {
    chips.push({ className: "board-chip project", label: `项目 ${metadata.project}` });
  }

  if (metadata.priority && metadata.priority !== "none") {
    chips.push({
      className: `board-chip priority-${metadata.priority}`,
      label: PRIORITY_LABELS[metadata.priority],
    });
  }

  if (metadata.stage && metadata.stage !== "none") {
    chips.push({
      className: `board-chip stage-${metadata.stage}`,
      label: STAGE_LABELS[metadata.stage],
    });
  }

  return chips;
}

function hasBoardDetails(metadata: ThreadBoardMetadata) {
  return getBoardChips(metadata).length > 0 || Boolean(metadata.note);
}

function isEmptyBoardMetadata(metadata: ThreadBoardMetadata) {
  return (
    !metadata.pinned &&
    !metadata.note &&
    !metadata.project &&
    (!metadata.priority || metadata.priority === "none") &&
    (!metadata.stage || metadata.stage === "none")
  );
}

function mergeThreadBoardState(
  state: ThreadBoardState,
  threadId: string,
  metadata: ThreadBoardMetadata,
) {
  const threads = { ...state.threads };
  if (isEmptyBoardMetadata(metadata)) {
    delete threads[threadId];
  } else {
    threads[threadId] = metadata;
  }

  return {
    ...state,
    threads,
  };
}

function getThreadGroupLabel(metadata: ThreadBoardMetadata, groupBy: ThreadGroupBy) {
  switch (groupBy) {
    case "project":
      return metadata.project || "未分类项目";
    case "priority":
      return PRIORITY_LABELS[metadata.priority ?? "none"];
    case "stage":
      return STAGE_LABELS[metadata.stage ?? "none"];
    default:
      return "全部任务";
  }
}

function getThreadGroupSortValue(label: string, metadata: ThreadBoardMetadata, groupBy: ThreadGroupBy) {
  switch (groupBy) {
    case "priority":
      return PRIORITY_ORDER.indexOf(metadata.priority ?? "none");
    case "stage":
      return STAGE_ORDER.indexOf(metadata.stage ?? "none");
    case "project":
      return label === "未分类项目" ? `~${label}` : label;
    default:
      return 0;
  }
}

function App() {
  const clientRef = useRef<DashboardClient | null>(null);
  const collapseTimerRef = useRef<number | null>(null);
  const dimTimerRef = useRef<number | null>(null);
  const lastExpandedWindowRef = useRef<FloatingWindowSnapshot | null>(null);
  const [connectionState, setConnectionState] =
    useState<ConnectionState>("disconnected");
  const serverUrl = DEFAULT_CODEX_WS_URL;
  const [account, setAccount] = useState<Account | null>(null);
  const [rateLimits, setRateLimits] = useState<RateLimitSnapshot[]>([]);
  const [threads, setThreads] = useState<Thread[]>([]);
  const [lastSyncAt, setLastSyncAt] = useState<number | null>(null);
  const [lastEvent, setLastEvent] = useState<string>("暂无事件");
  const [lastError, setLastError] = useState<string | null>(null);
  const [rateLimitError, setRateLimitError] = useState<string | null>(null);
  const [authRegistry, setAuthRegistry] = useState<StoredCodexRegistry | null>(null);
  const [authError, setAuthError] = useState<string | null>(null);
  const [threadBoard, setThreadBoard] = useState<ThreadBoardState>({
    schemaVersion: 1,
    threads: {},
  });
  const [threadBoardError, setThreadBoardError] = useState<string | null>(null);
  const [serverLogs, setServerLogs] = useState<string[]>([]);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<ThreadFilter>("all");
  const [groupBy, setGroupBy] = useState<ThreadGroupBy>("none");
  const [editingThreadId, setEditingThreadId] = useState<string | null>(null);
  const [floatingMode, setFloatingMode] = useState<FloatingMode>("expanded");
  const [isFloatingDimmed, setIsFloatingDimmed] = useState(false);
  const [isLaunching, setIsLaunching] = useState(false);
  const [isStartingAccountLogin, setIsStartingAccountLogin] = useState(false);
  const [isSavingAccount, setIsSavingAccount] = useState(false);
  const [isRestartingCodexClient, setIsRestartingCodexClient] = useState(false);
  const [accountLoginFlow, setAccountLoginFlow] = useState<AccountLoginFlow | null>(
    null,
  );
  const [accountLoginMessage, setAccountLoginMessage] = useState<string | null>(null);
  const [codexClientRestartMessage, setCodexClientRestartMessage] = useState<
    string | null
  >(null);
  const [restartCodexClientAfterSwitch, setRestartCodexClientAfterSwitch] =
    useState(() => {
      if (typeof window === "undefined") {
        return true;
      }

      const storedValue = window.localStorage.getItem(
        RESTART_CODEX_CLIENT_AFTER_SWITCH_KEY,
      );
      return storedValue === null ? true : storedValue === "true";
    });
  const [switchingAccountKey, setSwitchingAccountKey] = useState<string | null>(null);
  const isTauri = isTauriRuntime();
  const deferredQuery = useDeferredValue(query);

  const appendServerLog = useEffectEvent((line: string) => {
    const normalized = line.trim();
    if (!normalized) {
      return;
    }

    startTransition(() => {
      setServerLogs((current) => [...current, normalized].slice(-MAX_LOG_LINES));
    });
  });

  const loadAuthAccounts = useEffectEvent(async () => {
    if (!isTauri) {
      return;
    }

    try {
      const registry = await listLocalCodexAccounts();
      startTransition(() => {
        setAuthRegistry(registry);
        setAuthError(null);
      });
    } catch (error) {
      startTransition(() => {
        setAuthError(formatLocalAccountError(error, "无法读取本地 Codex 账号列表。"));
      });
    }
  });

  const loadThreadBoard = useEffectEvent(async () => {
    if (!isTauri) {
      return;
    }

    try {
      const state = await listThreadBoardMetadata();
      startTransition(() => {
        setThreadBoard(state);
        setThreadBoardError(null);
      });
    } catch (error) {
      startTransition(() => {
        setThreadBoardError(getErrorMessage(error, "无法读取本地任务看板数据。"));
      });
    }
  });

  const persistThreadBoardMetadata = useEffectEvent(
    async (threadId: string, metadata: ThreadBoardMetadata) => {
      try {
        const state = await setThreadBoardMetadata(
          threadId,
          normalizeThreadBoardMetadata(metadata),
        );
        startTransition(() => {
          setThreadBoard(state);
          setThreadBoardError(null);
        });
      } catch (error) {
        startTransition(() => {
          setThreadBoardError(getErrorMessage(error, "保存任务看板数据失败。"));
        });
      }
    },
  );

  const updateThreadBoardMetadata = (
    threadId: string,
    patch: Partial<ThreadBoardMetadata>,
    options: { persist?: boolean } = {},
  ) => {
    const nextMetadata = normalizeThreadBoardMetadata({
      ...getThreadBoardMetadata(threadBoard, threadId),
      ...patch,
    });

    setThreadBoard((current) =>
      mergeThreadBoardState(current, threadId, nextMetadata),
    );

    if (options.persist !== false) {
      void persistThreadBoardMetadata(threadId, nextMetadata);
    }
  };

  const disconnectClient = useEffectEvent(async (reason = "Disconnected") => {
    const activeClient = clientRef.current;
    clientRef.current = null;

    if (activeClient) {
      activeClient.disconnect(reason);
    }

    setConnectionState("disconnected");
    if (reason !== "Disconnected" && reason !== "Reconnecting") {
      setLastError(reason);
    }
  });

  const refreshSnapshot = useEffectEvent(async () => {
    const client = clientRef.current;
    if (!client || !client.isConnected()) {
      return;
    }

    const [accountResult, rateLimitResult, threadResult] = await Promise.allSettled([
      client.getAccount(),
      client.getRateLimits(),
      client.listThreads({
        limit: 40,
        sortKey: "updated_at",
        sortDirection: "desc",
      }),
    ]);

    startTransition(() => {
      if (accountResult.status === "fulfilled") {
        setAccount(accountResult.value.account);
      }

      if (rateLimitResult.status === "fulfilled") {
        setRateLimits(normalizeRateLimits(rateLimitResult.value));
        setRateLimitError(null);
      } else {
        setRateLimitError(
          getErrorMessage(rateLimitResult.reason, "无法读取 Codex 用量数据。"),
        );
      }

      if (threadResult.status === "fulfilled") {
        setThreads(threadResult.value.data);
      }

      if (accountResult.status === "fulfilled" || threadResult.status === "fulfilled") {
        setLastError(null);
      } else {
        setLastError("账号和任务状态暂时无法同步。");
      }

      setLastSyncAt(Date.now());
    });
  });

  const handleRestartCodexClient = useEffectEvent(async () => {
    if (!isTauri || isRestartingCodexClient) {
      return false;
    }

    setIsRestartingCodexClient(true);
    setAuthError(null);
    setCodexClientRestartMessage(null);

    try {
      await restartCodexDesktopClient();
      setCodexClientRestartMessage("已请求重启 Codex 客户端。");
      setLastEvent("codex-client/restarted");
      return true;
    } catch (error) {
      setAuthError(getErrorMessage(error, "重启 Codex 客户端失败。"));
      return false;
    } finally {
      setIsRestartingCodexClient(false);
    }
  });

  const handleAccountLoginCompleted = useEffectEvent(
    async (params: AccountLoginCompletedNotification) => {
      if (params.loginId && accountLoginFlow && accountLoginFlow.loginId !== params.loginId) {
        return;
      }

      setIsStartingAccountLogin(false);
      setAccountLoginFlow((current) => {
        if (!current || !params.loginId || current.loginId === params.loginId) {
          return null;
        }

        return current;
      });

      if (!params.success) {
        setAuthError(params.error || "账号登录失败。");
        setAccountLoginMessage(null);
        return;
      }

      setAccountLoginMessage("登录成功，正在保存账号快照...");
      setAuthError(null);

      try {
        const registry = await saveCurrentCodexAccount();
        setAuthRegistry(registry);
        setLastEvent("account/login/completed");
        setAccountLoginMessage("登录成功，已添加到账号列表并设为当前账号。");
        await refreshSnapshot();

        if (restartCodexClientAfterSwitch) {
          const restarted = await handleRestartCodexClient();
          if (restarted) {
            setCodexClientRestartMessage(
              "新账号已保存，已重启官方客户端。新版 ChatGPT 客户端如使用独立会话，可能仍需在客户端内确认账号。",
            );
          }
        } else {
          setCodexClientRestartMessage(
            "新账号已在浮窗和 CLI 中生效；官方桌面客户端尚未重启。",
          );
        }
      } catch (error) {
        setAuthError(formatLocalAccountError(error, "登录成功，但保存账号快照失败。"));
      }
    },
  );

  const handleNotification = useEffectEvent((notification: ServerNotification) => {
    startTransition(() => {
      setLastEvent(notification.method);
      setLastSyncAt(Date.now());
    });

    switch (notification.method) {
      case "account/updated":
        void refreshSnapshot();
        break;
      case "account/login/completed":
        void handleAccountLoginCompleted(notification.params);
        break;
      case "account/rateLimits/updated":
        startTransition(() => {
          setRateLimitError(null);
          setRateLimits((current) => {
            if (current.length === 0) {
              return [notification.params.rateLimits];
            }

            const targetKey =
              notification.params.rateLimits.limitId ??
              notification.params.rateLimits.limitName;
            let updated = false;
            const next = current.map((snapshot) => {
              const snapshotKey = snapshot.limitId ?? snapshot.limitName;
              if (snapshotKey === targetKey) {
                updated = true;
                return mergeRateLimitSnapshot(snapshot, notification.params.rateLimits);
              }

              return snapshot;
            });

            if (!updated) {
              next.unshift(notification.params.rateLimits);
            }

            return next;
          });
        });
        break;
      case "thread/status/changed":
        startTransition(() => {
          setThreads((current) =>
            current.map((thread) =>
              thread.id === notification.params.threadId
                ? { ...thread, status: notification.params.status }
                : thread,
            ),
          );
        });
        break;
      case "thread/started":
      case "thread/archived":
      case "thread/unarchived":
      case "thread/name/updated":
      case "thread/goal/updated":
      case "thread/goal/cleared":
        void refreshSnapshot();
        break;
      default:
        break;
    }
  });

  const connectToServer = useEffectEvent(
    async (options?: { launchIfNeeded?: boolean }) => {
      setConnectionState("connecting");
      setLastError(null);
      setRateLimitError(null);

      await disconnectClient("Reconnecting");

      const createClient = () =>
        new CodexAppServerClient({
          onNotification: handleNotification,
          onDisconnect: (reason) => {
            clientRef.current = null;
            setConnectionState("disconnected");
            if (reason !== "Disconnected") {
              setLastError(reason);
            }
          },
        });

      const createShellClient = () =>
        new CodexAppServerShellClient({
          onNotification: handleNotification,
          onDisconnect: (reason) => {
            clientRef.current = null;
            setConnectionState("disconnected");
            if (reason !== "Disconnected") {
              setLastError(reason);
            }
          },
          onServerLog: appendServerLog,
        });

      const attemptConnection = async () => {
        const nextClient = isTauri ? createShellClient() : createClient();
        if (isTauri) {
          await nextClient.connect(serverUrl);
        } else {
          await nextClient.connect(serverUrl);
        }
        clientRef.current = nextClient;
      };

      try {
        setIsLaunching(isTauri && Boolean(options?.launchIfNeeded));
        await attemptConnection();

        setConnectionState("connected");
        await refreshSnapshot();
      } catch (error) {
        clientRef.current = null;
        setConnectionState("error");
        setLastError(
          error instanceof Error ? error.message : "无法连接到 Codex app-server。",
        );
      } finally {
        setIsLaunching(false);
      }
    },
  );

  const clearFloatingIdleTimers = useEffectEvent(() => {
    if (dimTimerRef.current !== null) {
      window.clearTimeout(dimTimerRef.current);
      dimTimerRef.current = null;
    }

    if (collapseTimerRef.current !== null) {
      window.clearTimeout(collapseTimerRef.current);
      collapseTimerRef.current = null;
    }
  });

  const collapseFloatingWindow = useEffectEvent(
    async (options: { force?: boolean } = {}) => {
      if (!isTauri || floatingMode === "peek") {
        return;
      }

      try {
        const appWindow = getCurrentWindow();
        const [position, size, monitors] = await Promise.all([
          appWindow.outerPosition(),
          appWindow.outerSize(),
          availableMonitors(),
        ]);
        const monitor = getNearestMonitor(position, size.width, size.height, monitors);

        if (!monitor || (!options.force && !isNearRightDock(position, size.width, monitor))) {
          return;
        }

        clearFloatingIdleTimers();
        lastExpandedWindowRef.current = {
          position: {
            x: position.x,
            y: position.y,
          },
          size: {
            height: size.height,
            width: size.width,
          },
        };

        const peekSize = getPhysicalSizeFromLogical(
          FLOATING_PEEK_LOGICAL_SIZE,
          monitor,
        );
        const nextPosition = getRightDockPosition(
          monitor,
          peekSize.width,
          peekSize.height,
          position.y,
        );

        await appWindow.setMinSize(peekSize);
        await appWindow.setSize(peekSize);
        await appWindow.setPosition(nextPosition);

        setFloatingMode("peek");
        setIsFloatingDimmed(false);
      } catch {
        // Floating behavior is an ergonomic layer; connection/status sync should keep working.
      }
    },
  );

  const expandFloatingWindow = useEffectEvent(async () => {
    if (!isTauri || floatingMode !== "peek") {
      return;
    }

    try {
      clearFloatingIdleTimers();
      const appWindow = getCurrentWindow();
      const [position, size, monitors] = await Promise.all([
        appWindow.outerPosition(),
        appWindow.outerSize(),
        availableMonitors(),
      ]);
      const snapshot = lastExpandedWindowRef.current;
      const fallbackSize = snapshot?.size ?? {
        height: size.height,
        width: size.width,
      };
      const monitor = getNearestMonitor(
        position,
        fallbackSize.width,
        fallbackSize.height,
        monitors,
      );

      if (!monitor) {
        return;
      }

      const expandedMinSize = getPhysicalSizeFromLogical(
        FLOATING_EXPANDED_MIN_LOGICAL_SIZE,
        monitor,
      );
      const defaultExpandedSize = getPhysicalSizeFromLogical(
        FLOATING_DEFAULT_EXPANDED_LOGICAL_SIZE,
        monitor,
      );
      const targetSize = new PhysicalSize(
        Math.max(snapshot?.size.width ?? defaultExpandedSize.width, expandedMinSize.width),
        Math.max(
          snapshot?.size.height ?? defaultExpandedSize.height,
          expandedMinSize.height,
        ),
      );
      const nextPosition = getRightDockPosition(
        monitor,
        targetSize.width,
        targetSize.height,
        snapshot?.position.y ?? position.y,
      );

      await appWindow.setMinSize(expandedMinSize);
      await appWindow.setSize(targetSize);
      await appWindow.setPosition(nextPosition);

      setFloatingMode("expanded");
      setIsFloatingDimmed(false);
    } catch {
      setFloatingMode("expanded");
      setIsFloatingDimmed(false);
    }
  });

  const scheduleFloatingIdle = useEffectEvent(() => {
    if (!isTauri || floatingMode === "peek") {
      return;
    }

    clearFloatingIdleTimers();
    dimTimerRef.current = window.setTimeout(() => {
      setIsFloatingDimmed(true);
    }, FLOATING_DIM_DELAY_MS);
    collapseTimerRef.current = window.setTimeout(() => {
      void collapseFloatingWindow();
    }, FLOATING_COLLAPSE_DELAY_MS);
  });

  const handleFloatingMouseEnter = useEffectEvent(() => {
    clearFloatingIdleTimers();
    setIsFloatingDimmed(false);

    if (floatingMode === "peek") {
      void expandFloatingWindow();
    }
  });

  const handleFloatingMouseLeave = useEffectEvent(() => {
    scheduleFloatingIdle();
  });

  const handleHideFloatingWindow = useEffectEvent(async () => {
    if (!isTauri) {
      return;
    }

    try {
      await getCurrentWindow().hide();
    } catch {
      // The tray and global shortcut remain available if hiding is unsupported.
    }
  });

  const handleOpenAccountLoginUrl = useEffectEvent(async (url?: string) => {
    if (!url) {
      return;
    }

    try {
      await openUrl(url);
    } catch (error) {
      setAuthError(getErrorMessage(error, "无法打开登录页面。"));
    }
  });

  const handleCopyAccountLoginCode = useEffectEvent(async () => {
    const code = accountLoginFlow?.userCode;
    if (!code) {
      return;
    }

    try {
      if (!navigator.clipboard) {
        throw new Error("当前环境不支持剪贴板。");
      }

      await navigator.clipboard.writeText(code);
      setAccountLoginMessage("验证码已复制。");
    } catch (error) {
      setAuthError(getErrorMessage(error, "复制验证码失败，请手动复制。"));
    }
  });

  const handleStartAccountLogin = useEffectEvent(async () => {
    if (
      !isTauri ||
      isStartingAccountLogin ||
      accountLoginFlow ||
      connectionState === "connecting" ||
      isLaunching
    ) {
      return;
    }

    setIsStartingAccountLogin(true);
    setAuthError(null);
    setAccountLoginMessage(null);
    setCodexClientRestartMessage(null);

    try {
      const authStorage = await ensureFileAuthCredentialsStore();
      if (authStorage.changed) {
        setAccountLoginMessage("已启用本机多账号存储，正在启动登录...");
      }

      if (
        authStorage.changed ||
        !clientRef.current ||
        !clientRef.current.isConnected()
      ) {
        await connectToServer({ launchIfNeeded: true });
      }

      const client = clientRef.current;
      if (!client || !client.isConnected()) {
        throw new Error("Codex app-server 未连接。");
      }

      const currentAccount = await client.getAccount();
      if (currentAccount.account) {
        const registry = await saveCurrentCodexAccount();
        setAuthRegistry(registry);
      }

      let response: LoginAccountResponse;
      try {
        response = await client.startAccountLogin({
          type: "chatgpt",
        });
      } catch (browserLoginError) {
        const browserLoginMessage = getErrorMessage(
          browserLoginError,
          "浏览器登录启动失败。",
        );
        appendServerLog(`Browser login unavailable: ${browserLoginMessage}`);

        try {
          response = await client.startAccountLogin({
            type: "chatgptDeviceCode",
          });
        } catch (deviceLoginError) {
          const deviceLoginMessage = getErrorMessage(
            deviceLoginError,
            "设备码登录启动失败。",
          );
          throw new Error(
            `浏览器登录启动失败：${browserLoginMessage}；设备码登录也不可用：${deviceLoginMessage}`,
          );
        }
      }

      if (response.type === "chatgptDeviceCode") {
        setAccountLoginFlow({
          type: "chatgptDeviceCode",
          loginId: response.loginId,
          verificationUrl: response.verificationUrl,
          userCode: response.userCode,
          startedAt: Date.now(),
        });
        setAccountLoginMessage("请在浏览器完成登录，完成后会自动保存到账号列表。");
        setLastEvent("account/login/started");
        void handleOpenAccountLoginUrl(response.verificationUrl);
        return;
      }

      if (response.type === "chatgpt") {
        setAccountLoginFlow({
          type: "chatgpt",
          loginId: response.loginId,
          authUrl: response.authUrl,
          startedAt: Date.now(),
        });
        setAccountLoginMessage(
          "请在浏览器选择要添加的 ChatGPT 账号；完成后会自动保存到账号列表。",
        );
        setLastEvent("account/login/started");
        void handleOpenAccountLoginUrl(response.authUrl);
        return;
      }

      setAccountLoginMessage("登录请求已提交，等待 Codex 返回结果。");
    } catch (error) {
      setAuthError(getErrorMessage(error, "启动账号登录失败。"));
    } finally {
      setIsStartingAccountLogin(false);
    }
  });

  const handleCancelAccountLogin = useEffectEvent(async () => {
    const flow = accountLoginFlow;
    const client = clientRef.current;

    if (!flow || !client || !client.isConnected()) {
      setAccountLoginFlow(null);
      return;
    }

    try {
      await client.cancelAccountLogin(flow.loginId);
      setAccountLoginFlow(null);
      setAccountLoginMessage("已取消账号登录。");
    } catch (error) {
      setAuthError(getErrorMessage(error, "取消账号登录失败。"));
    }
  });

  const handleSwitchAccount = useEffectEvent(async (accountKey: string) => {
    if (!isTauri || switchingAccountKey || isRestartingCodexClient) {
      return;
    }

    const previousAccountKey = authRegistry?.active_account_key ?? null;
    let accountWasReplaced = false;
    setSwitchingAccountKey(accountKey);
    setAuthError(null);
    setCodexClientRestartMessage(null);
    setAccountLoginMessage("正在切换并验证账号...");
    setLastError(null);
    setRateLimitError(null);

    try {
      await ensureFileAuthCredentialsStore();
      await disconnectClient("Reconnecting");
      const result = await switchLocalCodexAccount(accountKey);
      accountWasReplaced = true;
      setAuthRegistry(result.registry);
      setLastEvent("account/switched");
      await connectToServer({ launchIfNeeded: true });

      const client = clientRef.current;
      if (!client || !client.isConnected()) {
        throw new Error("切换后无法连接 Codex app-server，目标账号尚未验证。");
      }

      const accountResult = await client.getAccount(true);
      if (!accountResult.account) {
        throw new Error("目标账号认证已失效，请重新登录并添加该账号。");
      }

      const verifiedRegistry = await saveCurrentCodexAccount();
      if (verifiedRegistry.active_account_key !== accountKey) {
        throw new Error("切换后的账号与目标账号不一致，已停止继续切换。");
      }

      setAuthRegistry(verifiedRegistry);
      setAccount(accountResult.account);
      setLastEvent("account/switch-verified");
      setAccountLoginMessage("账号已切换，认证验证成功。");
      await refreshSnapshot();

      if (restartCodexClientAfterSwitch) {
        const restarted = await handleRestartCodexClient();
        if (restarted) {
          setCodexClientRestartMessage(
            "已重启官方客户端。浮窗账号已切换；新版 ChatGPT 客户端如使用独立会话，可能仍需在客户端内确认账号。",
          );
        }
      } else {
        setCodexClientRestartMessage(
          "浮窗账号已完成切换；官方桌面客户端尚未重启。",
        );
      }
    } catch (error) {
      const switchError = formatLocalAccountError(error, "切换账号失败。");
      let rollbackError: string | null = null;
      let rollbackSucceeded = false;

      if (
        accountWasReplaced &&
        previousAccountKey &&
        previousAccountKey !== accountKey
      ) {
        setAccountLoginMessage("目标账号验证失败，正在恢复原账号...");

        try {
          await disconnectClient("Reconnecting");
          const rollbackResult = await switchLocalCodexAccount(previousAccountKey);
          setAuthRegistry(rollbackResult.registry);
          await connectToServer({ launchIfNeeded: true });

          const rollbackClient = clientRef.current;
          if (!rollbackClient || !rollbackClient.isConnected()) {
            throw new Error("恢复后无法连接 Codex app-server。");
          }

          const rollbackAccount = await rollbackClient.getAccount(true);
          if (!rollbackAccount.account) {
            throw new Error("原账号认证也已失效。");
          }

          const restoredRegistry = await saveCurrentCodexAccount();
          if (restoredRegistry.active_account_key !== previousAccountKey) {
            throw new Error("恢复后的账号与原账号不一致。");
          }

          setAuthRegistry(restoredRegistry);
          setAccount(rollbackAccount.account);
          setLastEvent("account/switch-rolled-back");
          setAccountLoginMessage("目标账号验证失败，已自动恢复原账号。");
          await refreshSnapshot();
          rollbackSucceeded = true;
        } catch (rollbackFailure) {
          rollbackError = formatLocalAccountError(
            rollbackFailure,
            "自动恢复原账号失败。",
          );
        }
      }

      if (!rollbackSucceeded) {
        setConnectionState("error");
        setAccountLoginMessage(null);
      }

      setAuthError(
        rollbackError
          ? `${switchError} 自动恢复原账号也失败：${rollbackError}`
          : rollbackSucceeded
            ? `${switchError} 已自动恢复原账号。`
            : switchError,
      );
    } finally {
      setSwitchingAccountKey(null);
    }
  });

  const handleSaveCurrentAccount = useEffectEvent(async () => {
    if (!isTauri || isSavingAccount || isStartingAccountLogin) {
      return;
    }

    setIsSavingAccount(true);
    setAuthError(null);
    setAccountLoginMessage(null);

    try {
      await ensureFileAuthCredentialsStore();
      const registry = await saveCurrentCodexAccount();
      setAuthRegistry(registry);
      setLastEvent("account/saved");
    } catch (error) {
      setAuthError(formatLocalAccountError(error, "保存当前账号失败。"));
    } finally {
      setIsSavingAccount(false);
    }
  });

  useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    window.localStorage.setItem(
      RESTART_CODEX_CLIENT_AFTER_SWITCH_KEY,
      String(restartCodexClientAfterSwitch),
    );
  }, [restartCodexClientAfterSwitch]);

  useEffect(() => {
    if (isTauri) {
      void loadAuthAccounts();
      void loadThreadBoard();
      void connectToServer({ launchIfNeeded: true });
    }

    return () => {
      void disconnectClient();
    };
  }, [isTauri]);

  useEffect(() => {
    if (!isTauri) {
      return undefined;
    }

    const appWindow = getCurrentWindow();
    let disposed = false;
    let isSnapping = false;
    let snapTimer: number | null = null;
    let unlistenMoved: (() => void) | null = null;

    void appWindow
      .onMoved(() => {
        if (disposed || isSnapping) {
          return;
        }

        if (snapTimer !== null) {
          window.clearTimeout(snapTimer);
        }

        snapTimer = window.setTimeout(() => {
          isSnapping = true;
          void snapWindowToScreenEdge(appWindow).finally(() => {
            scheduleFloatingIdle();
            window.setTimeout(() => {
              isSnapping = false;
            }, SNAP_SETTLE_DELAY_MS);
          });
        }, SNAP_SETTLE_DELAY_MS);
      })
      .then((unlisten) => {
        if (disposed) {
          unlisten();
        } else {
          unlistenMoved = unlisten;
        }
      })
      .catch(() => {
        // Window snapping is a convenience layer; the status board should still work without it.
      });

    return () => {
      disposed = true;
      if (snapTimer !== null) {
        window.clearTimeout(snapTimer);
      }
      clearFloatingIdleTimers();
      unlistenMoved?.();
    };
  }, [isTauri]);

  useEffect(() => {
    if (!isTauri) {
      return undefined;
    }

    scheduleFloatingIdle();

    return () => {
      clearFloatingIdleTimers();
    };
  }, [isTauri]);

  useEffect(() => {
    if (connectionState !== "connected") {
      return undefined;
    }

    const intervalId = window.setInterval(() => {
      void refreshSnapshot();
    }, 20_000);

    return () => window.clearInterval(intervalId);
  }, [connectionState]);

  const normalizedQuery = deferredQuery.trim().toLowerCase();
  const visibleThreads = threads
    .filter((thread) => {
      if (filter !== "all" && getThreadFilterKey(thread) !== filter) {
        return false;
      }

      if (!normalizedQuery) {
        return true;
      }

      const metadata = getThreadBoardMetadata(threadBoard, thread.id);
      const haystack = [
        getThreadLabel(thread),
        thread.preview,
        thread.cwd,
        thread.modelProvider,
        thread.source,
        metadata.note,
        metadata.project,
        metadata.priority ? PRIORITY_LABELS[metadata.priority] : null,
        metadata.stage ? STAGE_LABELS[metadata.stage] : null,
      ]
        .filter(Boolean)
        .join(" ")
        .toLowerCase();

      return haystack.includes(normalizedQuery);
    })
    .sort((left, right) => {
      const leftPinned = getThreadBoardMetadata(threadBoard, left.id).pinned;
      const rightPinned = getThreadBoardMetadata(threadBoard, right.id).pinned;

      if (leftPinned !== rightPinned) {
        return leftPinned ? -1 : 1;
      }

      return right.updatedAt - left.updatedAt;
    });

  const connectionBadge =
    connectionState === "connected"
      ? "已连接"
      : connectionState === "connecting"
        ? "连接中"
        : connectionState === "error"
          ? "需要处理"
          : "未连接";
  const isConnected = connectionState === "connected";
  const isBusy = connectionState === "connecting" || isLaunching;
  const displayedError = formatConnectionError(lastError);
  const displayedRateLimitError = formatRateLimitError(rateLimitError);
  const authAccounts = authRegistry?.accounts ?? [];
  const activeAuthKey = authRegistry?.active_account_key ?? null;
  const globalPrimaryUsage = getGlobalPrimaryUsage(rateLimits);
  const threadGroups = (() => {
    if (groupBy === "none") {
      return [{ key: "all", label: "全部任务", threads: visibleThreads }];
    }

    const groups = new Map<
      string,
      { label: string; sortValue: number | string; threads: Thread[] }
    >();

    for (const thread of visibleThreads) {
      const metadata = getThreadBoardMetadata(threadBoard, thread.id);
      const label = getThreadGroupLabel(metadata, groupBy);
      const key = `${groupBy}:${label}`;

      if (!groups.has(key)) {
        groups.set(key, {
          label,
          sortValue: getThreadGroupSortValue(label, metadata, groupBy),
          threads: [],
        });
      }

      groups.get(key)?.threads.push(thread);
    }

    return Array.from(groups.entries())
      .map(([key, value]) => ({ key, ...value }))
      .sort((left, right) => {
        if (typeof left.sortValue === "number" && typeof right.sortValue === "number") {
          return left.sortValue - right.sortValue;
        }

        return String(left.sortValue).localeCompare(String(right.sortValue), "zh-CN");
      });
  })();
  const isPeekMode = floatingMode === "peek";
  const shellClassName = [
    "shell",
    IS_WINDOWS ? "platform-windows" : null,
    `quota-${globalPrimaryUsage.tone}`,
    `floating-${floatingMode}`,
    isFloatingDimmed ? "floating-dimmed" : null,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <main
      className={shellClassName}
      onMouseEnter={() => handleFloatingMouseEnter()}
      onMouseLeave={() => handleFloatingMouseLeave()}
    >
      {isPeekMode ? (
        <button
          aria-label="展开 Codex 状态浮窗"
          className={`peek-capsule peek-${globalPrimaryUsage.tone}`}
          type="button"
          onClick={() => void expandFloatingWindow()}
        >
          <span className="peek-grip" data-tauri-drag-region />
          <span className="peek-label">{globalPrimaryUsage.compactLabel}</span>
          <strong>{formatPercent(globalPrimaryUsage.usedPercent)}</strong>
          <small>{getCompactUsageToneText(globalPrimaryUsage.usedPercent)}</small>
        </button>
      ) : (
        <>
      <header className="topbar" data-tauri-drag-region>
        <div>
          <p className="eyebrow">Codex 状态</p>
          <h1>{describeAccount(account)}</h1>
          <p className="subtitle">
            {lastSyncAt ? `${formatRelativeTime(lastSyncAt / 1000)}更新` : "等待数据"}
          </p>
        </div>
        <div className="topbar-actions">
          <div className={`status-pill tone-${connectionState}`}>{connectionBadge}</div>
          {IS_WINDOWS && isTauri ? (
            <button
              aria-label="隐藏到系统托盘"
              className="window-hide-button"
              title="隐藏到系统托盘"
              type="button"
              onClick={() => void handleHideFloatingWindow()}
            >
              −
            </button>
          ) : null}
        </div>
      </header>

      <aside className={`quota-ribbon quota-ribbon-${globalPrimaryUsage.tone}`}>
        <div>
          <span>{globalPrimaryUsage.label}</span>
          <strong>{formatPercent(globalPrimaryUsage.usedPercent)}</strong>
        </div>
        <p>
          {getUsageToneText(globalPrimaryUsage.usedPercent, globalPrimaryUsage.label)}
          {globalPrimaryUsage.poolLabel ? ` · ${globalPrimaryUsage.poolLabel}` : " · 等待额度数据"}
          {" · "}
          {globalPrimaryUsage.resetsAt
            ? `重置 ${formatResetTime(globalPrimaryUsage.resetsAt)}`
            : "重置时间暂无"}
        </p>
      </aside>

      <section className="panel connection-panel">
        <div className="action-row">
          <button
            className="primary"
            type="button"
            onClick={() => void connectToServer({ launchIfNeeded: isTauri })}
            disabled={isBusy || !isTauri}
          >
            {isBusy ? "连接中..." : isConnected ? "重新连接" : "重试连接"}
          </button>
          <button
            type="button"
            onClick={() => void refreshSnapshot()}
            disabled={!isConnected}
          >
            刷新
          </button>
        </div>

        {!isTauri ? (
          <p className="hint">
            请在 Tauri 客户端内运行，才能使用本地 stdio 桥接；浏览器预览仅用于查看界面。
          </p>
        ) : null}

        {displayedError ? <p className="error-text">{displayedError}</p> : null}
        {lastError && serverLogs.length > 0 ? (
          <div className="log-list compact-log">
            {serverLogs.map((line, index) => (
              <code className="log-line" key={`${line}-${index}`}>
                {line}
              </code>
            ))}
          </div>
        ) : null}
      </section>

      {isTauri ? (
        <section className="panel account-panel">
          <div className="section-header">
            <div>
              <p className="section-eyebrow">账号</p>
              <h2>账号切换</h2>
            </div>
            <span className="section-note">{authAccounts.length} 个账号</span>
          </div>

          <div className="action-row account-actions">
            <button
              type="button"
              onClick={() => void handleSaveCurrentAccount()}
              disabled={
                isSavingAccount ||
                isStartingAccountLogin ||
                Boolean(accountLoginFlow) ||
                Boolean(switchingAccountKey) ||
                isRestartingCodexClient
              }
            >
              {isSavingAccount ? "保存中..." : "保存当前账号"}
            </button>
            <button
              className="primary"
              type="button"
              onClick={() => void handleStartAccountLogin()}
              disabled={
                !isTauri ||
                isStartingAccountLogin ||
                Boolean(accountLoginFlow) ||
                Boolean(switchingAccountKey) ||
                isRestartingCodexClient ||
                connectionState === "connecting" ||
                isLaunching
              }
            >
              {isStartingAccountLogin ? "启动登录..." : "登录添加账号"}
            </button>
            <button
              type="button"
              onClick={() => void loadAuthAccounts()}
              disabled={
                isSavingAccount ||
                isStartingAccountLogin ||
                Boolean(switchingAccountKey) ||
                isRestartingCodexClient
              }
            >
              刷新账号
            </button>
          </div>

          {accountLoginFlow ? (
            <div className="account-login-box">
              <div>
                <strong>等待浏览器登录</strong>
                <small>登录完成后会自动保存为本地账号快照，并设为当前账号。</small>
              </div>
              {accountLoginFlow.userCode ? (
                <div className="login-code-row">
                  <span>验证码</span>
                  <code>{accountLoginFlow.userCode}</code>
                </div>
              ) : null}
              <div className="action-row account-login-actions">
                <button
                  type="button"
                  onClick={() =>
                    void handleOpenAccountLoginUrl(
                      accountLoginFlow.verificationUrl ?? accountLoginFlow.authUrl,
                    )
                  }
                  disabled={!accountLoginFlow.verificationUrl && !accountLoginFlow.authUrl}
                >
                  打开登录页面
                </button>
                {accountLoginFlow.userCode ? (
                  <button
                    type="button"
                    onClick={() => void handleCopyAccountLoginCode()}
                  >
                    复制验证码
                  </button>
                ) : null}
                <button type="button" onClick={() => void handleCancelAccountLogin()}>
                  取消登录
                </button>
              </div>
            </div>
          ) : null}

          <div className="account-restart-box">
            <label className="switch-row">
              <input
                checked={restartCodexClientAfterSwitch}
                disabled={Boolean(switchingAccountKey) || isRestartingCodexClient}
                type="checkbox"
                onChange={(event) =>
                  setRestartCodexClientAfterSwitch(event.currentTarget.checked)
                }
              />
              <span>
                <strong>账号变更后自动重启 Codex 客户端</strong>
                <small>
                  添加或切换账号成功后关闭并重新打开官方客户端；新版 ChatGPT 客户端的独立登录会话可能仍需手动确认。
                </small>
              </span>
            </label>
            <button
              type="button"
              onClick={() => void handleRestartCodexClient()}
              disabled={Boolean(switchingAccountKey) || isRestartingCodexClient}
            >
              {isRestartingCodexClient ? "重启中..." : "立即重启 Codex 客户端"}
            </button>
          </div>

          {authError ? <p className="error-text">{authError}</p> : null}
          {accountLoginMessage ? <p className="hint">{accountLoginMessage}</p> : null}
          {codexClientRestartMessage ? (
            <p className="hint">{codexClientRestartMessage}</p>
          ) : null}

          {authAccounts.length === 0 ? (
            <p className="empty-state">
              暂无已保存账号。点击“登录添加账号”完成登录，或先用官方 Codex 登录一次再保存当前账号。
            </p>
          ) : (
            <div className="account-list">
              {authAccounts.map((authAccount) => {
                const isActiveAccount = authAccount.account_key === activeAuthKey;
                const isSwitching = switchingAccountKey === authAccount.account_key;

                return (
                  <article className="account-card" key={authAccount.account_key}>
                    <div>
                      <h3>{getAuthAccountTitle(authAccount)}</h3>
                      <p>
                        {formatAuthPlan(authAccount.plan)} ·{" "}
                        {authAccount.auth_mode ?? "chatgpt"} · 最近使用{" "}
                        {formatUnixTime(authAccount.last_used_at)}
                      </p>
                    </div>
                    {isActiveAccount ? (
                      <span className="status-pill tone-connected">当前</span>
                    ) : (
                      <button
                        type="button"
                        className="chip"
                        disabled={
                          Boolean(switchingAccountKey) ||
                          isSavingAccount ||
                          isRestartingCodexClient
                        }
                        onClick={() => void handleSwitchAccount(authAccount.account_key)}
                      >
                        {isSwitching ? "切换中..." : "切换"}
                      </button>
                    )}
                  </article>
                );
              })}
            </div>
          )}

          <p className="hint">
            安装包已内置官方 Codex 登录服务，无需安装 CLI。新增和切换只读写本机账号快照；切换失败时会自动恢复原账号。
          </p>
        </section>
      ) : null}

      {isConnected ? (
        <section className="panel">
        <div className="section-header">
          <div>
            <p className="section-eyebrow">用量</p>
            <h2>额度状态</h2>
          </div>
          <span className="section-note">{rateLimits.length || 0} 个额度池</span>
        </div>

        {displayedRateLimitError ? (
          <p className="warning-text">{displayedRateLimitError}</p>
        ) : null}

        {rateLimits.length === 0 ? (
          <p className="empty-state">
            {displayedRateLimitError
              ? "任务看板会继续同步；可以稍后点击刷新重试用量读取。"
              : "暂无额度数据。连接已登录的 Codex 会话后，这里会自动显示剩余用量。"}
          </p>
        ) : (
          <div className="usage-list">
            {rateLimits.map((snapshot, index) => {
              const primaryTone = getUsageTone(snapshot.primary?.usedPercent);

              return (
                <article
                  className={`usage-card usage-${primaryTone}`}
                  key={`${snapshot.limitId ?? snapshot.limitName ?? "default"}-${index}`}
                >
                  <div className="usage-header">
                    <div>
                      <h3>{snapshot.limitName ?? snapshot.limitId ?? "默认额度池"}</h3>
                      <p>{formatPlanType(snapshot.planType)}</p>
                    </div>
                    <div className="usage-badge-stack">
                      <span className={`usage-badge usage-badge-${primaryTone}`}>
                        {getUsageToneText(
                          snapshot.primary?.usedPercent,
                          formatRateLimitWindowSummaryLabel(snapshot.primary),
                        )}
                      </span>
                      <span className="usage-badge">
                        {snapshot.rateLimitReachedType ? "受限" : "正常"}
                      </span>
                    </div>
                  </div>

                <div className="meter-group">
                  <div className="meter-copy">
                    <span>
                      {formatRateLimitWindowLabel(snapshot.primary, "短周期使用量")}
                    </span>
                    <strong>{formatPercent(snapshot.primary?.usedPercent)}</strong>
                  </div>
                  <div className="meter-track">
                    <div
                      className={`meter-fill primary usage-fill-${primaryTone}`}
                      style={{
                        width: `${Math.min(snapshot.primary?.usedPercent ?? 0, 100)}%`,
                      }}
                    />
                  </div>
                  <span className="meter-note">
                    重置时间 {formatResetTime(snapshot.primary?.resetsAt)}
                  </span>
                </div>

                <div className="meter-group">
                  <div className="meter-copy">
                    <span>
                      {formatRateLimitWindowLabel(snapshot.secondary, "长周期使用量")}
                    </span>
                    <strong>{formatPercent(snapshot.secondary?.usedPercent)}</strong>
                  </div>
                  <div className="meter-track">
                    <div
                      className="meter-fill secondary"
                      style={{
                        width: `${Math.min(snapshot.secondary?.usedPercent ?? 0, 100)}%`,
                      }}
                    />
                  </div>
                  <span className="meter-note">
                    重置时间 {formatResetTime(snapshot.secondary?.resetsAt)}
                  </span>
                </div>

                <div className="usage-footer">
                  <span>
                    点数{" "}
                    {snapshot.credits?.unlimited
                      ? "不限"
                      : snapshot.credits?.balance ?? "暂无"}
                  </span>
                  <span>
                    剩余额度{" "}
                    {snapshot.individualLimit
                      ? `${Math.round(snapshot.individualLimit.remainingPercent)}%`
                      : "暂无"}
                  </span>
                </div>
                </article>
              );
            })}
          </div>
        )}
        </section>
      ) : null}

      {isConnected ? (
        <section className="panel">
        <div className="section-header">
          <div>
            <p className="section-eyebrow">任务</p>
            <h2>状态看板</h2>
          </div>
          <span className="section-note">显示 {visibleThreads.length} 个</span>
        </div>

        <div className="toolbar">
          <input
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
            placeholder="搜索标题、备注、项目、阶段或路径"
          />
          <select
            aria-label="任务分组方式"
            value={groupBy}
            onChange={(event) => setGroupBy(event.currentTarget.value as ThreadGroupBy)}
          >
            {(["none", "project", "priority", "stage"] as const).map((value) => (
              <option key={value} value={value}>
                {THREAD_GROUP_LABELS[value]}
              </option>
            ))}
          </select>
          <div className="filter-row">
            {(["all", "recent", "idle", "systemError"] as const).map((value) => (
              <button
                key={value}
                type="button"
                className={filter === value ? "chip active" : "chip"}
                onClick={() => setFilter(value)}
              >
                {THREAD_FILTER_LABELS[value]}
              </button>
            ))}
          </div>
        </div>

        {threadBoardError ? <p className="error-text">{threadBoardError}</p> : null}

        {visibleThreads.length === 0 ? (
          <p className="empty-state">
            暂无匹配任务。可以放宽筛选条件，或等待下一次同步。
          </p>
        ) : (
          <div className="thread-list">
            {threadGroups.map((group) => (
              <div className="thread-group" key={group.key}>
                {groupBy !== "none" ? (
                  <div className="thread-group-header">
                    <span>{group.label}</span>
                    <span>{group.threads.length} 个</span>
                  </div>
                ) : null}

                {group.threads.map((thread) => {
                  const metadata = getThreadBoardMetadata(threadBoard, thread.id);
                  const boardChips = getBoardChips(metadata);
                  const isEditingBoard = editingThreadId === thread.id;
                  const threadTone = getThreadStatusTone(thread);

                  return (
                    <article
                      className={`thread-card thread-${threadTone} ${
                        metadata.pinned ? "thread-pinned" : ""
                      }`}
                      key={thread.id}
                    >
                      <div className="thread-topline">
                        <div>
                          <h3>{getThreadLabel(thread)}</h3>
                          <p className="thread-meta">
                            {shortenPath(thread.cwd)} / {thread.modelProvider} /{" "}
                            {String(thread.source)}
                          </p>
                        </div>
                        <div className="thread-actions">
                          <button
                            className={
                              metadata.pinned
                                ? "icon-button pinned active"
                                : "icon-button pinned"
                            }
                            title={metadata.pinned ? "取消关注" : "标记关注"}
                            type="button"
                            onClick={() =>
                              updateThreadBoardMetadata(thread.id, {
                                pinned: !metadata.pinned,
                              })
                            }
                          >
                            {metadata.pinned ? "★" : "☆"}
                          </button>
                          <div className={`status-pill tone-${threadTone}`}>
                            {getThreadStatusText(thread)}
                          </div>
                        </div>
                      </div>

                      {hasBoardDetails(metadata) ? (
                        <div className="board-chip-row">
                          {boardChips.map((chip) => (
                            <span className={chip.className} key={chip.label}>
                              {chip.label}
                            </span>
                          ))}
                          {metadata.note ? (
                            <span className="board-note">备注：{metadata.note}</span>
                          ) : null}
                        </div>
                      ) : null}

                      {isEditingBoard ? (
                        <div className="thread-board-editor">
                          <label>
                            备注
                            <input
                              value={metadata.note ?? ""}
                              onBlur={() =>
                                void persistThreadBoardMetadata(thread.id, metadata)
                              }
                              onChange={(event) =>
                                updateThreadBoardMetadata(
                                  thread.id,
                                  { note: event.currentTarget.value },
                                  { persist: false },
                                )
                              }
                              placeholder="等接口 / 等设计 / 今日处理"
                            />
                          </label>
                          <label>
                            项目
                            <input
                              value={metadata.project ?? ""}
                              onBlur={() =>
                                void persistThreadBoardMetadata(thread.id, metadata)
                              }
                              onChange={(event) =>
                                updateThreadBoardMetadata(
                                  thread.id,
                                  { project: event.currentTarget.value },
                                  { persist: false },
                                )
                              }
                              placeholder="项目名"
                            />
                          </label>
                          <label>
                            优先级
                            <select
                              value={metadata.priority ?? "none"}
                              onChange={(event) =>
                                updateThreadBoardMetadata(thread.id, {
                                  priority: event.currentTarget.value as ThreadPriority,
                                })
                              }
                            >
                              {(["none", "high", "medium", "low"] as const).map(
                                (value) => (
                                  <option key={value} value={value}>
                                    {PRIORITY_LABELS[value]}
                                  </option>
                                ),
                              )}
                            </select>
                          </label>
                          <label>
                            阶段
                            <select
                              value={metadata.stage ?? "none"}
                              onChange={(event) =>
                                updateThreadBoardMetadata(thread.id, {
                                  stage: event.currentTarget.value as ThreadStage,
                                })
                              }
                            >
                              {(["none", "todo", "doing", "waiting", "done"] as const).map(
                                (value) => (
                                  <option key={value} value={value}>
                                    {STAGE_LABELS[value]}
                                  </option>
                                ),
                              )}
                            </select>
                          </label>
                        </div>
                      ) : null}

                      <div className="thread-footer">
                        <span>{formatRelativeTime(thread.updatedAt)}</span>
                        <span>{thread.cliVersion}</span>
                        <button
                          className="link-button"
                          type="button"
                          onClick={() =>
                            setEditingThreadId(isEditingBoard ? null : thread.id)
                          }
                        >
                          {isEditingBoard ? "收起看板" : "编辑看板"}
                        </button>
                      </div>
                    </article>
                  );
                })}
              </div>
            ))}
          </div>
        )}
        <p className="event-line">最近事件：{formatEventName(lastEvent)}</p>
        </section>
      ) : null}
        </>
      )}
    </main>
  );
}

export default App;
