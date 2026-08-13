#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "kimi 0.34.0"
  exit 0
fi

while IFS= read -r line; do
  id="$(printf '%s' "$line" | sed -n 's/.*"id"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
  if [[ "$line" == *'"initialize"'* ]]; then
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-1},\"result\":{\"protocolVersion\":1,\"agentCapabilities\":{\"loadSession\":true,\"sessionCapabilities\":{\"resume\":{}}}}}"
  elif [[ "$line" == *'"session/new"'* ]]; then
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"sessionId\":\"auth_fixture\"}}"
  elif [[ "$line" == *'"session/prompt"'* ]]; then
    if [[ "${KIMI_FIXTURE_AUTH_MODE:-acp}" == "stderr" ]]; then
      echo "not logged in; token=fixture-secret-token; config=/tmp/kimi/config.toml" >&2
      exit 1
    fi
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"error\":{\"code\":401,\"message\":\"Unauthorized: token=fixture-secret-token config=/tmp/kimi/config.toml\"}}"
    exit 1
  fi
done
