#!/usr/bin/env bash
set -euo pipefail

skill_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
validator="$skill_dir/scripts/validate-rollback-state.sh"

if [[ ! -x "$validator" ]]; then
  echo "validator_missing: $validator" >&2
  exit 127
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

write_attempt() {
  local fixture_dir="$1"
  local current_work_item_id="$2"
  local active_unit_id="$3"
  local stage="${4:-prepare_context}"
  mkdir -p "$fixture_dir/units"
  jq -n \
    --arg current_work_item_id "$current_work_item_id" \
    --arg active_unit_id "$active_unit_id" \
    --arg stage "$stage" \
    '{
      id: "coding_attempt_0001",
      status: "running",
      stage: $stage,
      current_work_item_id: $current_work_item_id,
      active_unit_id: $active_unit_id
    }' > "$fixture_dir/attempt.json"
}

write_unit() {
  local fixture_dir="$1"
  local unit_id="$2"
  local work_item_id="$3"
  local status="$4"
  jq -n \
    --arg unit_id "$unit_id" \
    --arg work_item_id "$work_item_id" \
    --arg status "$status" \
    '{
      id: $unit_id,
      work_item_id: $work_item_id,
      status: $status,
      started_at: "2026-07-13T15:32:18.549045142+00:00",
      completed_at: null,
      handoff_ref: null,
      completion_commit: null
    }' > "$fixture_dir/units/$unit_id.json"
}

expect_pass() {
  local name="$1"
  local fixture_dir="$2"
  local output
  output="$($validator "$fixture_dir/attempt.json" "$fixture_dir/units")"
  if [[ "$output" != "rollback_state_ok" ]]; then
    echo "$name: expected rollback_state_ok, got: $output" >&2
    exit 1
  fi
}

expect_fail() {
  local name="$1"
  local fixture_dir="$2"
  local expected_code="$3"
  local output_file="$tmp_dir/$name.out"
  if "$validator" "$fixture_dir/attempt.json" "$fixture_dir/units" >"$output_file" 2>&1; then
    echo "$name: expected failure $expected_code" >&2
    exit 1
  fi
  if ! grep -Fq "$expected_code" "$output_file"; then
    echo "$name: expected $expected_code, got:" >&2
    cat "$output_file" >&2
    exit 1
  fi
}

valid="$tmp_dir/valid"
write_attempt "$valid" "work_item_0006" "coding_unit_0006"
write_unit "$valid" "coding_unit_0006" "work_item_0006" "running"
expect_pass "valid" "$valid"

pending="$tmp_dir/pending"
write_attempt "$pending" "work_item_0006" "coding_unit_0006"
write_unit "$pending" "coding_unit_0006" "work_item_0006" "pending"
expect_fail "pending" "$pending" "active_unit_not_running"

mismatch="$tmp_dir/mismatch"
write_attempt "$mismatch" "work_item_0006" "coding_unit_0006"
write_unit "$mismatch" "coding_unit_0006" "work_item_9999" "running"
expect_fail "mismatch" "$mismatch" "active_work_item_mismatch"

multiple="$tmp_dir/multiple"
write_attempt "$multiple" "work_item_0006" "coding_unit_0006"
write_unit "$multiple" "coding_unit_0006" "work_item_0006" "running"
write_unit "$multiple" "coding_unit_0007" "work_item_0007" "blocked"
expect_fail "multiple" "$multiple" "active_unit_count_invalid"

echo "rollback_skill_state_tests_ok"
