//! r54 越界根修(Ruling 11 A 面 prompt 纪律,oracle 定性 2026-10-08)红绿测。
//!
//! r54 现场(WHO=coder,提交 c2be7db/58bbbc8 均落在 coder role run 窗口):
//! LC attempt 的 coder/reviewer 会话 spawn cwd=authority root(聚合根,
//! REQ-ENV-10 冻结),组路径渲染 envelope 又不携带「Worktree Path」行——
//! coder 依计划验证命令「cd alpha」落入成员主 checkout 提交+构建,触发
//! cross_target_violation(r53 同环境 AI 自行导航正确=方差,Ruling 11 系统性
//! 消除)。本组测试钉住:LC attempt(target_snapshot 在场,与 cross_target_check
//! 同口径)的 coder fresh/rework 与 reviewer 实际下发 prompt 必须携带
//! worktree 纪律段(绝对路径唯一工作区+成员主 checkout 写禁令+交付门整轮
//! 作废),且绑定渲染 envelope 前缀保持不变(纪律段只追加,不动冻结哈希面)。

use super::provider_execution_context::{current_plan_defect_output, review_plan_defect_output};
use super::*;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::coding_models::AttemptTargetSnapshot;
use crate::product::logical_codebase::{
    GatewayRunAudit, LogicalRepositoryId, RepositoryCheckoutId,
};
use crate::product::work_item_projection::{
    CoderExecutionEnvelope, ReviewerExecutionEnvelope, renderer_for,
};
// Task 7 既有 LC gateway 装置复用(可见性 pub(super)):
use super::provider_gateway_validated_input::{
    build_gateway_with_registry, engine as gateway_engine, seed_logical_codebase_checkout,
};
use std::sync::{Arc, Mutex};

/// 捕获 prompt 的 LC validated provider:镜像真实 LC adapter 契约
/// (`start_validated` 拆出 input)。LC 分发按 provider 名走 gateway registry
/// (同测内 coder/reviewer 同为 ClaudeCode),故单一 adapter 按「调用序」弹出
/// 预置输出(Plain/Sentinel 两形态,sentinel 按结构化契约 nonce 包装,对齐
/// provider_execution_context::CapturingProjectionProvider)。
struct CapturingValidatedAdapter {
    outputs: Mutex<std::collections::VecDeque<CapturedOutput>>,
    inputs: Mutex<Vec<StreamingProviderInput>>,
}

enum CapturedOutput {
    Plain(String),
    Sentinel(String),
}

impl CapturingValidatedAdapter {
    fn new(outputs: Vec<CapturedOutput>) -> Arc<Self> {
        Arc::new(Self {
            outputs: Mutex::new(outputs.into()),
            inputs: Mutex::new(Vec::new()),
        })
    }

    fn prompt_of_invocation(&self, invocation: usize) -> String {
        self.inputs
            .lock()
            .unwrap()
            .get(invocation)
            .unwrap_or_else(|| panic!("provider invocation {invocation} captured"))
            .prompt
            .clone()
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CapturingValidatedAdapter {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let structured_output_contract = input.structured_output_contract.clone();
        let nonce = structured_output_contract
            .as_ref()
            .map(|contract| contract.nonce.clone());
        self.inputs.lock().unwrap().push(input);
        let next = self
            .outputs
            .lock()
            .unwrap()
            .pop_front()
            .expect("capturing adapter output queue exhausted");
        let output = match next {
            CapturedOutput::Plain(text) => text,
            CapturedOutput::Sentinel(text) => {
                let nonce = nonce.expect("sentinel payload requires structured output contract");
                let (receipt, payload) = text
                    .split_once('\n')
                    .expect("sentinel payload requires receipt and JSON payload");
                let mut payload: serde_json::Value =
                    serde_json::from_str(payload).expect("sentinel payload must be JSON");
                payload
                    .as_object_mut()
                    .expect("sentinel payload must be an object")
                    .insert("nonce".to_string(), serde_json::json!(nonce));
                format!(
                    "{receipt}\n<ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">{payload}</ARIA_STRUCTURED_OUTPUT>"
                )
            }
        };
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::from_output(
                        output,
                        structured_output_contract.as_ref(),
                        None,
                    ),
                ))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }
}

struct LcGroupFixture {
    root: tempfile::TempDir,
    worktree: std::path::PathBuf,
    head: String,
    store: CodingAttemptStore,
    attempt: CodingExecutionAttempt,
}

/// LC 组 attempt 装置:组单元运行 fixture + target_snapshot 在场(LC 口径)
/// + LogicalCodebaseStore 成员主 checkout 播种(cross_target baseline 采集
/// 可过)+ bootstrap 政策 gateway。coder/reviewer provider 均为 ClaudeCode
/// (避开 Codex danger-full-access 路由级硬门)。
fn lc_group_running_attempt() -> LcGroupFixture {
    let root = tempdir().unwrap();
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).unwrap();
    init_test_git_repo(&worktree);
    let head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: head.clone(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        })
        .unwrap();
    seed_group_attempt_fixture(&store, &attempt, true, false);
    let mut attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    attempt.head_commit = Some(head.clone());
    attempt.stage = CodingExecutionStage::Coding;
    attempt.target_snapshot = Some(AttemptTargetSnapshot {
        logical_repository_id: LogicalRepositoryId(uuid::Uuid::nil()),
        checkout_id: RepositoryCheckoutId(uuid::Uuid::nil()),
        physical_repository_id: "repository_0001".to_string(),
        canonical_path: worktree.clone(),
        git_dir_identity: "git-dir-identity".to_string(),
        revision: None,
        policy_digest: String::new(),
        membership_revision: 1,
        captured_at: "2026-10-08T00:00:00Z".to_string(),
        capture_source: "r54-worktree-discipline".to_string(),
    });
    store.write_coding_attempt_for_test(&attempt).unwrap();
    seed_logical_codebase_checkout(&store, &attempt);
    LcGroupFixture {
        root,
        worktree,
        head,
        store,
        attempt,
    }
}

fn lc_engine_with_adapter(
    fx: &LcGroupFixture,
    adapter: &Arc<CapturingValidatedAdapter>,
) -> CodingWorkspaceEngine {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, adapter.clone());
    let gateway = build_gateway_with_registry(
        &fx.store.paths(),
        &fx.attempt.project_id,
        Arc::new(registry),
        Arc::new(GatewayRunAudit::new()),
    );
    gateway_engine(&fx.store, Some(gateway))
}

fn assert_worktree_discipline(prompt: &str, worktree: &std::path::Path) {
    assert!(
        prompt.contains("[worktree_discipline]"),
        "r54 越界根修:LC prompt 必须携带 worktree 纪律段:\n{}",
        prompt
    );
    assert!(
        prompt.contains(&worktree.display().to_string()),
        "纪律段必须钉隔离 worktree 绝对路径:\n{}",
        prompt
    );
    assert!(
        prompt.contains("成员主 checkout"),
        "纪律段必须携带成员主 checkout 负边界禁线:\n{}",
        prompt
    );
    assert!(
        prompt.contains("cross_target_violation"),
        "纪律段必须声明交付门整轮作废后果:\n{}",
        prompt
    );
    // F3(r58 深掏审计):push 纪律句必须随纪律段注入 coder/reviewer 全族
    // 实际 prompt——交付 push 的权威执行面是产品进程(codex/kimi 沙箱内
    // bare origin 不可达,coder 自 push 失败会自判任务失败/重试空转)。
    assert!(
        prompt.contains("git push 由平台执行"),
        "纪律段必须声明 git push 由平台执行:\n{}",
        prompt
    );
    assert!(
        prompt.contains("禁止手动执行 git push"),
        "纪律段必须携带手动 git push 禁令:\n{}",
        prompt
    );
}

#[tokio::test]
async fn lc_group_coder_prompt_carries_worktree_discipline() {
    let fx = lc_group_running_attempt();
    let adapter =
        CapturingValidatedAdapter::new(vec![CapturedOutput::Plain(current_plan_defect_output())]);
    let engine = lc_engine_with_adapter(&fx, &adapter);

    let coded = engine
        .execute_coding(
            &fx.attempt,
            adapter.as_ref(),
            &CodingExecutionContext::default(),
        )
        .await
        .unwrap();

    let prompt = adapter.prompt_of_invocation(0);
    // 绑定渲染 envelope 前缀保持不变(纪律段只追加,冻结哈希面不动)。
    let run = fx.store.get_active_unit_run(&coded).unwrap();
    let revision_store = WorkItemRevisionStore::new(fx.store.paths());
    let lineage = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &run.projection_bundle_id)
        .unwrap();
    let expected_coder = renderer_for(&ProviderName::ClaudeCode)
        .render_coder(
            &bundle.coder_projection,
            &CoderExecutionEnvelope {
                repository_state_ref: fx.head.clone(),
                resolved_handoff_revision_ids: Vec::new(),
                unit_run_id: run.id.clone(),
                previous_actionable_review: None,
                start_commit: Some(fx.head.clone()),
            },
        )
        .unwrap();
    assert!(
        prompt.starts_with(&expected_coder.text),
        "绑定渲染 envelope 必须保持前缀"
    );
    assert_worktree_discipline(&prompt, &fx.worktree);
}

#[tokio::test]
async fn lc_group_reviewer_prompt_carries_worktree_discipline() {
    let fx = lc_group_running_attempt();
    let adapter = CapturingValidatedAdapter::new(vec![
        CapturedOutput::Plain(current_plan_defect_output()),
        CapturedOutput::Plain(review_plan_defect_output()),
    ]);
    let engine = lc_engine_with_adapter(&fx, &adapter);
    let coded = engine
        .execute_coding(
            &fx.attempt,
            adapter.as_ref(),
            &CodingExecutionContext::default(),
        )
        .await
        .unwrap();

    fs::write(fx.worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").unwrap();
    // LC 分发按 provider 名走 gateway registry(单 gateway bootstrap 非幂等,
    // 同 engine 顺序复用:第 2 次调用即 reviewer 角色)。
    let report = engine
        .execute_code_review(&coded, adapter.as_ref())
        .await
        .unwrap();
    assert_eq!(report.verdict, ReviewVerdict::Blocked);

    let prompt = adapter.prompt_of_invocation(1);
    let run = fx.store.get_active_unit_run(&coded).unwrap();
    let revision_store = WorkItemRevisionStore::new(fx.store.paths());
    let lineage = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &run.projection_bundle_id)
        .unwrap();
    let expected_reviewer = renderer_for(&ProviderName::ClaudeCode)
        .render_reviewer(
            &bundle.reviewer_projection,
            &ReviewerExecutionEnvelope {
                unit_run_id: run.id.clone(),
                diff_ref: format!("{}..worktree", fx.head),
                handoff_revision_ids: Vec::new(),
                contract_delta_refs: Vec::new(),
                completion_commit: fx.head.clone(),
            },
        )
        .unwrap();
    assert!(
        prompt.starts_with(&expected_reviewer.text),
        "绑定渲染 envelope 必须保持前缀"
    );
    assert!(
        prompt.ends_with("最终结论的 JSON 必须是合法对象。\n"),
        "终端结构化契约必须保持收尾"
    );
    assert_worktree_discipline(&prompt, &fx.worktree);
}

#[tokio::test]
async fn lc_group_rework_coder_prompt_carries_worktree_discipline() {
    let fx = lc_group_running_attempt();
    let adapter = CapturingValidatedAdapter::new(vec![
        CapturedOutput::Plain(current_plan_defect_output()),
        CapturedOutput::Plain("coder fixed reviewer findings".to_string()),
    ]);
    let engine = lc_engine_with_adapter(&fx, &adapter);
    let coded = engine
        .execute_coding(
            &fx.attempt,
            adapter.as_ref(),
            &CodingExecutionContext::default(),
        )
        .await
        .unwrap();
    let coded = fx
        .store
        .update_attempt_stage(
            &coded.project_id,
            &coded.issue_id,
            &coded.id,
            CodingExecutionStage::CodeReview,
        )
        .unwrap();
    // 同 reviewer 测:同 engine 顺序复用,第 2 次调用即 rework coder 角色。
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    engine
        .execute_coder_fix_from_review(
            &coded,
            &super::provider_driven::review_report_requesting_changes(&coded),
            &CodingExecutionContext::default(),
            adapter.as_ref(),
            &mut command_rx,
        )
        .await
        .unwrap();

    let prompt = adapter.prompt_of_invocation(1);
    assert!(prompt.contains("reviewer requested changes"));
    assert_worktree_discipline(&prompt, &fx.worktree);
}
