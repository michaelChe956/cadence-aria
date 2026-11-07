import type {
  EnrollmentCarrierTarget,
  EnrollmentOptions,
  EnrollmentSource,
  EnrollmentTarget,
  WorkspaceProviderName,
} from "../../api/types";

/**
 * 重新开启（terminated enrollment 的人工 Enable）时的 provider 选择。
 * 缺省＝沿用终止记录的原授权（表单里的「沿用服务端默认」在重新开启路径上
 * 没有服务端默认可解析——原授权才是唯一权威）。
 */
export type ReenableProviderChoice = {
  author_provider?: WorkspaceProviderName;
  reviewer_provider?: WorkspaceProviderName;
};

/** 终止记录的原授权 payload：重新开启必须逐字段相等（契约硬要求）。 */
export type ReenableOriginalAuthorization = {
  source: EnrollmentSource;
  options: EnrollmentOptions;
  target: EnrollmentTarget;
};

/**
 * 在原授权上套用用户本次选择的 provider——只换 provider，其余授权字段
 * （review_rounds／superpowers／openspec／plan_approval／plan_options）
 * 逐字保留终止记录的值，不被表单里的未生效值改写。
 */
export function reenableOptionsWithProviderChoice(
  original: EnrollmentOptions,
  choice: ReenableProviderChoice,
): EnrollmentOptions {
  return {
    ...original,
    author_provider: choice.author_provider ?? original.author_provider,
    reviewer_provider: choice.reviewer_provider ?? original.reviewer_provider,
  };
}

/** 两个授权的 provider 选择是否一致（是否需要换 payload 的判据）。 */
export function sameProviderChoice(
  left: Pick<EnrollmentOptions, "author_provider" | "reviewer_provider">,
  right: Pick<EnrollmentOptions, "author_provider" | "reviewer_provider">,
): boolean {
  return (
    left.author_provider === right.author_provider &&
    left.reviewer_provider === right.reviewer_provider
  );
}

/**
 * 发起侧载体守卫（REQ-ROUTE-C5-TARGET 前端侧）：把**投影侧**三变体的
 * enrollment target 收敛成**命令侧**两变体（逻辑代码库）。
 *
 * - `logical_codebase` / `logical_codebase_members` → 原样透传；
 * - `single_repository`（**仅 legacy 只读**）→ `null`。
 *
 * `null` 的语义是「这条记录不能作为命令载体」：调用方**禁用提交并给出提示**
 * （需重建为逻辑代码库授权）——MUST NOT 静默降级、MUST NOT 猜一个 LC。
 */
export function carrierTargetFromEnrollmentTarget(
  target: EnrollmentTarget,
): EnrollmentCarrierTarget | null {
  switch (target.kind) {
    case "logical_codebase":
      return {
        kind: "logical_codebase",
        logical_codebase_id: target.logical_codebase_id,
        logical_repository_id: target.logical_repository_id,
      };
    case "logical_codebase_members":
      return {
        kind: "logical_codebase_members",
        logical_codebase_id: target.logical_codebase_id,
        logical_repository_ids: target.logical_repository_ids,
      };
    case "single_repository":
      // legacy 只读载体：不得再作为命令载体。
      return null;
    default:
      return null;
  }
}

/** legacy 单仓载体不可再发起命令时的用户可见提示（唯一文案来源）。 */
export const LEGACY_CARRIER_BLOCKED_MESSAGE =
  "该自动化的授权载体是旧版单仓载体，无法重新开启或提交：需重建为逻辑代码库授权。";
