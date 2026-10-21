# Codex Status Floater

English | [简体中文](README.zh-CN.md)

A small Tauri utility window that keeps two Codex pain points visible:

- current ChatGPT / Codex rate-limit buckets
- live thread status changes across recent projects

It reads data from the official `codex app-server` surface and is designed to stay pinned beside your main workspace.
Packaged builds include the official Codex native binary as a local sidecar, so recipients do not need to install Node.js, npm, pnpm, or the Codex CLI.

## What It Shows

- account mode and plan summary
- saved local Codex accounts with one-click switching
- imported or manually configured Responses-compatible API connections, switchable alongside Plus accounts
- invalid-account markers with re-login and safe deletion for inactive accounts
- short-window and long-window usage, reset times, and available reset-credit expirations
- recent threads sorted by latest activity
- local task board metadata: pinned threads, notes, project, priority, and stage
- live status notifications such as `thread/status/changed`
- tray menu controls, a global window toggle, transparent floating, and edge snapping
- right-edge auto-collapse into a small usage capsule with click-to-expand
- signed in-app update checks, installation, and relaunch on Windows and macOS
- a one-time post-update summary with release notes
- a tiny local server log when the app launches `codex app-server` itself

## Account Switching

The app implements local account switching itself. It does not require `codex-auth`.

- Saved accounts live under `~/.codex/accounts/`.
- The active account is still the standard `~/.codex/auth.json` used by Codex.
- The app automatically enables file-backed Codex credentials before adding, saving, or switching an account and backs up an existing config before changing it.
- Clicking "登录添加账号" starts the official Codex app-server login flow, then saves the completed auth as a local snapshot.
- Clicking "保存当前账号" stores the currently logged-in Codex auth as a local snapshot.
- Switching to a Plus account first saves the active account's latest tokens, replaces `auth.json`, then restarts app-server and verifies the existing login without forcing refresh-token rotation.
- Failed verification automatically restores the previous account instead of leaving Codex signed out.
- Failed target accounts are marked for re-login. A successful official login clears the marker, while inactive accounts can be deleted after confirmation.
- API keys are stored long-term in macOS Keychain or Windows Credential Manager; the registry stores only a last-four-character hint.
- Activating an API connection preserves the Plus login and switches the provider and model catalog. Switching back restores the GPT configuration while retaining key-free definitions required by older chats.
- Restart the official client from this tool to enable the native compatibility bridge. It queries all providers by default and resumes old chats with the active connection and configured model, without migrating stored history. A changed connection requires another restart before sending messages.
- On first cross-provider resume, the original rollout is backed up and only visible user/assistant messages are replayed. Provider-private reasoning, function calls, tool outputs, and web-search items are excluded; the thread ID remains intact. Backups stay under `CODEX_HOME/status-floater/history-compat-backups/`.
- While the compatibility bridge is active, the floater shows the desktop task's working, retrying, failed, and completed states, including retry progress for overloaded services.
- While an API connection is active, its key is copied only into the user-readable provider configuration; connection backups redact that bearer token.
- History protection runs during account switching. An issue reveals a recovery entry for cross-provider records, message viewing, JSON export, and confirmed unarchiving; there is no permanent history panel.
- Before switching, the app backs up readable local conversation files. It checks thread IDs, provider bindings, and newly archived records after switching and again after a successful desktop restart, reporting history issues separately from authentication failures.
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

Release notes and version history are available in [CHANGELOG.md](CHANGELOG.md).

## App Updates

Click the version badge in the header to check for updates. The app also performs one silent check after startup.

- Windows and macOS download and install signed updater artifacts in-app, then relaunch.
- After relaunch, the app shows the completed version transition and release notes once.
- On macOS, copy the app to `/Applications` before using in-app updates; do not run it directly from the DMG.
- macOS must be upgraded manually once to the first release that supports automatic installation; later releases can update in-app.

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

## Reset Credit Data

Account, task, and normal rate-limit data continue to come from the official `codex app-server`. Because the current app-server protocol does not expose available reset-credit counts, the app reads the active `auth.json` session only for this supplemental value and makes a read-only request to the ChatGPT reset-credit endpoint. Tokens, account IDs, and raw responses are never persisted or logged. If this endpoint is unavailable or changes shape, only the reset-credit summary is hidden; the rest of the dashboard keeps working.

## Build Notes

- `.cargo/config.toml` points Cargo at the `rsproxy` sparse index to make crate downloads more reliable on mainland networks.
- `src-tauri/patches/` contains vendored local patches for `cookie`, `tauri-utils`, and `tauri`.
- Those patches only narrow a few overly broad generic `From<T>` impls so the project compiles cleanly on current Rust toolchains while preserving the runtime behavior this app needs.

## Regenerate Protocol Types

If Codex updates the app-server protocol, regenerate the TypeScript bindings with:

```bash
pnpm protocol:generate
```
