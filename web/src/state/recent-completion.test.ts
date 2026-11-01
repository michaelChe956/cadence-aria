import { describe, expect, it } from "vitest";
import type { RecentCompletionInfoItem } from "../api/types";
import {
  INFO_TTL_MS,
  RECENT_COMPLETION_WINDOW_MS,
  isRecentCompletionVisible,
  recentCompletionIdentity,
  recentCompletionInboxItem,
} from "./recent-completion";

// P3 WIGA Task 2（tasks.md §4.1 / REQ-WIGA-07）：近期完成目录的纯前端
// 稳定身份、投影与 TTL 判据。

const planItem: RecentCompletionInfoItem = {
  kind: "plan_confirmed",
  key: "plan_confirmed:plan_1:compile_1",
  project_id: "project_1",
  issue_id: "issue_1",
  plan_id: "plan_1",
  session_id: "session_1",
  attempt_id: null,
  occurred_at: "2026-09-27T00:00:00Z",
  title: "Work Item Plan 已确认",
  final_confirmed: null,
};

const codingItem: RecentCompletionInfoItem = {
  kind: "coding_final_confirm",
  key: "coding_final_confirm:attempt_1:node_1",
  project_id: "project_1",
  issue_id: "issue_1",
  plan_id: "plan_1",
  session_id: null,
  attempt_id: "attempt_1",
  occurred_at: "2026-09-27T03:20:00Z",
  title: "编码执行完成，待最终确认",
  final_confirmed: false,
};

describe("recentCompletionIdentity", () => {
  it("is the JSON of [project_id, issue_id, kind, key]", () => {
    expect(recentCompletionIdentity(planItem)).toBe(
      JSON.stringify([
        "project_1",
        "issue_1",
        "plan_confirmed",
        "plan_confirmed:plan_1:compile_1",
      ]),
    );
  });

  it("keeps equal keys distinct across project/issue/kind and stable otherwise", () => {
    expect(recentCompletionIdentity(planItem)).not.toBe(
      recentCompletionIdentity({ ...planItem, project_id: "project_2" }),
    );
    expect(recentCompletionIdentity(planItem)).not.toBe(
      recentCompletionIdentity({ ...planItem, issue_id: "issue_2" }),
    );
    expect(recentCompletionIdentity(planItem)).not.toBe(
      recentCompletionIdentity({ ...planItem, key: codingItem.key }),
    );
    expect(recentCompletionIdentity(planItem)).toBe(
      recentCompletionIdentity({ ...planItem, title: "另一文案" }),
    );
  });
});

describe("recentCompletionInboxItem", () => {
  it("projects plan entries with the existing plan info copy, drill-down and identity", () => {
    const item = recentCompletionInboxItem(planItem);
    expect(item.kind).toBe("info");
    expect(item.source).toBe("plan_confirmed_info");
    expect(item.title).toBe("Work Item Plan 已确认");
    expect(item.planInfo?.sessionId).toBe("session_1");
    expect(item.planInfo?.planId).toBe("plan_1");
    expect(item.completionIdentity).toBe(recentCompletionIdentity(planItem));
    expect(item.codingInfo ?? null).toBeNull();
  });

  it("projects coding entries with attempt drill-down and final_confirmed state", () => {
    const item = recentCompletionInboxItem(codingItem);
    expect(item.source).toBe("coding_final_confirm_info");
    expect(item.title).toBe("编码执行完成，待最终确认");
    expect(item.codingInfo?.attemptId).toBe("attempt_1");
    expect(item.codingInfo?.finalConfirmed).toBe(false);
    expect(item.completionIdentity).toBe(recentCompletionIdentity(codingItem));
    expect(item.planInfo ?? null).toBeNull();

    // 人工确认后同 key 状态升级：投影文案更新、身份不变（不二次提醒）。
    const confirmed = recentCompletionInboxItem({
      ...codingItem,
      title: "已最终确认",
      final_confirmed: true,
    });
    expect(confirmed.title).toBe("已最终确认");
    expect(confirmed.codingInfo?.finalConfirmed).toBe(true);
    expect(confirmed.completionIdentity).toBe(recentCompletionIdentity(codingItem));
  });
});

describe("isRecentCompletionVisible", () => {
  const now = Date.parse("2026-09-27T12:00:00Z");

  it("shows entries strictly inside the TTL window including age zero", () => {
    expect(isRecentCompletionVisible("2026-09-27T12:00:00Z", now)).toBe(true);
    expect(isRecentCompletionVisible("2026-09-27T11:59:59.999Z", now)).toBe(true);
    expect(
      isRecentCompletionVisible(new Date(now - INFO_TTL_MS + 1).toISOString(), now),
    ).toBe(true);
  });

  it("hides entries at or beyond TTL, future entries, and unparseable times", () => {
    expect(isRecentCompletionVisible(new Date(now - INFO_TTL_MS).toISOString(), now)).toBe(
      false,
    );
    expect(isRecentCompletionVisible(new Date(now - INFO_TTL_MS - 1).toISOString(), now)).toBe(
      false,
    );
    expect(isRecentCompletionVisible("2026-09-27T13:00:00Z", now)).toBe(false);
    expect(isRecentCompletionVisible("not-a-time", now)).toBe(false);
    expect(isRecentCompletionVisible("", now)).toBe(false);
  });

  it("normalizes RFC3339 timezone offsets to the same instant", () => {
    expect(isRecentCompletionVisible("2026-09-27T20:00:00+08:00", now)).toBe(true);
    expect(isRecentCompletionVisible("2026-09-27T04:00:00-08:00", now)).toBe(true);
    // +08:00 写法的过期瞬时同样按 UTC 判定
    const expired = now - INFO_TTL_MS - 1;
    const expiredOffsetWritten = new Date(expired)
      .toISOString()
      .replace("Z", "+00:00");
    expect(isRecentCompletionVisible(expiredOffsetWritten, now)).toBe(false);
  });

  it("keeps the info TTL equal to the 24h recent completion window", () => {
    expect(INFO_TTL_MS).toBe(RECENT_COMPLETION_WINDOW_MS);
    expect(RECENT_COMPLETION_WINDOW_MS).toBe(24 * 60 * 60 * 1000);
  });
});
