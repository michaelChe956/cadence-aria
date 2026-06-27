const STAGE_LABELS: Record<string, string> = {
  prepare_context: "准备上下文",
  running: "运行中",
  author_confirm: "等待作者确认",
  cross_review: "审核中",
  review: "审核中",
  review_decision: "等待审核结论处理",
  revision: "返修中",
  human_confirm: "等待人工确认",
  completed: "已完成",
  work_item_plan_outline_confirm: "等待 Outline 确认",
  work_item_generation_mode: "选择 Work Item 生成模式",
  work_item_draft_confirm: "等待 Draft 确认",
  work_item_batch_confirm: "等待 Batch 确认",
};

export function workspaceStageLabel(stage: string) {
  return STAGE_LABELS[stage] ?? stage;
}

export function stageChangeContent(stage: string) {
  return workspaceStageLabel(stage);
}
