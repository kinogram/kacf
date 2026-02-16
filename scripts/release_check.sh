#!/usr/bin/env bash
set -euo pipefail

echo "[release-check] 1/7 fmt check"
cargo fmt -- --check

echo "[release-check] 2/7 clippy (core targets)"
cargo clippy --all-targets -- -D warnings

echo "[release-check] 3/7 tests (core targets)"
cargo test --all-targets

echo "[release-check] 4/7 release build"
cargo build --release

echo "[release-check] 5/7 web smoke test"
bash scripts/smoke_web.sh

echo "[release-check] 6/7 metrics release gate"
bash scripts/metrics_gate.sh

echo "[release-check] done"
