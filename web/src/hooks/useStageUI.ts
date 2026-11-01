// C3/REQ-HTR-02 死代码清理：`actions: StageAction[]` 字段与 StageAction 类型删除——
// 全仓 grep 证零渲染消费（两页只用 providerEditable/headerBadge，F53Diag §C）；
// 动作呈现归门卡/输入条单一归属，stage 配置只保留呈现位。
export type GateActionFacade = "typed" | "legacy";
export interface StageUIConfig {
  headerBadge: string;
  showContextInput: boolean;
  providerEditable: boolean;
}

const STAGE_CONFIG_MAP: Record<string, StageUIConfig> = {
  prepare_context: {
    headerBadge: "准备中",
    showContextInput: true,
    providerEditable: true,
  },
  running: {
    headerBadge: "运行中 · 保持本页打开",
    showContextInput: false,
    providerEditable: false,
  },
  author_confirm: {
    headerBadge: "Author 待确认",
    showContextInput: false,
    providerEditable: false,
  },
  cross_review: {
    headerBadge: "审核中",
    showContextInput: false,
    providerEditable: false,
  },
  // L1 重承载（REQ-RET-02）：legacy 决策动作面（select_revision_path/confirm/
  // request_change/terminate）随旧协议退役收敛为只读呈现；headerBadge 保留——
  // 历史会话 stage 值仍在服务端快照内。
  review_decision: {
    headerBadge: "审核结论待处理",
    showContextInput: false,
    providerEditable: false,
  },
  revision: {
    headerBadge: "修订中",
    showContextInput: false,
    providerEditable: false,
  },
  human_confirm: {
    headerBadge: "等待确认",
    showContextInput: false,
    providerEditable: false,
  },
  completed: {
    headerBadge: "已完成",
    showContextInput: false,
    providerEditable: false,
  },
};

export function useStageUI(stage: string): StageUIConfig {
  return STAGE_CONFIG_MAP[stage] ?? STAGE_CONFIG_MAP.prepare_context;
}
