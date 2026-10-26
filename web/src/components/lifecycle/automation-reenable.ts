import type {
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
