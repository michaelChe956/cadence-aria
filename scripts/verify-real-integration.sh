#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ARIA_REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
TARGET_REPO_ROOT="$ARIA_REPO_ROOT"
TASK_ID=""
INPUT_REPO_ROOT=""

usage() {
  cat <<'EOF'
用法:
  scripts/verify-real-integration.sh [--task-id <task-id>] [--repo-root <repo-root>]

说明:
  只读验证已有任务，不创建任务、不推进状态。
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --task-id)
      if [ "$#" -lt 2 ]; then
        echo "FAIL: 缺少 --task-id 参数值"
        exit 1
      fi
      TASK_ID="$2"
      shift 2
      ;;
    --repo-root)
      if [ "$#" -lt 2 ]; then
        echo "FAIL: 缺少 --repo-root 参数值"
        exit 1
      fi
      INPUT_REPO_ROOT="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "FAIL: 未知参数 $1"
      exit 1
      ;;
  esac
done

if [ -n "$INPUT_REPO_ROOT" ]; then
  if [ ! -d "$INPUT_REPO_ROOT" ]; then
    echo "FAIL: repo root 不存在 $INPUT_REPO_ROOT"
    exit 1
  fi

  TARGET_REPO_ROOT=$(CDPATH= cd -- "$INPUT_REPO_ROOT" && pwd)
fi

TASKS_ROOT="$TARGET_REPO_ROOT/cadence/cache/aria/tasks"

if [ ! -d "$TASKS_ROOT" ]; then
  echo "FAIL: 缺少任务目录 $TASKS_ROOT"
  exit 1
fi

if [ -z "$TASK_ID" ]; then
  TASK_ID=$(find "$TASKS_ROOT" -mindepth 1 -maxdepth 1 -type d -exec basename {} \; | sort | tail -n 1)
fi

if [ -z "$TASK_ID" ]; then
  echo "FAIL: 未找到可检查任务，请使用 --task-id 指定"
  exit 1
fi

TASK_ROOT="$TASKS_ROOT/$TASK_ID"
ARTIFACTS_DIR="$TASK_ROOT/artifacts"
FAILURES=""
MISSING_FILES="|"

append_failure() {
  if [ -z "$FAILURES" ]; then
    FAILURES="$1"
  else
    FAILURES="$FAILURES
$1"
  fi
}

require_file() {
  if [ ! -f "$1" ]; then
    case "$MISSING_FILES" in
      *"|$2|"*) ;;
      *)
        MISSING_FILES="${MISSING_FILES}${2}|"
        append_failure "缺少文件: $2"
        ;;
    esac
  fi
}

require_contains() {
  case "$MISSING_FILES" in
    *"|$2|"*)
      return
      ;;
  esac

  if [ ! -f "$1" ]; then
    require_file "$1" "$2"
    return
  fi

  if ! grep -Fq -- "$3" "$1"; then
    append_failure "文件缺少字段: $2 -> $3"
  fi
}

if [ -f "$ARIA_REPO_ROOT/dist/src/index.js" ]; then
  DOCTOR_OUTPUT=$(node "$ARIA_REPO_ROOT/dist/src/index.js" aria:doctor 2>&1 || true)
  for capability in claude_code codex OpenSpec superpowers; do
    case "$DOCTOR_OUTPUT" in
      *"$capability"*) ;;
      *)
        append_failure "aria:doctor 缺少能力项: $capability"
        ;;
    esac
  done
fi

SPEC_FILE="$ARTIFACTS_DIR/spec-artifact.md"
PLAN_FILE="$ARTIFACTS_DIR/plan-brief.md"
BUNDLE_FILE="$ARTIFACTS_DIR/execution-context-bundle.yaml"
CONTRACT_FILE="$ARTIFACTS_DIR/dispatch-contract-exec-01.yaml"
EXEC_FILE="$ARTIFACTS_DIR/exec-result-exec-01.yaml"
REVIEW_FILE="$ARTIFACTS_DIR/review-report.yaml"
TEST_FILE="$ARTIFACTS_DIR/test-report.yaml"

for file in \
  "$SPEC_FILE:spec-artifact.md" \
  "$PLAN_FILE:plan-brief.md" \
  "$BUNDLE_FILE:execution-context-bundle.yaml" \
  "$CONTRACT_FILE:dispatch-contract-exec-01.yaml" \
  "$EXEC_FILE:exec-result-exec-01.yaml" \
  "$REVIEW_FILE:review-report.yaml" \
  "$TEST_FILE:test-report.yaml"
do
  path_part=${file%%:*}
  name_part=${file#*:}
  require_file "$path_part" "$name_part"
done

require_contains "$SPEC_FILE" "spec-artifact.md" "producer: claude-code"
require_contains "$SPEC_FILE" "spec-artifact.md" "source_capabilities: [OpenSpec, superpowers]"
require_contains "$SPEC_FILE" "spec-artifact.md" "open_spec_evidence: provider=OpenSpec"
require_contains "$SPEC_FILE" "spec-artifact.md" "superpowers_evidence: provider=superpowers"

require_contains "$PLAN_FILE" "plan-brief.md" "producer: claude-code"
require_contains "$PLAN_FILE" "plan-brief.md" "source_capabilities: [OpenSpec, superpowers]"
require_contains "$PLAN_FILE" "plan-brief.md" "open_spec_evidence: provider=OpenSpec"
require_contains "$PLAN_FILE" "plan-brief.md" "superpowers_evidence: provider=superpowers"

require_contains "$BUNDLE_FILE" "execution-context-bundle.yaml" "source_capabilities:"
require_contains "$BUNDLE_FILE" "execution-context-bundle.yaml" "  - OpenSpec"
require_contains "$BUNDLE_FILE" "execution-context-bundle.yaml" "  - superpowers"
require_contains "$BUNDLE_FILE" "execution-context-bundle.yaml" "required_methods:"
require_contains "$BUNDLE_FILE" "execution-context-bundle.yaml" "  - writing-plans"
require_contains "$BUNDLE_FILE" "execution-context-bundle.yaml" "  - test-driven-development"
require_contains "$BUNDLE_FILE" "execution-context-bundle.yaml" "  - verification-before-completion"

require_contains "$CONTRACT_FILE" "dispatch-contract-exec-01.yaml" "worker_cli: codex"
require_contains "$CONTRACT_FILE" "dispatch-contract-exec-01.yaml" "required_methods:"
require_contains "$CONTRACT_FILE" "dispatch-contract-exec-01.yaml" "  - verification-before-completion"

require_contains "$EXEC_FILE" "exec-result-exec-01.yaml" "capabilities_used:"
require_contains "$EXEC_FILE" "exec-result-exec-01.yaml" "  - codex"
require_contains "$EXEC_FILE" "exec-result-exec-01.yaml" "openspec_refs_consumed:"
require_contains "$EXEC_FILE" "exec-result-exec-01.yaml" "  - artifacts/spec-artifact.md"
require_contains "$EXEC_FILE" "exec-result-exec-01.yaml" "superpowers_refs_consumed:"
require_contains "$EXEC_FILE" "exec-result-exec-01.yaml" "  - test-driven-development"
require_contains "$EXEC_FILE" "exec-result-exec-01.yaml" "  - verification-before-completion"

require_contains "$REVIEW_FILE" "review-report.yaml" "producer: claude-code"
require_contains "$REVIEW_FILE" "review-report.yaml" "source_capabilities:"
require_contains "$REVIEW_FILE" "review-report.yaml" "  - OpenSpec"
require_contains "$REVIEW_FILE" "review-report.yaml" "  - superpowers"

require_contains "$TEST_FILE" "test-report.yaml" "producer: claude-code"
require_contains "$TEST_FILE" "test-report.yaml" "source_capabilities:"
require_contains "$TEST_FILE" "test-report.yaml" "  - OpenSpec"
require_contains "$TEST_FILE" "test-report.yaml" "  - superpowers"

if [ -n "$FAILURES" ]; then
  echo "FAIL: 真实接入验证未通过"
  echo "task_id: $TASK_ID"
  printf '%s\n' "$FAILURES"
  exit 1
fi

echo "PASS: 真实接入验证通过"
echo "task_id: $TASK_ID"
echo "checked_artifacts: spec plan bundle contract exec review test"
