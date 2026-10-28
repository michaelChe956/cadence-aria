// 执行流投影（CockpitFlowState/CockpitFlowRow/cockpitFlowState/flowElapsedMs/
// formatFlowElapsed/topologyTokenName）拆至 workspace-cockpit-flow.ts；此处
// re-export 维持本模块公共 API 不变（large_file_guard 1200 行上限，纯移动）。
export {
  type CockpitFlowState,
  type CockpitFlowRow,
  cockpitFlowState,
  flowElapsedMs,
  formatFlowElapsed,
  topologyTokenName,
} from "./workspace-cockpit-flow";

// 门投影 / 收件箱类型 / 收件箱条目 / 选择器拆至同目录分片（large_file_guard
// 1200 行上限，纯移动零行为变化）；此处 re-export 维持本模块公共 API 不变。
export type {
  GateActionBlockReason,
  GateKind,
  GateKindState,
  GateProjection,
} from "./workspace-cockpit-gate-projection";
export {
  GATE_TRIGGER_LABELS,
  isAdvanceReplayCode,
  terminalGateClosure,
  gateKindOf,
  isStoryDesignAuthorConfirm,
  gateActionBlockReason,
  gateTerminateBlockReason,
  gateActionBlockCopy,
  isGateTriage,
  selectGateProjection,
} from "./workspace-cockpit-gate-projection";

export type {
  CockpitInboxKind,
  ChoiceInboxProjection,
  PlanConfirmedInfoProjection,
  CodingFinalConfirmInfoProjection,
  CockpitInboxItem,
  TrustMissingProjection,
  LogicalCodebaseBootstrapInfoProjection,
  LcResumeRecoveryInboxProjection,
  WaitingItemActionProjection,
  C1WaitingProjection,
  RepositoryInitFailureDiagnosticsProjection,
  CodingAttentionInboxProjection,
} from "./workspace-cockpit-inbox-types";

export type { ClassifiedC1WaitingItem } from "./workspace-cockpit-inbox-items";
export {
  c1TargetLabel,
  c1WaitingItem,
  logicalCodebaseBootstrapItem,
  isStaleDriverLeaseItem,
  planConfirmedInfoItem,
  codingFinalConfirmInfoItem,
  codingAttentionIdentity,
  canonicalCodingChoiceIdentity,
  inboxCodingChoiceIdentity,
  dedupeCodingChoiceInboxItems,
  codingAttentionItem,
  cockpitInboxItemSessionId,
} from "./workspace-cockpit-inbox-items";

export {
  selectCockpitInbox,
  selectCockpitFlow,
} from "./workspace-cockpit-projection-selectors";

// 裸 driver 抢走租约后的协议码——常量源头在 protocol-error-copy（F-50）。
export { STALE_DRIVER_LEASE_CODE } from "./protocol-error-copy";
