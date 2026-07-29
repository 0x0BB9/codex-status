# Codex Status Floater

A small Tauri utility window that keeps two Codex pain points visible:

- current ChatGPT / Codex rate-limit buckets
- live thread status changes across recent projects

It reads data from the official `codex app-server` surface and is designed to stay pinned beside your main workspace.
Packaged builds include the official Codex native binary as a local sidecar, so recipients do not need to install Node.js, npm, pnpm, or the Codex CLI.

## What It Shows

- account mode and plan summary
- saved local Codex accounts with one-click switching
- short-window and long-window usage for each rate-limit bucket
- recent threads sorted by latest activity
- local task board metadata: pinned threads, notes, project, priority, and stage
- live status notifications such as `thread/status/changed`
- tray menu controls, a global window toggle, transparent floating, and edge snapping
- right-edge auto-collapse into a small usage capsule with hover-to-expand
- a tiny local server log when the app launches `codex app-server` itself

## Account Switching

The app implements local account switching itself. It does not require `codex-auth`.

- Saved accounts live under `~/.codex/accounts/`.
- The active account is still the standard `~/.codex/auth.json` used by Codex.
- The app automatically enables file-backed Codex credentials before adding, saving, or switching an account and backs up an existing config before changing it.
- Clicking "登录添加账号" starts the official Codex app-server login flow, then saves the completed auth as a local snapshot.
- Clicking "保存当前账号" stores the currently logged-in Codex auth as a local snapshot.
- Clicking "切换" first saves the active account's latest tokens, replaces `auth.json`, then forces Codex to refresh and verify the target account.
- Failed verification automatically restores the previous account instead of leaving Codex signed out.
- The account panel can restart the official Codex desktop client on macOS or Windows after a verified switch. Newer ChatGPT desktop builds may keep a separate host session that still needs confirmation inside the official client.

See [docs/account-switching.md](docs/account-switching.md) for the full file layout, switching flow, backups, compatibility notes, and limitations.

## Local Task Board

Task board annotations are local-only and do not modify Codex thread state.

- Mark important threads as pinned so they stay at the top.
- Add short notes such as "等接口", "等设计", or "今日处理".
- Classify threads by project, priority, and stage, then group the board by those fields.

See [docs/thread-board.md](docs/thread-board.md) for the storage location and field details.

## Distribution

For packaging and sharing the app with other users, see [docs/distribution.md](docs/distribution.md). Windows-specific behavior and build instructions are in [docs/windows.md](docs/windows.md).

## Local Web Preview

Run:

```bash
pnpm dev
```

This is a UI preview only. Live Codex data is wired through the Tauri stdio bridge because browser websocket handshakes are rejected by the local app-server.

## Tauri Desktop Run

From this directory, run:

```bash
pnpm install
pnpm tauri dev
```

The Tauri shell is configured to:

- stay always on top
- stay visible as a transparent floating utility window
- expose a system tray icon for show/hide and quit
- toggle the window globally with `Option + Space` on macOS or `Ctrl + Alt + Space` on Windows
- snap to screen edges after dragging near a boundary
- dim after idle and collapse into a right-edge capsule when docked to the screen edge
- appear as a narrow side utility window
- launch the bundled official `codex app-server` sidecar over stdio, with installed desktop apps and PATH as compatibility fallbacks
- manage local Codex account snapshots through Tauri commands

## Build Notes

- `.cargo/config.toml` points Cargo at the `rsproxy` sparse index to make crate downloads more reliable on mainland networks.
- `src-tauri/patches/` contains vendored local patches for `cookie`, `tauri-utils`, and `tauri`.
- Those patches only narrow a few overly broad generic `From<T>` impls so the project compiles cleanly on current Rust toolchains while preserving the runtime behavior this app needs.

## Regenerate Protocol Types

If Codex updates the app-server protocol, regenerate the TypeScript bindings with:

```bash
pnpm protocol:generate
```
