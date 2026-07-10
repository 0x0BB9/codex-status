import type {
  ClientRequest,
  InitializeResponse,
  ServerNotification,
} from "../generated";
import type {
  CancelLoginAccountResponse,
  GetAccountResponse,
  GetAccountRateLimitsResponse,
  LoginAccountParams,
  LoginAccountResponse,
  ThreadListParams,
  ThreadListResponse,
} from "../generated/v2";

export const DEFAULT_CODEX_WS_URL = "ws://127.0.0.1:48123";

type PendingRequest = {
  reject: (reason: Error) => void;
  resolve: (value: any) => void;
};

type CodexAppServerClientOptions = {
  onDisconnect?: (reason: string) => void;
  onNotification?: (notification: ServerNotification) => void;
};

type RpcError = {
  code?: number;
  message?: string;
};

function toError(error: unknown, fallback: string) {
  if (error instanceof Error) {
    return error;
  }

  if (typeof error === "string") {
    return new Error(error);
  }

  return new Error(fallback);
}

export class CodexAppServerClient {
  private nextRequestId = 1;
  private options: CodexAppServerClientOptions;
  private pending = new Map<number, PendingRequest>();
  private socket: WebSocket | null = null;

  constructor(options: CodexAppServerClientOptions = {}) {
    this.options = options;
  }

  connect(url: string) {
    this.disconnect("Reconnecting");

    return new Promise<InitializeResponse>((resolve, reject) => {
      const socket = new WebSocket(url);
      let settled = false;
      const timeoutId = window.setTimeout(() => {
        finalizeError(new Error("Timed out while connecting to Codex app-server."));
      }, 5_000);

      const finalizeError = (error: unknown) => {
        window.clearTimeout(timeoutId);
        const nextError = toError(error, "Unable to connect to Codex app-server.");
        if (!settled) {
          settled = true;
          reject(nextError);
        }
        socket.close();
      };

      socket.addEventListener(
        "open",
        async () => {
          this.socket = socket;
          this.attachSocketHandlers(socket);

          try {
            const response = await this.request<InitializeResponse>("initialize", {
              clientInfo: {
                name: "codex-status-floater",
                version: "0.1.0",
              },
              capabilities: null,
            });

            this.send({ method: "initialized" });
            window.clearTimeout(timeoutId);
            settled = true;
            resolve(response);
          } catch (error) {
            finalizeError(error);
          }
        },
        { once: true },
      );

      socket.addEventListener(
        "error",
        () => {
          finalizeError(new Error("WebSocket refused the connection."));
        },
        { once: true },
      );
    });
  }

  disconnect(reason = "Disconnected") {
    const socket = this.socket;
    this.socket = null;

    if (!socket) {
      return;
    }

    this.rejectPending(new Error(reason));
    socket.onclose = null;
    socket.onerror = null;
    socket.onmessage = null;

    if (socket.readyState === WebSocket.CONNECTING || socket.readyState === WebSocket.OPEN) {
      socket.close();
    }
  }

  isConnected() {
    return this.socket?.readyState === WebSocket.OPEN;
  }

  getAccount() {
    return this.request<GetAccountResponse>("account/read", { refreshToken: false });
  }

  getRateLimits() {
    return this.request<GetAccountRateLimitsResponse>("account/rateLimits/read");
  }

  startAccountLogin(params: LoginAccountParams) {
    return this.request<LoginAccountResponse>("account/login/start", params);
  }

  cancelAccountLogin(loginId: string) {
    return this.request<CancelLoginAccountResponse>("account/login/cancel", { loginId });
  }

  listThreads(params: ThreadListParams) {
    return this.request<ThreadListResponse>("thread/list", params);
  }

  request<T>(method: ClientRequest["method"], params?: unknown) {
    const socket = this.socket;
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      return Promise.reject(new Error("Not connected to Codex app-server."));
    }

    const id = this.nextRequestId;
    this.nextRequestId += 1;

    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({
        id,
        method,
        params,
      });
    });
  }

  private attachSocketHandlers(socket: WebSocket) {
    socket.onmessage = (event) => {
      const message = JSON.parse(String(event.data)) as
        | {
            error?: RpcError;
            id?: number;
            result?: unknown;
          }
        | ServerNotification;

      if ("id" in message && typeof message.id === "number") {
        const pendingRequest = this.pending.get(message.id);
        if (!pendingRequest) {
          return;
        }

        this.pending.delete(message.id);
        if ("error" in message && message.error) {
          pendingRequest.reject(
            new Error(message.error.message ?? "Codex app-server returned an error."),
          );
          return;
        }

        pendingRequest.resolve("result" in message ? message.result : undefined);
        return;
      }

      if ("method" in message && this.options.onNotification) {
        this.options.onNotification(message);
      }
    };

    socket.onclose = (event) => {
      this.socket = null;
      const reason = event.reason || "Codex app-server disconnected.";
      this.rejectPending(new Error(reason));
      this.options.onDisconnect?.(reason);
    };

    socket.onerror = () => {
      this.options.onDisconnect?.("Codex app-server encountered a websocket error.");
    };
  }

  private rejectPending(error: Error) {
    for (const pendingRequest of this.pending.values()) {
      pendingRequest.reject(error);
    }
    this.pending.clear();
  }

  private send(message: unknown) {
    this.socket?.send(JSON.stringify(message));
  }
}
