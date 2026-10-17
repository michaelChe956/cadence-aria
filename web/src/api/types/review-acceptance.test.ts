import { describe, expect, it } from "vitest";
import type { CodeReviewReport, InternalPrReview } from "./coding";
import { REVIEW_REPORT_ACCEPTANCE_VALUES } from "./coding";

// B 档采信投影(T18):CodeReviewReport/InternalPrReview 新增可选
// acceptance/review_window_id,镜像后端
// `src/product/coding_models/review.rs::ReviewReportAcceptance` 的 serde
// snake_case wire。红绿机制:类型断言由 `tsc -b` 编译期裁决(红 = 字段/
// 枚举未定义);runtime 值域由 REVIEW_REPORT_ACCEPTANCE_VALUES 锁定。

type Equal<X, Y> =
  (<T>() => T extends X ? 1 : 2) extends <T>() => T extends Y ? 1 : 2
    ? true
    : false;
type Expect<T extends true> = T;

// wire 值恰好为三成员(serde rename_all="snake_case",无第四成员)。
type _AcceptanceIsThreeValued = Expect<
  Equal<
    NonNullable<CodeReviewReport["acceptance"]>,
    "pending_acceptance" | "accepted" | "invalidated"
  >
>;

// 两字段可选:缺字段 = 历史报告未核验(旧 JSON 不含字段仍可读)。
type _AcceptanceOptional = Expect<
  Equal<CodeReviewReport["acceptance"], ReviewAcceptanceOptional>
>;
type ReviewAcceptanceOptional =
  | "pending_acceptance"
  | "accepted"
  | "invalidated"
  | undefined;
type _WindowIdOptional = Expect<
  Equal<CodeReviewReport["review_window_id"], string | undefined>
>;

// InternalPrReview 同形(公开报告投影两 DTO 同步)。
type _PrAcceptanceOptional = Expect<
  Equal<InternalPrReview["acceptance"], ReviewAcceptanceOptional>
>;
type _PrWindowIdOptional = Expect<
  Equal<InternalPrReview["review_window_id"], string | undefined>
>;

// 历史 wire(缺字段)合法:未核验 ≠ accepted,不默认 B 档 Accepted。
const legacyReport: CodeReviewReport = {
  id: "code_review_report_0001",
  attempt_id: "coding_attempt_0001",
  round: 1,
  verdict: "approve",
  findings: [],
  tested_evidence_refs: [],
  diff_refs: [],
  summary: "legacy",
  created_at: "2026-06-10T00:00:02Z",
};
const legacyPr: InternalPrReview = {
  id: "internal_review_0001",
  attempt_id: "coding_attempt_0001",
  review_request_id: "rr-1",
  verdict: "approve",
  findings: [],
  impact_scope: [],
  pr_description: "",
  commit_message_suggestion: "",
  tested_evidence_refs: [],
  diff_refs: [],
  summary: "legacy",
  created_at: "2026-06-10T00:00:02Z",
};

// 新投影:显式三态 + 窗口绑定。
const adjudicated: CodeReviewReport = {
  ...legacyReport,
  acceptance: "accepted",
  review_window_id: "w-1",
};
const invalidatedPr: InternalPrReview = {
  ...legacyPr,
  acceptance: "invalidated",
  review_window_id: "w-1",
};

describe("coding.ts 报告采信投影(T18)", () => {
  it("wire 值域恰为三成员(serde snake_case)", () => {
    expect([...REVIEW_REPORT_ACCEPTANCE_VALUES]).toEqual([
      "pending_acceptance",
      "accepted",
      "invalidated",
    ]);
  });

  it("缺字段 = 历史未核验(undefined),不默认 Accepted", () => {
    expect(legacyReport.acceptance).toBeUndefined();
    expect(legacyReport.review_window_id).toBeUndefined();
    expect(legacyPr.acceptance).toBeUndefined();
    expect(legacyPr.review_window_id).toBeUndefined();
    // 忠实透传:缺字段的报告绝不因 verdict=approve 被视作 accepted。
    expect(legacyReport.verdict).toBe("approve");
    expect(legacyReport.acceptance).not.toBe("accepted");
  });

  it("显式投影忠实透传不改写", () => {
    expect(adjudicated.acceptance).toBe("accepted");
    expect(adjudicated.review_window_id).toBe("w-1");
    expect(invalidatedPr.acceptance).toBe("invalidated");
    expect(invalidatedPr.review_window_id).toBe("w-1");
    const spread: CodeReviewReport = { ...adjudicated };
    expect(spread.acceptance).toBe("accepted");
    expect(spread.review_window_id).toBe("w-1");
  });
});
