import { describe, expect, it } from "vitest";
import type {
  CodingAttemptStatus,
  CodingExecutionStage,
  PushStatus,
} from "../api/types";
import {
  ATTEMPT_STATUS_LABELS,
  LOCAL_DELIVERY_LABEL,
  PUSH_STATUS_LABELS,
  STAGE_LABELS,
} from "./coding-execution-labels";

// FE-1：三组标签的单一定义点。值域快照显式列出，TS 联合类型新增成员时此数组
// 编译失败（比运行期断言更早暴露漏译）。
const ATTEMPT_STATUSES: CodingAttemptStatus[] = [
  "created",
  "running",
  "waiting_for_human",
  "blocked",
  "awaiting_manual_recovery",
  "awaiting_plan_amendment",
  "applying_plan_amendment",
  "amendment_apply_failed",
  "completed",
  "failed",
  "aborted",
];

const STAGES: CodingExecutionStage[] = [
  "prepare_context",
  "worktree_prepare",
  "coding",
  "code_review",
  "review_request",
  "internal_pr_review",
  "final_confirm",
];

const PUSH_STATUSES: PushStatus[] = [
  "not_pushed",
  "pushed",
  "failed",
  "skipped_not_configured",
];

describe("coding-execution-labels", () => {
  it("为每个 attempt 状态提供中文标签", () => {
    for (const status of ATTEMPT_STATUSES) {
      expect(ATTEMPT_STATUS_LABELS[status]).toBeTruthy();
    }
  });

  it("为每个执行 stage 提供中文标签", () => {
    for (const stage of STAGES) {
      expect(STAGE_LABELS[stage]).toBeTruthy();
    }
  });

  it("为每个推送状态提供中文标签", () => {
    for (const status of PUSH_STATUSES) {
      expect(PUSH_STATUS_LABELS[status]).toBeTruthy();
    }
  });

  it("skipped_not_configured 标为本地交付，不与已推送混淆", () => {
    expect(PUSH_STATUS_LABELS.skipped_not_configured).toBe("本地交付(无远端)");
    expect(PUSH_STATUS_LABELS.skipped_not_configured).toBe(LOCAL_DELIVERY_LABEL);
    expect(PUSH_STATUS_LABELS.pushed).toBe("已推送");
    expect(PUSH_STATUS_LABELS.pushed).not.toBe(LOCAL_DELIVERY_LABEL);
  });
});
