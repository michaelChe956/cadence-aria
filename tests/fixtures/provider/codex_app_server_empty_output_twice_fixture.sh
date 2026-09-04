#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "codex 0.133.0"
  exit 0
fi

turn_count=0
while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-1},\"result\":{\"userAgent\":\"cadence-aria-test\"}}"
  elif [[ "$line" == *'"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"thread\":{\"id\":\"codex_empty_output_thread\"},\"approvalPolicy\":\"never\"}}"
  elif [[ "$line" == *'"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    turn_count=$((turn_count + 1))
    if [[ "$turn_count" -le 2 ]]; then
      echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"result\":{\"turn\":{\"id\":\"codex_empty_output_turn_${turn_count}\",\"status\":\"inProgress\"}}}"
      echo "{\"jsonrpc\":\"2.0\",\"method\":\"turn/completed\",\"params\":{\"threadId\":\"codex_empty_output_thread\",\"turn\":{\"id\":\"codex_empty_output_turn_${turn_count}\",\"status\":\"completed\"}}}"
      if [[ "$turn_count" -eq 2 ]]; then
        exit 0
      fi
    else
      echo "unexpected turn/start #$turn_count: retry must be bounded to one" >&2
      exit 1
    fi
  fi
done
