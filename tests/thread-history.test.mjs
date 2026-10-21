import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";

const source = readFileSync(new URL("../src/lib/thread-history.ts", import.meta.url), "utf8");
const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 } });
const { listThreadHistory, compareThreadHistory } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString("base64")}`);
const entry = (id, archived = false, provider = "openai") => ({ thread: { id, modelProvider: provider, updatedAt: 1 }, archived });

test("backend string errors remain visible to the user", async () => {
  const source = readFileSync(new URL("../src/lib/error-message.ts", import.meta.url), "utf8");
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } }).outputText;
  const { getErrorMessage } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString("base64")}`);
  assert.equal(getErrorMessage("backup path denied", "generic"), "backup path denied");
  assert.equal(getErrorMessage(new Error("RPC timeout"), "generic"), "RPC timeout");
  assert.equal(getErrorMessage({ message: "invalid args" }, "generic"), "invalid args");
  assert.equal(getErrorMessage(null, "generic"), "generic");
});

test("history includes paginated active and archived records from every provider", async () => {
  const calls = [];
  const history = await listThreadHistory({ listThreads: async (params) => {
    calls.push(params);
    assert.deepEqual(params.modelProviders, []);
    if (params.archived) return { data: [entry("deepseek", true, "custom").thread], nextCursor: null };
    return params.cursor ? { data: [entry("second").thread], nextCursor: null }
      : { data: [entry("first").thread], nextCursor: "page2" };
  } });
  assert.equal(calls.length, 3);
  assert.deepEqual(history.map((e) => [e.thread.id, e.archived]), [["first", false], ["second", false], ["deepseek", true]]);
});

test("a new task cannot mask a missing task with the same total count", () => {
  const result = compareThreadHistory([entry("old")], [entry("new")]);
  assert.deepEqual(result.missing.map((e) => e.thread.id), ["old"]);
});

test("newly archived and provider changes are distinct from missing history", () => {
  const result = compareThreadHistory([entry("a"), entry("b", true), entry("c")], [entry("a", true), entry("b", true), entry("c", false, "custom")]);
  assert.equal(result.missing.length, 0);
  assert.deepEqual(result.newlyArchived.map((e) => e.thread.id), ["a"]);
  assert.deepEqual(result.providerChanged.map((e) => e.thread.id), ["c"]);
});

test("repeated cursors and failed archive queries cannot silently produce a partial snapshot", async () => {
  await assert.rejects(() => listThreadHistory({ listThreads: async () => ({ data: [], nextCursor: "loop" }) }), /分页异常/);
  await assert.rejects(() => listThreadHistory({ listThreads: async ({ archived }) => {
    if (archived) throw new Error("archive unavailable");
    return { data: [entry("a").thread], nextCursor: null };
  } }), /archive unavailable/);
});
