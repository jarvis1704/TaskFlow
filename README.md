# TaskFlow

A native, lightweight Linux desktop task manager that syncs bidirectionally with Google Tasks.

TaskFlow is designed to be highly responsive, offline-first, and lightweight, utilizing Rust and GPU-accelerated rendering. It bridges the gap between the convenience of the Google Tasks ecosystem and the speed of a native desktop application.

### Note: This is a vibecoded software. The owner of this repository is not responsible for any slop code in this repository. It just serves its function flawlessly, which was exactly what was intented as the end result of the project. 

---

## Features

- **Sleek Desktop GUI:** Built using Rust and `iced` featuring smooth state transitions, animations, dark/light theme support, and custom typography (Inter & JetBrains Mono).
- **Background Daemon:** A tiny, headless background process (`taskflow-daemon`) that manages notifications for upcoming tasks and keeps your tasks synced in the background.
- **Bidirectional Sync:** Fully synchronized with the Google Tasks REST API, resolving conflicts using a last-write-wins strategy based on updated timestamps.
- **Natural Language Parsing:** Quick-add tasks parse dates and times intuitively (e.g., "tomorrow at 5pm", "next Friday").
- **Keyring Security:** Keeps your Google OAuth refresh tokens secure by storing them using the system keyring (via D-Bus Secret Service).
- **SQLite Storage:** Local SQLite database configured in WAL (Write-Ahead Logging) mode, enabling safe, concurrent access by both the GUI and the daemon.

---

## Architecture

TaskFlow is structured as a Cargo workspace with three crates:

- [taskflow-core](file:///home/biprangshu/Work/tasks_linux/crates/taskflow-core) — **Shared Library**: Core logic including models (`Task`, `TaskList`), SQLite migrations, the natural language input parser, Google OAuth/REST API client, and the sync engine.
- [taskflow-gui](file:///home/biprangshu/Work/tasks_linux/crates/taskflow-gui) — **Desktop GUI**: Built with the Elm Architecture (State-Update-View). Handles UI rendering, input palette (`Ctrl+K`), custom animations, and keyboard navigation.
- [taskflow-daemon](file:///home/biprangshu/Work/tasks_linux/crates/taskflow-daemon) — **Background Daemon**: A background service that checks for reminders every 15 seconds (using `notify-rust` for notifications) and syncs tasks with Google Tasks every 5 minutes.

---

## Setup & Installation

### 1. Google OAuth2 API Credentials
Because TaskFlow communicates directly with Google's API, you need your own desktop application credentials:
1. Open the [Google Cloud Console](https://console.cloud.google.com/).
2. Create a new project and enable the **Google Tasks API**.
3. Set up the OAuth consent screen (internal/testing mode is sufficient).
4. Create an **OAuth Client ID** for a **Desktop app**.
5. Save the downloaded credentials as `oauth_client.json` (or `client_secret_*.json`) in the project root.

### 2. Quick Installation
Run the installer script to build, install binaries, register desktop icons, and activate the background daemon service:
```bash
./install.sh
```
This automatically registers the daemon as a systemd user service (`taskflow-daemon.service`) and registers a desktop launcher menu item (`taskflow.desktop`).

---

## Development

Use the following commands from the root directory for development:

- **Build Workspace:** `cargo build --workspace`
- **Run GUI:** `cargo run -p taskflow-gui`
- **Run Daemon:** `cargo run -p taskflow-daemon`
- **Run Tests:** `cargo test --workspace`

---

## Note on Contributions

> [!IMPORTANT]
> **This is personal software.** It is developed and maintained for personal use. Consequently, **no pull requests will be accepted** in this repository. You are, however, welcome to fork the project for your own use.
