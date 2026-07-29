import { copyFile, mkdir, chmod } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const TARGETS = {
  "darwin-arm64": {
    packageName: "@openai/codex-darwin-arm64",
    targetTriple: "aarch64-apple-darwin",
  },
  "darwin-x64": {
    packageName: "@openai/codex-darwin-x64",
    targetTriple: "x86_64-apple-darwin",
  },
  "win32-arm64": {
    packageName: "@openai/codex-win32-arm64",
    targetTriple: "aarch64-pc-windows-msvc",
  },
  "win32-x64": {
    packageName: "@openai/codex-win32-x64",
    targetTriple: "x86_64-pc-windows-msvc",
  },
};

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const target = TARGETS[`${process.platform}-${process.arch}`];

if (!target) {
  throw new Error(`Codex sidecar is not configured for ${process.platform}-${process.arch}.`);
}

const codexPackageJson = import.meta.resolve("@openai/codex/package.json");
const requireFromCodex = createRequire(codexPackageJson);
const platformPackageJson = requireFromCodex.resolve(`${target.packageName}/package.json`);
const platformPackageRoot = path.dirname(platformPackageJson);
const executableName = process.platform === "win32" ? "codex.exe" : "codex";
const sourcePath = path.join(
  platformPackageRoot,
  "vendor",
  target.targetTriple,
  "bin",
  executableName,
);
const outputDirectory = path.join(projectRoot, "src-tauri", "binaries");
const outputPath = path.join(
  outputDirectory,
  `codex-app-server-${target.targetTriple}${process.platform === "win32" ? ".exe" : ""}`,
);

const versionCheck = spawnSync(sourcePath, ["--version"], {
  encoding: "utf8",
  windowsHide: true,
});

if (versionCheck.status !== 0) {
  throw new Error(
    `Unable to run Codex sidecar source: ${versionCheck.stderr || versionCheck.error || "unknown error"}`,
  );
}

await mkdir(outputDirectory, { recursive: true });
await copyFile(sourcePath, outputPath);

if (process.platform !== "win32") {
  await chmod(outputPath, 0o755);
}

console.log(`Prepared ${versionCheck.stdout.trim()} sidecar at ${outputPath}`);
