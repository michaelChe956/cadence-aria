#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
scenario=$(cat "$script_dir/$(basename -- "$0").scenario")

case "$scenario" in
  claude_ok)
    printf '%s\n' 'Claude Code 1.2.3'
    ;;
  codex_ok)
    printf '%s\n' 'codex-cli 4.5.6'
    ;;
  non_zero_exit)
    printf '%s\n' 'authentication failed' >&2
    exit 23
    ;;
  version_unparseable)
    printf '%s\n' 'provider is installed'
    ;;
  timeout)
    while :; do :; done
    ;;
  *)
    printf '%s\n' "unknown scenario: $scenario" >&2
    exit 64
    ;;
esac
