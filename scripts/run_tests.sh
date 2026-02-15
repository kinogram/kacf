#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

export RUST_BACKTRACE=1

# Network access may be restricted in some environments.
cargo test --offline

