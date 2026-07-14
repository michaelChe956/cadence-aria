#!/usr/bin/env bash
set -euo pipefail

fail() {
  echo "$1" >&2
  exit 1
}

if [[ $# -ne 2 ]]; then
  fail "usage: validate-rollback-state.sh <attempt-json> <units-dir>"
fi

attempt_json="$1"
units_dir="$2"

command -v jq >/dev/null 2>&1 || fail "jq_not_found"
[[ -f "$attempt_json" ]] || fail "attempt_json_missing: $attempt_json"
[[ -d "$units_dir" ]] || fail "units_dir_missing: $units_dir"
jq empty "$attempt_json" >/dev/null || fail "attempt_json_invalid: $attempt_json"

attempt_status="$(jq -r '.status // empty' "$attempt_json")"
attempt_stage="$(jq -r '.stage // empty' "$attempt_json")"
current_work_item_id="$(jq -r '.current_work_item_id // empty' "$attempt_json")"
active_unit_id="$(jq -r '.active_unit_id // empty' "$attempt_json")"

[[ "$attempt_status" == "running" ]] || fail "attempt_not_running: $attempt_status"
[[ "$attempt_stage" == "prepare_context" ]] || fail "attempt_not_prepare_context: $attempt_stage"
[[ -n "$current_work_item_id" ]] || fail "current_work_item_missing"
[[ -n "$active_unit_id" ]] || fail "active_unit_missing"

active_unit_json="$units_dir/$active_unit_id.json"
[[ -f "$active_unit_json" ]] || fail "active_unit_file_missing: $active_unit_json"
jq empty "$active_unit_json" >/dev/null || fail "active_unit_json_invalid: $active_unit_json"

unit_id="$(jq -r '.id // empty' "$active_unit_json")"
unit_work_item_id="$(jq -r '.work_item_id // empty' "$active_unit_json")"
unit_status="$(jq -r '.status // empty' "$active_unit_json")"
unit_started_at="$(jq -r '.started_at // empty' "$active_unit_json")"

[[ "$unit_id" == "$active_unit_id" ]] || fail "active_unit_id_mismatch: $unit_id"
[[ "$unit_work_item_id" == "$current_work_item_id" ]] || fail "active_work_item_mismatch: $unit_work_item_id"
[[ "$unit_status" == "running" ]] || fail "active_unit_not_running: $unit_status"
[[ -n "$unit_started_at" ]] || fail "active_unit_started_at_missing"

jq -e '
  (.completed_at == null)
  and (.handoff_ref == null)
  and (.completion_commit == null)
' "$active_unit_json" >/dev/null || fail "active_unit_completion_fields_not_empty"

shopt -s nullglob
unit_files=("$units_dir"/coding_unit_*.json)
(( ${#unit_files[@]} > 0 )) || fail "unit_files_missing: $units_dir"
active_count="$(jq -s '[.[] | select(.status == "running" or .status == "waiting_for_human" or .status == "blocked")] | length' "${unit_files[@]}")"
[[ "$active_count" == "1" ]] || fail "active_unit_count_invalid: $active_count"

echo "rollback_state_ok"
