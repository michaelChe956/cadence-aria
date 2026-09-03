//! 阶段 3 Task 8.4b —— campaign stage3 崩溃/重连恢复矩阵（Step 3 四行合一）。
//!
//! 表驱动骨架：四行（amendment 修订 / turn reservation / takeover / advance
//! journal）共享同一过滤名 `campaign_stage3_recovery_matrix`，统一验证
//! 「WS disconnect + process/store reopen + 同 command 恢复」下的身份/账本
//! 不变量：
//!
//! - **对账以 durable ledger 与 `attempt_no`/turn 终态为准，绝不以 WS
//!   event count 推断**；每行都从磁盘重开读取断言。
//! - **provider alive/dead 两态只在 turn reservation 行区分**（恢复分类
//!   WaitForProvider vs ResumeSameTurn），其余行不引入该区分。
//! - amendment 行为深行：四窗口（context / new revision(出版
//!   `PlanAmendmentPublicationCheckpoint`×7) / binding / application
//!   journal+delivery）逐一崩溃+重启+同 command 恢复，断言同 attempt、
//!   原预算源（原 plan session 门快照，group attempt 无预算账）、binding
//!   前缀不变（直到恰一次应用切换）、resume target 恰一次。
//! - 另三行为薄行：以既有单窗口测试（8.2/8.2a/8.3）同款断言语义做最小
//!   复跑，不重复其深测面（flavor 循环/busy 窗口/五 checkpoint 全扫等）。
//!
//! 🔴 failpoint 键隔离纪律：全部 fixture 经 `campaign_stage3_fixture` /
//! `campaign_amendment_fixture` 系列（会话 scope 进程级原子计数唯一化，
//! TempDir 路径键天然隔离）；advance 行使用本行唯一 command id。禁止裸
//! 字面量 fixture 注册，杜绝全量并发下的键碰撞挂死。

use super::campaign_stage3_advance::{
    confirmed_campaign_harness, next_restarted_outbound, send_via_restarted_worker,
};
use super::campaign_stage3_interactive::{
    CampaignStage3Harness, campaign_candidate_base, campaign_candidate_v2, campaign_stage3_fixture,
};
use super::*;
use crate::product::advance_store::{AdvanceInitializationPhase, AdvanceStore};
use crate::product::coding_attempt_store::{
    CodingAttemptStore, register_plan_amendment_delivery_mark_failpoint,
};
use crate::product::coding_models::{
    CodingAmendmentApplicationPhase, CodingAttemptStatus, CodingExecutionUnitStatus,
    CodingPlanAmendmentDeliveryStatus, PlanAmendmentContextStatus,
};
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::coding_workspace_engine::tests::campaign_stage3_amendment::{
    CampaignAmendmentFixture, campaign_amendment_fixture, confirm_real_amendment_publication,
    open_amendment_turn_and_run_fake_revision, publish_real_amendment, stage_amendment_candidate,
};
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::models::{
    AmendmentResumeMode, HumanGateTurnStatus, PlanAmendmentManifest, PlanAmendmentPublicationPhase,
    PlanRepairRequestStatus, WorkspaceSessionStatus,
};
use crate::product::work_item_plan_policy::{
    HumanGateSnapshot, HumanReason, PolicyDiagnostic, RunPolicy, WorkItemPlanFlowKind,
};
use crate::product::work_item_revision_store::{
    PlanAmendmentPublicationCheckpoint, register_plan_amendment_publication_failpoint,
};
use crate::product::workspace_engine::{
    AdvanceInitializationFailpoint, AdvanceInitializationFailpointMode, AdvanceStatus,
    HumanGateCommandOutcome, HumanGateFeedbackInput, HumanGateRecoveryAction,
    ScManualRevisionResult, register_advance_initialization_failpoint,
};
use crate::web::coding_ws_handler::CodingWsOutMessage;
use crate::web::coding_ws_handler::delivery_ack::{
    confirm_plan_amendment_socket_write, fail_plan_amendment_socket_write,
};
use crate::web::handlers::workspace_session_takeover;
use crate::web::workspace_ws_types::ArtifactPayload;
use std::sync::Mutex as StdMutex;

// 大文件守卫(>1200 行)按 include! 拆分:amendment_row=修订行四窗口深测;
// thin_rows=turn reservation/takeover/advance journal 三条薄行。
include!("campaign_stage3_recovery_matrix/amendment_row.rs");
include!("campaign_stage3_recovery_matrix/thin_rows.rs");
