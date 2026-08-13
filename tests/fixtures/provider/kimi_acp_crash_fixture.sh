#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "kimi 0.34.0"
  exit 0
fi

exit "${KIMI_FIXTURE_EXIT_CODE:-1}"
