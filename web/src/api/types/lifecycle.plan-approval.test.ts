import { describe, expect, it } from "vitest";
import type { EnrollmentOptions, PlanApprovalPolicy } from "./lifecycle";

// B2-R3 波 1 类型子提交（REQ-WIGA-02）：lifecycle.ts 镜像后端
// `src/product/models/automation.rs::PlanApprovalPolicy` 的 serde snake_case wire。
// 红绿机制：类型断言由 `tsc -b` 编译期裁决（红＝PlanApprovalPolicy 未定义）。

type Equal<X, Y> =
  (<T>() => T extends X ? 1 : 2) extends <T>() => T extends Y ? 1 : 2
    ? true
    : false;
type Expect<T extends true> = T;

// wire 值恰好为 "manual" | "auto"（serde rename_all="snake_case"，无第三成员）。
type _PolicyIsManualOrAuto = Expect<
  Equal<PlanApprovalPolicy, "manual" | "auto">
>;

// EnrollmentOptions.plan_approval 可选：缺省 manual 对应后端 `#[serde(default)]`，
// 旧 JSON / 旧客户端 payload 不含该字段仍可读、可发送。
type _PlanApprovalIsOptional = Expect<
  Equal<EnrollmentOptions["plan_approval"], PlanApprovalPolicy | undefined>
>;

// 旧 wire（P1 之前持久化的 enrollment / 旧客户端请求）缺 plan_approval 仍合法。
const legacyOptions: EnrollmentOptions = {
  author_provider: "fake",
  reviewer_provider: "fake",
  review_rounds: 0,
  superpowers_enabled: false,
  openspec_enabled: false,
  plan_options: {
    include_integration_tests: true,
    include_e2e_tests: false,
    force_frontend_backend_split: false,
    require_execution_plan_confirm: true,
  },
};

// 新自动化入口显式提交 auto（P1 前端段将引用同一字段）。
const autoOptions: EnrollmentOptions = {
  ...legacyOptions,
  plan_approval: "auto",
};

const wirePolicies = [
  "manual",
  "auto",
] as const satisfies readonly PlanApprovalPolicy[];

describe("lifecycle.ts EnrollmentOptions.plan_approval（B2-R3 波 1）", () => {
  it("镜像后端 PlanApprovalPolicy 的 serde snake_case wire 值", () => {
    expect([...wirePolicies]).toEqual(["manual", "auto"]);
  });

  it("plan_approval 可选：legacy payload 缺省 manual、显式 auto 合法", () => {
    expect(legacyOptions.plan_approval).toBeUndefined();
    expect(autoOptions.plan_approval).toBe("auto");
  });
});
