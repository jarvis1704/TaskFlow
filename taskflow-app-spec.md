# TaskFlow — Technical & Product Specification

> A native, lightweight Linux desktop task manager that syncs bidirectionally with Google Tasks.
> This document is written to be read and implemented end-to-end by an AI coding agent or a human developer. It covers architecture, data model, auth flow, sync logic, UI/UX design system, animations, crate choices, and packaging.

---

## 1. Product Summary

**Name:** TaskFlow (placeholder — rename freely)
**Platform:** Linux desktop (Wayland-first, tested under Hyprland/Omarchy; X11 compatible)
**Language:** Rust
**UI Framework:** `iced` (primary recommendation) — `egui` as fallback option (see §3)
**Core value prop:** A fast, native, low-memory task manager with a polished UI that stays in sync with Google Tasks, so the user remains in the Google ecosystem while getting a better desktop-native experience and reminder system.

### Non-goals
- No multi-provider sync (Todoist, Microsoft To Do, etc.) — Google Tasks only, for now.
- No mobile app.
- No real-time push sync (webhooks) — polling is sufficient and far simpler for a single-user desktop app.
- No collaborative/shared task lists in v1.

---

## 2. High-Level Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        TaskFlow.app                          │
│                                                               │
│  ┌────────────┐   ┌──────────────┐   ┌────────────────────┐ │
│  │   UI Layer │←→│  App Core /  │←→│   Sync Engine        │ │
│  │ (iced/egui)│   │  State Mgmt  │   │ (Google Tasks REST) │ │
│  └────────────┘   └──────┬───────┘   └─────────┬──────────┘ │
│                           │                     │            │
│                    ┌──────▼───────┐      ┌──────▼───────┐    │
│                    │  SQLite (local)│     │  OAuth2 Token │   │
│                    │  via rusqlite  │     │  Manager      │   │
│                    └────────────────┘     └──────┬───────┘    │
│                                                   │            │
│                                          ┌────────▼────────┐  │
│                                          │ Secret Service   │  │
│                                          │ (libsecret/      │  │
│                                          │ keyring crate)   │  │
│                                          └──────────────────┘  │
│                                                               │
│  ┌──────────────────────────┐   ┌──────────────────────────┐ │
│  │ Notification Service      │   │ systemd user service /   │ │
│  │ (notify-rust / D-Bus)     │   │ background daemon mode   │ │
│  └──────────────────────────┘   └──────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

### Process model
Two binaries built from one workspace:

1. **`taskflow-gui`** — the main windowed application. Launched normally from the app launcher.
2. **`taskflow-daemon`** — a tiny headless background process that wakes on a timer (driven by a systemd user timer or its own internal tokio interval), checks for due tasks, fires desktop notifications, and performs periodic sync even when the GUI isn't open. The GUI and daemon communicate through the shared SQLite database (file-based, with a lightweight file lock) — no need for IPC sockets in v1.

This separation is what keeps the "always-on" footprint tiny: the daemon does no rendering, holds no GPU context, and can idle at near-zero CPU between wakeups.

---

## 3. Technology Stack & Crate Choices

| Concern | Crate | Notes |
|---|---|---|
| GUI framework | `iced` (0.13+) | Elm-architecture style (Message/Update/View), GPU-accelerated via `wgpu`, good animation primitives via `iced_anim` or manual tweening. Pick this over `egui` if you want a more "designed," app-like UI rather than an immediate-mode tool/debug-panel look. |
| Alt GUI framework | `egui` + `eframe` | Simpler immediate-mode model, faster to prototype, slightly less suited to elaborate custom animations/transitions but very capable and even lighter to compile. Use this if development speed matters more than visual polish in v1. |
| Async runtime | `tokio` | Required by `iced`'s subscription model and for HTTP calls. |
| HTTP client | `reqwest` (rustls-tls feature, NOT openssl, to avoid system OpenSSL dependency issues) | Used for OAuth token exchange and Google Tasks API calls. |
| OAuth2 | `oauth2` crate | Implements PKCE, auth code exchange, token refresh per RFC 6749/7636. |
| Local DB | `rusqlite` (bundled feature) | Bundled SQLite means no system libsqlite3 dependency — keeps the binary portable. |
| Secret storage | `keyring` crate | Wraps Secret Service (libsecret) on Linux. Falls back gracefully; surface an error state in UI if no keyring daemon is running (rare under Omarchy default setup, but possible on minimal WMs). |
| Desktop notifications | `notify-rust` | Wraps `org.freedesktop.Notifications` over D-Bus. |
| Tray icon (optional) | `ksni` (StatusNotifierItem) | Only needed if you want a persistent tray icon instead of relying purely on the daemon. |
| Serialization | `serde`, `serde_json` | For Google API payloads and local config. |
| Date/time | `chrono` or `time` | `time` is lighter weight; `chrono` has nicer API ergonomics. Either is fine. |
| Logging | `tracing` + `tracing-subscriber` | Structured logs, useful for debugging sync issues. |
| Config | `directories` crate | Resolves XDG base directories correctly (`~/.config/taskflow`, `~/.local/share/taskflow`). |

### Why iced over egui (recommendation)
Since the user explicitly wants "sleek, modern, productive-looking UI with great animations," `iced`'s retained-mode widget tree and theming system make it considerably easier to do smooth state-driven transitions (e.g., a task sliding out on completion, a panel easing open). `egui`'s immediate-mode model can do animation too (it's just lerping values every frame), but composing it into long-lived layered transitions tends to require more manual bookkeeping. Either is "Rust-native and light" — this is purely a UI-polish tradeoff.

---

## 4. Authentication: Google OAuth2 Desktop Flow (PKCE)

**Do not use Firebase Auth.** Use a direct Google Cloud OAuth2 "Desktop app" client. Rationale already established: Firebase Auth is designed for authenticating into a Firebase backend, not for obtaining long-lived, refreshable, scoped access tokens for the Google Tasks API.

### 4.1 Google Cloud Console setup (one-time, manual, documented for the human)
1. Create a project in Google Cloud Console.
2. Enable the **Google Tasks API**.
3. Create OAuth consent screen (Testing or Production mode; "Testing" is fine for personal use, supports up to 100 test users without verification).
4. Create OAuth Client ID → Application type: **Desktop app**.
5. Note the `client_id` and `client_secret` (for a desktop/native app, the secret isn't really "secret" since it ships in the binary — Google's PKCE flow accounts for this; treat it as a public identifier, not a real secret).
6. Scope required: `https://www.googleapis.com/auth/tasks`

### 4.2 Flow sequence
1. App generates a PKCE `code_verifier` and derived `code_challenge` (S256).
2. App starts a temporary local HTTP listener on `127.0.0.1:<random free port>`.
3. App opens the user's default browser to Google's auth URL:
   ```
   https://accounts.google.com/o/oauth2/v2/auth
     ?client_id=...
     &redirect_uri=http://127.0.0.1:<port>/callback
     &response_type=code
     &scope=https://www.googleapis.com/auth/tasks
     &code_challenge=...
     &code_challenge_method=S256
     &access_type=offline
     &prompt=consent
   ```
   - `access_type=offline` is what makes Google issue a **refresh token**.
   - `prompt=consent` ensures a refresh token is reissued even on repeat authorizations (Google sometimes omits it on subsequent logins otherwise).
4. User logs in / grants consent in their normal browser (keeps their existing Google session, passkeys, 2FA, etc. — the app never touches the password).
5. Google redirects to `http://127.0.0.1:<port>/callback?code=...`.
6. The local listener captures the code, immediately closes, and shows a simple static "You may close this tab / return to TaskFlow" HTML response.
7. App exchanges the code (`code` + `code_verifier`) for an access token + refresh token via `https://oauth2.googleapis.com/token`.
8. **Access token** (short-lived, ~1hr) is kept in memory only.
9. **Refresh token** (long-lived) is written to the OS keyring via the `keyring` crate, under a service name like `taskflow` / account `google-tasks`.
10. On every subsequent app launch, the refresh token is read from keyring and silently exchanged for a fresh access token at startup — no browser popup needed unless the refresh token is revoked.

### 4.3 Token refresh during runtime
Before every Google Tasks API call, check access-token expiry (track `expires_in` and obtained-at timestamp). If within ~60 seconds of expiry, refresh proactively. Implement this as a thin wrapper/middleware around the `reqwest` client used for Tasks calls so call sites never need to think about token lifecycle.

### 4.4 Revocation / Sign-out
Provide a "Disconnect Google Account" setting that:
- Calls Google's revoke endpoint (`https://oauth2.googleapis.com/revoke?token=...`) with the refresh token.
- Deletes the token from the OS keyring.
- Clears the local `google_task_id` mappings (tasks remain locally, just unsynced) or optionally offers to wipe local data too.

---

## 5. Google Tasks API Integration

Base URL: `https://tasks.googleapis.com/tasks/v1/`

### 5.1 Key endpoints used
| Purpose | Method & Path |
|---|---|
| List task lists | `GET /users/@me/lists` |
| Create a task list | `POST /users/@me/lists` |
| List tasks in a list | `GET /lists/{tasklistId}/tasks` (supports `showCompleted`, `showHidden`, `updatedMin`) |
| Get single task | `GET /lists/{tasklistId}/tasks/{taskId}` |
| Insert task | `POST /lists/{tasklistId}/tasks` |
| Update task | `PATCH /lists/{tasklistId}/tasks/{taskId}` |
| Delete task | `DELETE /lists/{tasklistId}/tasks/{taskId}` |
| Move task (reorder/reparent for subtasks) | `POST /lists/{tasklistId}/tasks/{taskId}/move` |

### 5.2 Task resource fields that matter
```json
{
  "id": "string (Google-assigned, immutable)",
  "title": "string",
  "notes": "string",
  "status": "needsAction | completed",
  "due": "RFC3339 date-time string, but Google only honors the DATE portion",
  "completed": "RFC3339 date-time, set when status=completed",
  "updated": "RFC3339 date-time, server-managed, used for sync diffing",
  "parent": "id of parent task, for subtasks",
  "position": "string, lexicographic ordering key"
}
```

**Critical nuance:** The `due` field is date-only in practice — Google Tasks' API does not support a time-of-day component for reminders, even though `due` is typed as a full date-time string (it's always normalized to midnight UTC). **TaskFlow's own reminder time-of-day must be stored locally** (in the SQLite `tasks` table) and is not synced to Google. Only the date syncs. Document this clearly in the UI (e.g., a small "🔔 local reminder" indicator next to tasks with a custom time set) so the user isn't confused about why the time doesn't show up in the Google Tasks mobile app.

### 5.3 Rate limits
Google Tasks API default quota is generous for a single-user app (per-user limits are not typically a concern at polling intervals of a few minutes). Still implement exponential backoff on `429`/`5xx` responses.

---

## 6. Local Data Model (SQLite)

```sql
CREATE TABLE task_lists (
    id              TEXT PRIMARY KEY,         -- local UUID
    google_id       TEXT UNIQUE,              -- null until first synced
    title           TEXT NOT NULL,
    position        INTEGER NOT NULL DEFAULT 0,
    is_default      INTEGER NOT NULL DEFAULT 0,
    updated_at      TEXT NOT NULL
);

CREATE TABLE tasks (
    id              TEXT PRIMARY KEY,         -- local UUID
    google_id       TEXT UNIQUE,              -- null until first synced
    list_id         TEXT NOT NULL REFERENCES task_lists(id),
    title           TEXT NOT NULL,
    notes           TEXT,
    status          TEXT NOT NULL DEFAULT 'needsAction', -- needsAction | completed
    due_date        TEXT,                      -- date only, synced to Google
    reminder_time   TEXT,                      -- local-only HH:MM, not synced
    parent_id       TEXT REFERENCES tasks(id),  -- subtasks
    position        TEXT,
    updated_at      TEXT NOT NULL,             -- local last-modified
    google_updated_at TEXT,                    -- last known server 'updated' value
    sync_state      TEXT NOT NULL DEFAULT 'pending', -- pending | synced | conflict | deleted_pending
    is_deleted      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE sync_meta (
    key             TEXT PRIMARY KEY,
    value           TEXT
);
-- e.g. key='last_full_sync_at', key='default_list_google_id'
```

`sync_state` values drive the sync engine's behavior:
- `pending` — created/edited locally, not yet pushed.
- `synced` — matches last known server state.
- `conflict` — both local and remote changed since last sync; needs resolution.
- `deleted_pending` — deleted locally, deletion not yet pushed to Google.

---

## 7. Sync Engine

### 7.1 Triggers
- On app launch.
- On a timer (default every 5 minutes; configurable), run by both GUI (while open) and daemon (always).
- On explicit user action (manual "Sync now" button, with a visible spinner/animation).
- Immediately after any local mutation (debounced ~2s) so changes propagate quickly without hammering the API on every keystroke.

### 7.2 Algorithm (per task list)
1. **Pull phase:** `GET tasks?updatedMin={last_sync_timestamp}&showCompleted=true&showHidden=true`. Note: `updatedMin` is exclusive-ish in practice — fetch with a few seconds of overlap and de-dupe by `id` to avoid edge-case misses.
2. For each remote task returned:
   - If `google_id` not found locally → insert new local row, `sync_state = synced`.
   - If found locally and local `sync_state == synced` → overwrite local fields with remote (remote wins, nothing changed locally).
   - If found locally and local `sync_state == pending` → **conflict**. Resolve via `updated_at` (local) vs `updated` (remote) timestamp — most-recent-write-wins by default. Surface a non-blocking toast ("Resolved a sync conflict on 'Buy groceries' — kept your latest edit") rather than a blocking dialog, to keep the UX calm.
3. **Push phase:** For every local row with `sync_state == pending`:
   - If `google_id` is null → `POST` (insert), store returned `id` as `google_id`.
   - Else → `PATCH` with changed fields.
   - Update local `sync_state = synced`, `google_updated_at = <server updated value>`.
4. For every local row with `sync_state == deleted_pending`:
   - `DELETE` remotely, then hard-delete (or soft-delete, your call) the local row.
5. Update `sync_meta.last_full_sync_at`.

### 7.3 Offline behavior
All mutations always go to SQLite first (optimistic local-first writes), regardless of connectivity. The sync engine simply has nothing to push until connectivity returns. Detect connectivity failures via `reqwest` error kind and back off (e.g., 30s → 1min → 5min escalating retry, capped) rather than retrying constantly.

---

## 8. Background Daemon & Reminders

- `taskflow-daemon` is a minimal tokio binary: on an interval (e.g., every 60s), it queries SQLite for tasks where `reminder_time` matches "now" (within the current minute) and `status = needsAction`, and fires a notification via `notify-rust`.
- It also runs the sync engine on its own longer interval independently of whether the GUI is open.
- Installed as a **systemd user service** (`~/.config/systemd/user/taskflow-daemon.service`) with `WantedBy=default.target`, so it starts on login and the user doesn't need to manually launch it.
- Memory footprint target: a tokio async binary with no GUI/GPU context idling on a timer should comfortably sit under ~5–10MB RSS.

Example unit file to generate during install/setup:
```ini
[Unit]
Description=TaskFlow background sync & reminders

[Service]
ExecStart=%h/.local/bin/taskflow-daemon
Restart=on-failure

[Install]
WantedBy=default.target
```

---

## 9. UI / UX Design System

The goal: **sleek, modern, "productive" feeling** — think a calmer, native-feeling cousin of apps like Linear, Things 3, or Todoist's cleaner views, but rendered natively and fast, fitting naturally into a dark, minimal Hyprland/Omarchy desktop aesthetic.

### 9.1 Visual language

**Theme:** Dark-first (with an optional light theme), since this is built for a Hyprland/Omarchy user — but build theming as a first-class concept (a `Theme` struct with semantic color tokens), not hardcoded colors, so light mode and even custom user themes are trivial to add later.

**Color tokens** (dark theme defaults — tune to taste, but keep this structure):

| Token | Example value | Usage |
|---|---|---|
| `bg.base` | `#15161B` | App background |
| `bg.surface` | `#1C1E26` | Cards, panels, sidebar |
| `bg.surface-hover` | `#262833` | Hover state on rows/cards |
| `border.subtle` | `#2A2D3A` | Hairline dividers |
| `text.primary` | `#E7E8EC` | Main task titles |
| `text.secondary` | `#8B8D98` | Notes, metadata, timestamps |
| `accent.primary` | `#7C9EFF` (soft indigo-blue) | Active states, focus rings, primary buttons |
| `accent.success` | `#5FD9A4` | Completed tasks, success toasts |
| `accent.warning` | `#F2B86C` | Due-soon indicators |
| `accent.danger` | `#F2746C` | Overdue, delete actions |

Avoid pure black (`#000000`) and pure white — slightly tinted near-blacks/near-whites read as more "designed" and reduce harsh contrast/eye strain, which matters for an always-open productivity app.

**Typography:**
- UI font: **Inter** or **IBM Plex Sans** (both excellent, free, and render crisply at small sizes — common in modern productivity apps).
- Monospace accents (e.g., for due dates/times or keyboard shortcut hints): **JetBrains Mono** or **Iosevka** (fits the Omarchy/terminal-aesthetic crowd nicely).
- Type scale: 12px (meta/secondary), 14px (body/task titles), 16px (section headers), 22–28px (page titles/empty states).

**Spacing & layout:**
- Use an 8px base spacing grid (8/16/24/32) for consistent rhythm.
- Generous padding inside task rows (12–16px vertical) — cramped task lists feel stressful, not productive.
- Rounded corners: 8–10px on cards/panels, 6px on buttons/chips — soft but not "bubbly."
- Subtle elevation via soft shadows or a 1px border + slight background-shade shift (preferred over heavy drop-shadows for a flat, modern look).

### 9.2 Layout structure

```
┌──────────────┬──────────────────────────────────────────────┐
│              │  [Today]  [Upcoming]  [All Tasks]   🔄 Sync   │
│   Sidebar    ├──────────────────────────────────────────────┤
│              │                                                │
│  • Today  3  │   ☐ Buy groceries           Due today  🔔5pm  │
│  • My Tasks  │   ☐ Finish report           Due tomorrow      │
│  • Work   12 │   ☑ Call dentist  (struck through, faded)     │
│  • Personal  │                                                │
│              │   + Add a task...                             │
│  ──────────  │                                                │
│  ⚙ Settings  │                                                │
│  ⏻ Account   │                                                │
└──────────────┴──────────────────────────────────────────────┘
```

- **Sidebar:** task lists (mirrors Google Tasks lists), with live counts. Sync status indicator (small colored dot: green=synced, amber=syncing, red=error) near the account/settings area, not intrusive.
- **Main pane:** filtered/grouped task view. Default views: **Today**, **Upcoming** (next 7 days), **All Tasks**, plus one section per Google Tasks list.
- **Quick-add bar:** persistent at the bottom or top of the task list — typing and hitting Enter creates a task instantly (optimistic UI: it appears immediately, synced state badge updates after the sync engine confirms).
- **Task row:** checkbox, title, secondary line for notes/due date/reminder time, optional list/tag chip on the right.

### 9.3 Animation & motion design

This is where `iced`'s strengths matter. Recommended motion moments — each should be **fast** (120–220ms) and use an ease-out curve (`cubic-bezier(0.16, 1, 0.3, 1)`-equivalent or `ease_out_cubic` if implementing manually) so the app feels snappy, never sluggish:

1. **Task completion:** checkbox fills with a small checkmark draw-in (~150ms), title gets a strikethrough that animates left-to-right, then the row fades + collapses height (~200ms) before settling into the "completed" section (if completed tasks are grouped separately) or just visually receding (lower opacity, no removal) if shown inline.
2. **New task add:** the new row slides down + fades in from height 0 → full height, rather than just popping in.
3. **Delete/dismiss:** swipe-or-button-triggered row slides out horizontally + fades, with remaining rows animating upward to fill the gap (a simple list-reflow tween).
4. **Sidebar list switch:** content cross-fades (120ms) rather than hard-cutting, so switching between "Today"/"Upcoming"/lists feels continuous.
5. **Sync indicator:** subtle pulsing/rotating icon while syncing; on success, a brief green checkmark flash (~400ms) then fades back to idle dot — this gives quiet confidence the sync is working without being chatty.
6. **Hover states:** background-color transitions on row hover (~80ms) — short and snappy since these fire constantly with mouse movement; anything longer feels laggy.
7. **Modal/panel open (e.g., task detail view, settings):** scale-up-from-98%-to-100% + fade, combined with a slight backdrop dim — gives a sense of depth without heavy blur effects (which are expensive to render well in a lightweight app).
8. **Empty states:** when a list has zero tasks, show a calm illustration/icon with a subtle idle float/breathing animation (very slow, ~3s loop, ±4px) — purely decorative polish that reinforces "modern app" feel without being distracting.

**Implementation note for iced:** animations are typically done by storing animation progress (0.0–1.0) as state per animated element, advancing it via a `Subscription::time` tick (e.g., every frame or every 16ms while any animation is active, idle otherwise to save CPU), and interpolating layout/opacity/color properties accordingly. Only run the animation subscription/timer while at least one animation is in-flight — this is the key to keeping idle CPU usage near zero, which matters a lot for the "not a memory/CPU hog" goal.

### 9.4 Iconography
Use a consistent icon set — **Lucide** or **Phosphor Icons** (both have clean, modern, minimal line icons, free/open-source, and SVGs that can be embedded directly or rasterized at build time). Avoid mixing icon styles.

### 9.5 Keyboard-first interactions (fits a Hyprland/keyboard-driven user)
- `Ctrl+N` — quick add task (focuses the quick-add bar from anywhere).
- `Ctrl+K` — command palette / fuzzy task & list search (a command-palette pattern fits the target user perfectly).
- `J/K` or arrow keys — navigate task list.
- `Space` or `Enter` — toggle complete / open detail.
- `Ctrl+1..9` — jump to sidebar list N.
This isn't just a UX nicety — it should be considered core to the "productive" feel for the target user.

---

## 10. Application Module Layout (Rust workspace)

```
taskflow/
├── Cargo.toml                  # workspace root
├── crates/
│   ├── taskflow-core/          # shared logic, no UI deps
│   │   ├── src/
│   │   │   ├── db/             # rusqlite schema, queries, migrations
│   │   │   ├── google/         # Tasks API client, OAuth flow, token refresh
│   │   │   ├── sync/           # sync engine, conflict resolution
│   │   │   ├── models.rs       # Task, TaskList, SyncState, etc.
│   │   │   └── lib.rs
│   ├── taskflow-gui/            # iced application
│   │   ├── src/
│   │   │   ├── theme.rs        # color tokens, typography, Theme struct
│   │   │   ├── widgets/        # custom task row, sidebar, quick-add, animations
│   │   │   ├── views/          # today.rs, upcoming.rs, list_view.rs, settings.rs
│   │   │   ├── app.rs          # root Application impl (Message/Update/View)
│   │   │   └── main.rs
│   └── taskflow-daemon/         # headless background binary
│       └── src/main.rs
├── assets/
│   ├── icons/                  # Lucide/Phosphor SVGs
│   └── fonts/                  # Inter, JetBrains Mono (bundled, licensed for redistribution)
└── packaging/
    ├── flatpak/
    ├── systemd/taskflow-daemon.service
    └── debian/ (optional .deb control files)
```

`taskflow-core` having zero UI dependencies is important: it lets `taskflow-gui` and `taskflow-daemon` both depend on it without either pulling in unnecessary deps (the daemon should never link against `iced`/`wgpu`).

---

## 11. Settings / Configuration

Stored at `~/.config/taskflow/config.toml` (via `directories` crate for proper XDG resolution):

```toml
[sync]
interval_minutes = 5
auto_sync_on_launch = true

[notifications]
enabled = true
default_reminder_lead_minutes = 0   # 0 = at due time, can support "15 min before" etc later

[ui]
theme = "dark"          # dark | light | system
default_view = "today"

[google]
# no secrets here — only non-sensitive state, e.g.:
last_synced_account_email = "user@gmail.com"
```

Refresh tokens **never** go in this file — keyring only.

---

## 12. Packaging & Distribution

1. **Flatpak (recommended primary target):**
   - Plays well with sandboxing; use the `org.freedesktop.Secret` and notification portals so it works correctly even sandboxed.
   - Manifest references `org.gnome.Platform` or a minimal freedesktop runtime — doesn't need GNOME-specific runtime since this isn't a GTK app, a bare freedesktop runtime is lighter.
2. **AppImage:** simple fallback for users who don't want Flatpak; bundle the binary + assets, no sandboxing concerns since it's a single trusted-user desktop tool.
3. **AUR / native package (optional, given Omarchy is Arch-based):** a `PKGBUILD` that builds from source via `cargo build --release` is very idiomatic for this audience and arguably the most natural distribution channel for the target user base.
4. Ship both `taskflow-gui` and `taskflow-daemon` binaries, plus an install step that copies/enables the systemd user service.

---

## 13. Testing Strategy

- **Unit tests** in `taskflow-core`: sync conflict resolution logic, DB query correctness, OAuth token refresh logic (mock the token endpoint).
- **Integration tests:** spin up an in-memory SQLite DB, simulate a mocked Google Tasks API (via `wiremock` crate) to test the full pull/push sync cycle including conflict scenarios.
- **Manual QA checklist:** offline create → reconnect → sync; concurrent edit in Google Tasks web UI + local edit → verify conflict resolution and toast; reminder fires at correct local time; daemon survives GUI being closed; keyring missing/locked edge case shows a sensible error rather than crashing.

---

## 14. Build Order / Milestones (suggested for an agent or solo dev)

1. **M1 — Core data layer:** SQLite schema + migrations, `taskflow-core` models, basic CRUD, no networking yet.
2. **M2 — OAuth & API client:** implement PKCE flow, token storage in keyring, Google Tasks REST client (list/insert/patch/delete).
3. **M3 — Sync engine:** pull/push logic, conflict resolution, tested against mocked API.
4. **M4 — Minimal GUI shell:** iced app skeleton, sidebar + task list view, no animations yet, wired to `taskflow-core`.
5. **M5 — Design system pass:** apply theme tokens, typography, spacing per §9; implement quick-add, today/upcoming views.
6. **M6 — Animation pass:** implement the motion moments in §9.3.
7. **M7 — Daemon + notifications:** background binary, systemd unit, reminder firing.
8. **M8 — Packaging:** Flatpak manifest / AppImage / PKGBUILD, install script for systemd unit.
9. **M9 — Polish:** keyboard shortcuts, command palette, settings screen, light theme, empty states, error states (offline, revoked token, keyring locked).

---

## 15. Open Decisions for the Implementer

These are flagged rather than pre-decided, since they're matters of taste/scope:
- Subtasks: Google Tasks supports one level of nesting (`parent` field) — decide how deep to support in the UI (recommend mirroring Google's one-level limit rather than inventing deeper nesting that won't round-trip).
- Multiple Google accounts: v1 should assume a single connected account; multi-account adds real complexity to the keyring/token model and isn't necessary for the stated use case.
- Recurring tasks: Google Tasks API has **no native recurrence support** — if you want recurring tasks, it has to be modeled entirely client-side (TaskFlow generates the next instance locally on completion) and synced as plain one-off tasks. Worth deciding early if this is in scope for v1, since it touches the data model.

---

*End of specification.*
