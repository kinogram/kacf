# KACF (Kinogram AutoCoding Framework)

Language: **English (Default)** | [简体中文-母语](README.zh-CN.md)

KACF is an AI coding app for real-world projects.
You describe what you want in plain language, and KACF runs an automatic loop:

`plan -> write code -> test -> fix -> repeat`

Current mode: **Web UI**.

## Choose your branch

- `personal`: single-user, lightweight
- `multi-user`: account system + admin panel (recommended when multiple people use one deployment)

## Quick start (Beginner-friendly)

### 1) Download and run

Run KACF in a terminal (do not double-click the file).

Example:

```bash
cd Downloads
./kacf-multi-user-v1.1.0-linux-amd64
# or
./kacf-multi-user-v1.1.0-linux-arm64
# or
./kacf-multi-user-v1.1.0-windows-amd64.exe
```

You can also run from source code:

```bash
cargo run --release
```

### 2) Open the page

Open this URL in your browser:

`http://localhost:8080`

### 3) First-time setup

- Create the admin account first
- Then users can register/login
- By default, registration only accepts mainstream email providers

### 4) Start building

- Create a project in the UI
- Describe your goal in normal language
- Click run, then let KACF iterate automatically

## Important before running

- Always run KACF in a terminal, not by double-clicking the binary.
- If you already double-clicked it, kill the old process first, or you may get a port conflict.
- At startup, terminal output may appear a bit later; the service can already be listening.
- If the terminal does not exit with an error, open `http://localhost:8080` directly.
- To stop KACF, press `Ctrl-C` in the same terminal.

## What you get in `multi-user`

- Roles: `Admin / User / Guest`
- Guest mode: read-only browsing without registration
- Admin panel: manage users, bans, notices, password reset, audit logs
- Account center: update nickname/password/username/email, switch login method
- Data isolation: per-user `ui_cache/projects/workspaces`

## Troubleshooting (simple)

- Cannot open the page:
  - Confirm the process is still running in terminal
  - Confirm you are visiting `http://localhost:8080`
- "Port already in use":
  - Another process is already using `8080`; stop it, then start KACF again
- Opened by double-click and now broken:
  - Kill old process, then start from terminal again

## For advanced users

Useful scripts still exist in `scripts/` (tests, release checks, metrics gate, VM stress).
If you are new, you can ignore them safely.

## License

Custom license in repo:
- `LICENSE` (KACF Personal & Non-Commercial License 1.1, bilingual)

Summary:
- Free for personal/non-commercial use and non-commercial derivative work
- Non-commercial redistribution allowed with license + attribution
- Any commercial use or commercial derivative work requires written permission

Commercial license contact: `gregsons334@gmail.com`
