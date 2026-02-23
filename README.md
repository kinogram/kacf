# KACF (Kinogram AutoCoding Framework)

Language: **English (Default)** | [简体中文-母语](README.zh-CN.md)

## English

KACF is an AI autonomous coding framework for real-world development.
You describe the goal, and the system runs a closed loop:

`write code -> run tests/evals -> analyze failures -> fix -> verify`

Current project mode: **Web UI**.

### Why KACF

- Autonomous coding loop: model outputs patch, system applies and advances
- Autonomous evaluation loop: scripted checks prevent “generate-only” workflows
- Unattended execution: auto-iteration, auto-recovery, boundary-safe stop
- Project workflow: per-project save/load/delete with isolated data
- Strong observability: SSE logs + metrics + health + debug logs
- Language pack system with strict startup validation

### Multi-user capabilities (`multi-user` branch)

- Roles: `Admin / User / Guest`
- Guest mode: no registration; globally read-only, backend operations blocked
- Admin panel:
  - create/delete users
  - ban/unban users
  - force notices
  - reset user password (cleartext management by design requirement)
  - view audit logs
- Account center:
  - update nickname/password/username/email
  - switch login methods (password, email verification, etc.)
- Data isolation: per-user `ui_cache/projects/workspaces`
- Session auth: `kacf_session` cookie-based sessions

If you deploy for multiple people, use `multi-user`.

### Branches

- `personal`: single-user, lightweight
- `multi-user`: complete account + admin system

### Quick start

Requirements:
- Rust (stable recommended)
- Linux/macOS (Windows is possible, validate dependencies yourself)
- Available model API key

Build and run:

```bash
cargo build --release
cargo run --release
```

Default URL: `http://localhost:8080`

Basic flow:
1. Open Web UI and enter goal
2. Click "Run Current Project"
3. System iterates automatically (code/eval/fix/retry)
4. Optionally enable unattended, auto-recovery, stop timer

### Key capabilities in detail

#### Self-iteration and unattended mode

- First round can ask for clarification; later rounds iterate by patching
- On failure, auto-recovery can resume according to global policy
- When stop-time is reached, execution stops safely at a round boundary

#### Project and workspace management

- UI is project-centric: create/save/load/delete/switch
- Each project has an isolated workspace to avoid cross-project pollution
- State and config can be restored after browser refresh

#### Observability and runtime stability

- Live logs via SSE with polling fallback
- Runtime metrics (including readiness/gate)
- Frontend offline lock + read-only protections against accidental ops
- UI buffering/truncation strategy to reduce long-session lag

### Directory layout (short)

```text
.
├── src/
├── static/
│   ├── index.html
│   ├── app.css
│   ├── js/
│   └── languages/
├── scripts/
└── autocoding_data/
```

### Common commands

```bash
# tests
bash scripts/run_tests.sh

# release checks
bash scripts/release_check.sh

# metrics gate
bash scripts/metrics_gate.sh

# VM stress/fault-injection (requires kacf_session)
KACF_BASE_URL=http://127.0.0.1:8080 \
KACF_SESSION=<your_session_cookie_value> \
bash scripts/vm_ops_stress.sh --vm <vm_name> --rounds 10 --inject 50 --age 180
```

### License

Custom license in repo:
- `LICENSE` (KACF Personal & Non-Commercial License 1.1, bilingual)

Summary:
- Free for personal/non-commercial use and non-commercial derivative work
- Non-commercial redistribution allowed with license + attribution
- Any commercial use or commercial derivative work requires written permission

Commercial license contact: `gregsons334@gmail.com`


