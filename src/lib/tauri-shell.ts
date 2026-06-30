import type {
  ClientRequest,
  InitializeResponse,
  ServerNotification,
} from "../generated";
import type {
  GetAccountResponse,
  GetAccountRateLimitsResponse,
  ThreadListParams,
  ThreadListResponse,
} from "../generated/v2";

type PendingRequest = {
  reject: (reason: Error) => void;
  resolve: (value: any) => void;
};

type RpcError = {
  code?: number;
  message?: string;
};

type CodexAppServerShellClientOptions = {
  onDisconnect?: (reason: string) => void;
  onNotification?: (notification: ServerNotification) => void;
  onServerLog?: (line: string) => void;
};

const RPC_TIMEOUT_MS = 8_000;
const COMMAND_NAMES = ["codex-app-server", "codex-app-server-nvm"] as const;

export function isTauriRuntime() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export class CodexAppServerShellClient {
  private buffer = "";
  private child: import("@tauri-apps/plugin-shell").Child | null = null;
  private closing = false;
  private nextRequestId = 1;
  private options: CodexAppServerShellClientOptions;
  private pending = new Map<number, PendingRequest>();

  constructor(options: CodexAppServerShellClientOptions = {}) {
    this.options = options;
  }

  async connect(_target?: string) {
    if (!isTauriRuntime()) {
      throw new Error("The stdio transport is only available inside Tauri.");
    }

    this.disconnect("Reconnecting");
    this.closing = false;

    this.child = await this.spawnAppServer();

    try {
      const response = await this.request<InitializeResponse>("initialize", {
        clientInfo: {
          name: "codex-status-floater",
          version: "0.1.0",
        },
        capabilities: null,
      });

      await this.send({ method: "initialized" });
      return response;
    } catch (error) {
      this.disconnect("Failed to initialize Codex app-server.");
      throw error;
    }
  }

  private async spawnAppServer() {
    const { Command } = await import("@tauri-apps/plugin-shell");
    const errors: string[] = [];

    for (const commandName of COMMAND_NAMES) {
      try {
        const command = Command.create(commandName, ["app-server"]);

        command.stdout.on("data", (chunk) => this.handleStdout(chunk));
        command.stderr.on("data", (line) => this.options.onServerLog?.(line));
        command.on("error", (error) => {
          this.options.onServerLog?.(`${commandName}: ${String(error)}`);
        });
        command.on("close", (payload) => {
          const wasManual = this.closing;
          this.closing = false;
          this.child = null;
          this.buffer = "";
          this.rejectPending(new Error("Codex app-server exited."));

          if (wasManual) {
            return;
          }

          this.options.onServerLog?.(
            `${commandName} closed: code=${payload.code ?? "null"} signal=${
              payload.signal ?? "null"
            }`,
          );
          this.options.onDisconnect?.("Codex app-server exited.");
        });

        const child = await command.spawn();
        this.options.onServerLog?.(`Started ${commandName}.`);
        return child;
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        errors.push(`${commandName}: ${message}`);
      }
    }

    throw new Error(`Unable to start codex app-server. ${errors.join(" | ")}`);
  }

  disconnect(reason = "Disconnected") {
    this.closing = true;
    const child = this.child;
    this.child = null;
    this.buffer = "";
    this.rejectPending(new Error(reason));

    if (child) {
      void child.kill();
    }
  }

  isConnected() {
    return this.child !== null;
  }

  getAccount() {
    return this.request<GetAccountResponse>("account/read", { refreshToken: false });
  }

  getRateLimits() {
    return this.request<GetAccountRateLimitsResponse>("account/rateLimits/read");
  }

  listThreads(params: ThreadListParams) {
    return this.request<ThreadListResponse>("thread/list", params);
  }

  request<T>(method: ClientRequest["method"], params?: unknown) {
    if (!this.child) {
      return Promise.reject(new Error("Codex app-server is not running."));
    }

    const id = this.nextRequestId;
    this.nextRequestId += 1;

    return new Promise<T>((resolve, reject) => {
      const timeoutId = window.setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} timed out waiting for Codex app-server.`));
      }, RPC_TIMEOUT_MS);
      this.pending.set(id, {
        resolve: (value) => {
          window.clearTimeout(timeoutId);
          resolve(value);
        },
        reject: (error) => {
          window.clearTimeout(timeoutId);
          reject(error);
        },
      });
      this.send({
        id,
        method,
        params,
      }).catch((error) => {
        this.pending.delete(id);
        window.clearTimeout(timeoutId);
        reject(error instanceof Error ? error : new Error(String(error)));
      });
    });
  }

  private handleStdout(chunk: string) {
    this.buffer += chunk;

    while (this.buffer.includes("\n")) {
      const newlineIndex = this.buffer.indexOf("\n");
      const rawLine = this.buffer.slice(0, newlineIndex).trim();
      this.buffer = this.buffer.slice(newlineIndex + 1);

      if (!rawLine) {
        continue;
      }

      try {
        const message = JSON.parse(rawLine) as
          | {
              error?: RpcError;
              id?: number;
              result?: unknown;
            }
          | ServerNotification;

        if ("id" in message && typeof message.id === "number") {
          const pendingRequest = this.pending.get(message.id);
          if (!pendingRequest) {
            continue;
          }

          this.pending.delete(message.id);
          if ("error" in message && message.error) {
            pendingRequest.reject(
              new Error(message.error.message ?? "Codex app-server returned an error."),
            );
            continue;
          }

          pendingRequest.resolve("result" in message ? message.result : undefined);
          continue;
        }

        if ("method" in message) {
          this.options.onNotification?.(message);
        }
      } catch {
        this.options.onServerLog?.(rawLine);
      }
    }
  }

  private rejectPending(error: Error) {
    for (const pendingRequest of this.pending.values()) {
      pendingRequest.reject(error);
    }
    this.pending.clear();
  }

  private async send(message: unknown) {
    if (!this.child) {
      throw new Error("Codex app-server is not running.");
    }

    await this.child.write(`${JSON.stringify(message)}\n`);
  }
}
