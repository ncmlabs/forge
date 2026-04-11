#!/usr/bin/env bash
# Fast mock-only validation for checked-in FORGE examples.
set -euo pipefail

export FORGE_MOCK=1
export FORGE_LOG_LEVEL="${FORGE_LOG_LEVEL:-quiet}"

cargo test --test example_validation_tests "$@" -- --show-output
