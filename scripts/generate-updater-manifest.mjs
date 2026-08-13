import { readFile, readdir, rename, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";

const releaseDirectory = path.resolve(process.argv[2] ?? "release");
const repository = process.env.GITHUB_REPOSITORY ?? "0x0BB9/codex-status";
const packageJson = JSON.parse(
  await readFile(new URL("../package.json", import.meta.url), "utf8"),
);
const version = packageJson.version;
const tag = `v${version}`;
const notesPath = new URL(`../.github/release-notes/${tag}.md`, import.meta.url);
const notes = await readFile(notesPath, "utf8");

for (const filename of await readdir(releaseDirectory)) {
  const normalized = filename.replaceAll(" ", ".");
  if (normalized !== filename) {
    await rename(
      path.join(releaseDirectory, filename),
      path.join(releaseDirectory, normalized),
    );
  }
}

const files = await readdir(releaseDirectory);

function requireSingleFile(suffix, label) {
  const matches = files.filter((filename) => filename.endsWith(suffix));
  if (matches.length !== 1) {
    throw new Error(`Expected one ${label}, found ${matches.length}.`);
  }
  return matches[0];
}

async function platformEntry(signatureSuffix, label) {
  const signatureName = requireSingleFile(signatureSuffix, `${label} signature`);
  const assetName = signatureName.slice(0, -4);
  await stat(path.join(releaseDirectory, assetName));
  const signature = (
    await readFile(path.join(releaseDirectory, signatureName), "utf8")
  ).trim();
  if (!signature) {
    throw new Error(`${label} signature is empty.`);
  }

  return {
    signature,
    url: `https://github.com/${repository}/releases/download/${tag}/${encodeURIComponent(assetName)}`,
  };
}

const manifest = {
  version,
  notes: notes.trim(),
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": await platformEntry(".exe.sig", "Windows updater"),
    "darwin-aarch64": await platformEntry(
      ".app.tar.gz.sig",
      "macOS updater",
    ),
  },
};

await writeFile(
  path.join(releaseDirectory, "latest.json"),
  `${JSON.stringify(manifest, null, 2)}\n`,
);
