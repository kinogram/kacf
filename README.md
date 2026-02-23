# KACF (Kinogram AutoCoding Framework)

Language: **English (Default)** | [简体中文-母语](README.zh-CN.md)

## STOP MAINTENANCE NOTICE (PERSONAL)

> [!WARNING]
> The `personal` branch is in maintenance-stop state.
> No active feature updates are planned here.
> Use `multi-user` for all new setups, fixes, and ongoing usage.
>
> Migrate now:
> ```bash
> git fetch origin
> git checkout multi-user
> ```
> GitHub users: switch branch selector to `multi-user`.

## English

This README documents the behavior of the **current code in this workspace only**.

### What KACF is

KACF is a Web-UI-driven autonomous coding framework. You provide a goal, and it runs an iterative loop:

`generate patch -> apply patch -> run evaluation -> continue iteration`

Current workspace traits:
- Single binary startup (backend + Web UI)
- Project workflow (create/switch/delete)
- Autosave + resume from checkpoint
- Unattended mode (auto-recovery count, stop time)
- SSE real-time events with polling fallback
- Diff in a dedicated page (not rendered inline in main page)

### Quick start

Requirements:
- Rust stable
- Linux/macOS (Windows requires your own validation)
- Model API key (DeepSeek-compatible by default)

Run:

```bash
cargo run
```

Default bind: `0.0.0.0:8080`

Custom port:

```bash
AUTOCODING_PORT=18080 cargo run
```

Release run:

```bash
cargo build --release
./target/release/kacf
```

### Current UI flow

1. Enter your goal
2. Click "Run Current Project"
3. Check status and logs
4. Resume from checkpoint when needed
5. Tune language / auto-recovery / stop time / log limit in global settings

Notes:
- Project config is primarily autosaved
- Diff opens in a new tab via "View Diff ↗"

### Main endpoints

Pages and static:
- `GET /`
- `GET /diff`
- `GET /assets/app.css`
- `GET /assets/app.js`
- `GET /assets/diff.js`
- `GET /assets/languages/list`
- `GET /assets/languages/{code}.json`

Run control:
- `POST /start`
- `POST /resume`
- `POST /stop`
- `POST /clarify`
- `POST /revert`
- `POST /push`

Project and state:
- `GET /projects`
- `POST /projects`
- `DELETE /projects/{id}`
- `POST /projects/suggest_slug`
- `GET /project_config`
- `GET /ui_state`
- `GET /ui_cache`
- `PUT /ui_cache`
- `GET /diff_data`

Observability:
- `GET /health`
- `GET /metrics`
- `GET /events`
- `GET /events/stream`
- `POST /debug/client_logs`
- `GET /debug/client_logs`

### Directory layout

```text
.
├── src/
├── static/
│   ├── index.html
│   ├── diff.html
│   ├── app.css
│   ├── js/
│   └── languages/
├── scripts/
│   ├── release_check.sh
│   ├── smoke_web.sh
│   └── metrics_gate.sh
├── autocoding_data/
└── LICENSE
```

### Dev checks

```bash
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
```

Release check:

```bash
bash scripts/release_check.sh
```

### Language-pack rules

At startup, `static/languages` is strictly validated:
- Filename format: `KACF_<language_code>_<pack_version>.json`
- Must include `__meta`
- `language_code`, `pack_version`, `inputer_version` must match program requirements
- Key sets across all packs must exactly match `en`

If any rule fails, service startup is rejected.

### License

Custom license in repo: `LICENSE` (KACF Personal & Non-Commercial License 1.1).

Commercial license contact: `gregsons334@gmail.com`
