// Uses synthetic sessions in a temporary CODEX_HOME; never opens the user's auth or history.
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdtemp, mkdir, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import readline from "node:readline";
import http from "node:http";

const diagnosticArgs = process.argv.slice(2);
const embeddedIndex = diagnosticArgs.indexOf("--embedded-bridge");
const embeddedBridge = embeddedIndex >= 0;
if (embeddedBridge) diagnosticArgs.splice(embeddedIndex, 1);
const followIndex = diagnosticArgs.indexOf("--follow-current");
const followCurrent = followIndex >= 0;
if (followCurrent) diagnosticArgs.splice(followIndex, 1);
const bridgeIndex = diagnosticArgs.indexOf("--bridge");
const bridge = bridgeIndex >= 0 ? path.resolve(diagnosticArgs.splice(bridgeIndex, 2)[1]) : null;
const binaries = diagnosticArgs.map((binary) => path.resolve(binary));
if (!binaries.length) throw new Error("Usage: node scripts/check-provider-history.mjs <codex-binary> [...]");

for (const { binary, nativeOnly } of binaries.flatMap((binary) => followCurrent ? [{ binary, nativeOnly: true }, { binary, nativeOnly: false }] : [{ binary, nativeOnly: false }])) {
  const directory = await mkdtemp(path.join(tmpdir(), "floater-provider-check-"));
  let child;
  let closed;
  let server;
    const bridgeStatusPath = path.join(directory, "bridge-status.json");
  const pending = new Map();
  const notices = [];
  try {
    const activeDir = path.join(directory, "sessions", "2026", "10", "21");
    const archivedDir = path.join(directory, "archived_sessions");
    let customActiveId;
    await mkdir(activeDir, { recursive: true });
    await mkdir(archivedDir, { recursive: true });
    let requestObserved;
    let observeRequest;
    if (followCurrent) {
      if (!bridge) throw new Error("--follow-current requires --bridge");
      requestObserved = new Promise((resolve) => { observeRequest = resolve; });
      server = http.createServer((request, response) => {
        const chunks = [];
        request.on("data", (chunk) => chunks.push(chunk));
        request.on("end", () => {
          if (request.method === "POST" && request.url.includes("responses")) {
            observeRequest({ body: JSON.parse(Buffer.concat(chunks).toString()), auth: request.headers.authorization });
            response.writeHead(400, { "Content-Type": "application/json" });
            response.end(JSON.stringify({ error: { message: "Synthetic test stops before inference", type: "invalid_request_error" } }));
          } else { response.writeHead(200, { "Content-Type": "application/json" }); response.end('{"data":[]}'); }
        });
      });
      await new Promise((resolve, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", resolve); });
      await writeFile(path.join(directory, "auth.json"), JSON.stringify({ OPENAI_API_KEY: "fixture-openai-key" }));
      await writeFile(path.join(directory, "active-route.json"), JSON.stringify({ model: "gpt-5", modelProvider: nativeOnly ? "openai" : "fixture_current" }));
    }
    await writeFile(path.join(directory, "config.toml"), [
      'cli_auth_credentials_store = "file"',
      'model_provider = "openai"',
      ...(followCurrent ? [
        '[model_providers.fixture_current]',
        'name = "Local fixture only"',
        `base_url = "http://127.0.0.1:${server.address().port}/v1"`,
        'wire_api = "responses"',
        'requires_openai_auth = false',
        'experimental_bearer_token = "fixture-openai-key"',
      ] : [
        '[model_providers.fixture_custom]',
        'name = "Fixture only"',
        'base_url = "http://127.0.0.1:9/v1"',
        'wire_api = "responses"',
        'requires_openai_auth = false',
      ]),
    ].join("\n"));
    for (const archived of [false, true]) {
      for (const provider of ["openai", "fixture_custom"]) {
        const id = randomUUID();
        if (!archived && provider === "fixture_custom") customActiveId = id;
        const timestamp = "2026-10-21T00:00:00.000Z";
        const rows = [
          { timestamp, type: "session_meta", payload: { id, timestamp, cwd: directory, originator: "codex_cli_rs", cli_version: "0.146.0", source: "cli", model_provider: provider } },
          { timestamp, type: "response_item", payload: { type: "message", role: "user", content: [{ type: "input_text", text: `Synthetic ${provider} conversation` }] } },
          ...(provider === "fixture_custom" ? [
            { timestamp, type: "response_item", payload: { type: "reasoning", content: [{ type: "reasoning_text", text: "Synthetic private reasoning that must not cross providers" }], encrypted_content: null } },
            { timestamp, type: "response_item", payload: { type: "function_call", id: "msg_synthetic_third_party", call_id: "call_synthetic", name: "exec_command", arguments: "{}" } },
            { timestamp, type: "response_item", payload: { type: "function_call_output", call_id: "call_synthetic", output: "Synthetic private tool output" } },
            { timestamp, type: "response_item", payload: { type: "message", role: "assistant", content: [{ type: "output_text", text: "Synthetic visible assistant answer" }] } },
          ] : []),
          { timestamp, type: "event_msg", payload: { type: "user_message", message: `Synthetic ${provider} conversation`, images: [], local_images: [], text_elements: [] } },
        ];
        await writeFile(path.join(archived ? archivedDir : activeDir, `rollout-2026-10-21T00-00-00-${id}.jsonl`), rows.map((row) => JSON.stringify(row)).join("\n") + "\n");
      }
    }
    child = spawn(bridge || binary, bridge && !embeddedBridge ? ["--upstream", binary, "app-server"] : ["app-server"], {
      cwd: directory,
      env: {
        PATH: process.env.PATH, TMPDIR: tmpdir(), TMP: tmpdir(), TEMP: tmpdir(), CODEX_HOME: directory,
        ...(embeddedBridge ? { STATUS_FLOATER_UPSTREAM_CODEX: binary } : {}),
        ...(process.platform === "win32" ? { SystemRoot: process.env.SystemRoot || process.env.SYSTEMROOT, WINDIR: process.env.WINDIR } : {}),
        ...(followCurrent ? { STATUS_FLOATER_HISTORY_ROUTE: path.join(directory, "active-route.json") } : {}),
        ...(followCurrent ? { STATUS_FLOATER_BRIDGE_STATUS: bridgeStatusPath } : {}),
      },
      stdio: ["pipe", "pipe", "pipe"],
    });
    closed = new Promise((resolve) => child.once("close", resolve));
    let stderr = "";
    child.stderr.on("data", (data) => { stderr = (stderr + data.toString()).slice(-1200); });
    child.on("error", (error) => { for (const item of pending.values()) item.reject(error); });
    child.on("exit", (code) => { for (const item of pending.values()) item.reject(new Error(`App-server exited (${code}): ${stderr}`)); });
    readline.createInterface({ input: child.stdout }).on("line", (line) => {
      let message;
      try { message = JSON.parse(line); } catch { return; }
      if (message.method === "error" || message.method === "turn/completed") notices.push(message);
      const item = pending.get(message.id);
      if (!item) return;
      pending.delete(message.id);
      if (message.error) item.reject(new Error(message.error.message));
      else item.resolve(message.result);
    });
    let nextId = 0;
    function request(method, params) {
      const id = ++nextId;
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timeout: ${stderr}`)); }, 15000);
        pending.set(id, {
          resolve: (value) => { clearTimeout(timer); resolve(value); },
          reject: (reason) => { clearTimeout(timer); reject(reason); },
        });
        child.stdin.write(JSON.stringify({ id, method, params }) + "\n");
      });
    }
    const initialized = await request("initialize", { clientInfo: { name: "provider-history-fixture", version: "0.1" }, capabilities: null });
    child.stdin.write(JSON.stringify({ method: "initialized" }) + "\n");
    const results = [];
    for (const archived of [false, true]) {
      for (const mode of ["all", "null", "omitted"]) {
        const params = { archived, limit: 100, sortKey: "updated_at", useStateDbOnly: false };
        if (mode !== "omitted") params.modelProviders = mode === "all" ? [] : null;
        const response = await request("thread/list", params);
        results.push({ archived, mode, providers: response.data.map((thread) => thread.modelProvider).sort() });
      }
    }
    console.log(JSON.stringify({ binary: path.basename(binary), bridge: Boolean(bridge), userAgent: initialized.userAgent, results }, null, 2));
    if (results.some((result) => result.mode === "all" && result.providers.length !== 2)) throw new Error("Fixture inventory incomplete; cannot interpret provider filtering.");
    if (bridge && results.some((result) => result.providers.length !== 2)) throw new Error("Bridge failed to preserve cross-provider visibility.");
    try {
      const resumed = await request("thread/resume", { threadId: customActiveId, modelProvider: followCurrent ? "fixture_custom" : "openai", model: followCurrent ? "deepseek-test" : "gpt-5", approvalPolicy: "never", sandbox: "read-only" });
      if (followCurrent) {
        if (resumed.thread.id !== customActiveId || resumed.modelProvider !== (nativeOnly ? "openai" : "fixture_current") || resumed.model !== "gpt-5") throw new Error("Resume did not preserve thread ID and select the current model/provider");
        if (nativeOnly) {
          console.log(JSON.stringify({ nativeOpenAIResume: true, sameThread: true, model: resumed.model, inferenceRequested: false }));
          continue;
        }
        // Never send a turn to the native OpenAI endpoint. Inference uses a local fixture.
        await request("turn/start", { threadId: customActiveId, model: "deepseek-test", input: [{ type: "text", text: "Synthetic continuation", text_elements: [] }] });
        let timer;
        const observed = await Promise.race([requestObserved, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`No local inference request received: ${JSON.stringify(notices)} ${stderr}`)), 15000); })]).finally(() => clearTimeout(timer));
        const inputJson = JSON.stringify(observed.body.input);
        const backup = await readFile(path.join(directory, "status-floater", "history-compat-backups", `${customActiveId}.before-cross-provider.jsonl`), "utf8");
        const bridgeStatus = JSON.parse(await readFile(bridgeStatusPath, "utf8"));
        if (observed.body.model !== "gpt-5" || observed.auth !== "Bearer fixture-openai-key" || !inputJson.includes("Synthetic fixture_custom conversation") || !inputJson.includes("Synthetic visible assistant answer") || inputJson.includes("Synthetic private reasoning") || inputJson.includes("msg_synthetic_third_party") || inputJson.includes("Synthetic private tool output") || !backup.includes("Synthetic private reasoning") || !backup.includes("msg_synthetic_third_party") || !["working", "retrying", "failed"].includes(bridgeStatus.state)) throw new Error("Wrong route, missing visible context, provider-private items leaked, bridge status missing, or backup missing");
        console.log(JSON.stringify({ followCurrent: { sameThread: true, missingOldProviderDefinition: true, inferenceProvider: "fixture_current", model: observed.body.model, visibleContextIncluded: true, providerPrivateItemsRemoved: true, originalBackupCreated: true, bridgeStatusVisible: true, credentialMatchedTarget: true, syntheticLocalEndpointOnly: true } }, null, 2));
        await writeFile(path.join(directory, "active-route.json"), JSON.stringify({ model: "gpt-5", modelProvider: "openai" }));
        let blocked = false;
        try { await request("turn/start", { threadId: customActiveId, input: [{ type: "text", text: "Must not be forwarded", text_elements: [] }] }); }
        catch (error) { blocked = error.message.includes("立即重启"); }
        if (!blocked) throw new Error("Changed connection was allowed to reuse the old runtime");
        console.log(JSON.stringify({ staleConnectionBlocked: true }));
        continue;
      }
      const afterResume = await request("thread/list", { archived: false, modelProviders: null, limit: 100, useStateDbOnly: true });
      const afterRepair = await request("thread/list", { archived: false, modelProviders: null, limit: 100, useStateDbOnly: false });
      console.log(JSON.stringify({ resumeOverrideExperiment: {
        returnedProvider: resumed.modelProvider,
        threadProvider: resumed.thread.modelProvider,
        visibleInDatabase: afterResume.data.some((thread) => thread.id === customActiveId),
        visibleAfterMetadataRepair: afterRepair.data.some((thread) => thread.id === customActiveId),
      } }, null, 2));
    } catch (error) {
      if (followCurrent) throw error;
      console.log(JSON.stringify({ resumeOverrideExperiment: { error: error.message } }, null, 2));
    }
  } finally {
    if (child) {
      child.kill();
      await closed;
    }
    if (server) await new Promise((resolve) => server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  }
}
