export type StageAction =
  | "start_generation"
  | "abort"
  | "accept_author"
  | "reject_author";

export type GateActionFacade = "typed" | "legacy";
export interface StageUIConfig {
  actions: StageAction[];
  headerBadge: string;
  showContextInput: boolean;
  providerEditable: boolean;
}

const STAGE_CONFIG_MAP: Record<string, StageUIConfig> = {
  prepare_context: {
    actions: ["start_generation"],
    headerBadge: "准备中",
    showContextInput: true,
    providerEditable: true,
  },
  running: {
    actions: ["abort"],
    headerBadge: "运行中 · 保持本页打开",
    showContextInput: false,
    providerEditable: false,
  },
  author_confirm: {
    actions: ["accept_author", "reject_author"],
    headerBadge: "Author 待确认",
    showContextInput: false,
    providerEditable: false,
  },
  cross_review: {
    actions: ["abort"],
    headerBadge: "审核中",
    showContextInput: false,
    providerEditable: false,
  },
  // L1 重承载（REQ-RET-02）：legacy 决策动作面（select_revision_path/confirm/
  // request_change/terminate）随旧协议退役收敛为只读呈现；headerBadge 保留——
  // 历史会话 stage 值仍在服务端快照内。
  review_decision: {
    actions: [],
    headerBadge: "审核结论待处理",
    showContextInput: false,
    providerEditable: false,
  },
  revision: {
    actions: ["abort"],
    headerBadge: "修订中",
    showContextInput: false,
    providerEditable: false,
  },
  human_confirm: {
    actions: [],
    headerBadge: "等待确认",
    showContextInput: false,
    providerEditable: false,
  },
  completed: {
    actions: [],
    headerBadge: "已完成",
    showContextInput: false,
    providerEditable: false,
  },
};

export function useStageUI(stage: string): StageUIConfig {
  return STAGE_CONFIG_MAP[stage] ?? STAGE_CONFIG_MAP.prepare_context;
}
