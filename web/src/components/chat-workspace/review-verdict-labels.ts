// 评审结论（review_verdict）中文文案的单一事实源：ArtifactPane 头部徽章与
// RevisionDiffView 对比表头/单版本区块共用，避免第二份文案表漂移。
import type { ReviewVerdictType } from "../../api/types";

export const REVIEW_VERDICT_LABELS: Readonly<Record<ReviewVerdictType, string>> = {
  pass: "通过",
  revise: "建议返修",
  needs_human: "需要人工确认",
};
