#!/usr/bin/env bash
# Build/launch with receipt-scoped process control; --verify is fully isolated.
set -euo pipefail
exec python3 "$(dirname "${BASH_SOURCE[0]}")/build_rocket.py" "$@"
