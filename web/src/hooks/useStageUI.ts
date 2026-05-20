export type StagePanel =
  | "PrepareContextPanel"
  | "RunningPanel"
  | "CrossReviewPanel"
  | "ReviewDecisionPanel"
  | "RevisionPanel"
  | "HumanConfirmPanel"
  | "CompletedPanel";

export type StageAction =
  | "start_generation"
  | "abort"
  | "confirm"
  | "request_change"
  | "terminate"
  | "select_revision_path";

export interface StageUIConfig {
  panel: StagePanel;
  actions: StageAction[];
  headerBadge: string;
  showContextInput: boolean;
  providerEditable: boolean;
}

const STAGE_CONFIG_MAP: Record<string, StageUIConfig> = {
  prepare_context: {
    panel: "PrepareContextPanel",
    actions: ["start_generation"],
    headerBadge: "准备中",
    showContextInput: true,
    providerEditable: true,
  },
  running: {
    panel: "RunningPanel",
    actions: ["abort"],
    headerBadge: "运行中 · 保持本页打开",
    showContextInput: false,
    providerEditable: false,
  },
  cross_review: {
    panel: "CrossReviewPanel",
    actions: ["abort"],
    headerBadge: "审核中",
    showContextInput: false,
    providerEditable: false,
  },
  review_decision: {
    panel: "ReviewDecisionPanel",
    actions: ["select_revision_path", "abort"],
    headerBadge: "审核结论待处理",
    showContextInput: false,
    providerEditable: false,
  },
  revision: {
    panel: "RevisionPanel",
    actions: ["abort"],
    headerBadge: "修订中",
    showContextInput: false,
    providerEditable: false,
  },
  human_confirm: {
    panel: "HumanConfirmPanel",
    actions: ["confirm", "request_change", "terminate"],
    headerBadge: "等待确认",
    showContextInput: false,
    providerEditable: false,
  },
  completed: {
    panel: "CompletedPanel",
    actions: [],
    headerBadge: "已完成",
    showContextInput: false,
    providerEditable: false,
  },
};

export function useStageUI(stage: string): StageUIConfig {
  return STAGE_CONFIG_MAP[stage] ?? STAGE_CONFIG_MAP.prepare_context;
}
