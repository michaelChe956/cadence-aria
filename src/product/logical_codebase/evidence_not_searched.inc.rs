// 片8（REQ-COD-05 delta）：跨仓证据「未检索」语义——按钉住世代的成员覆盖
// 分叉。evidence_mediator.rs 余量不足，本片段承接分叉判定、入口级输入校验
// 与跳过支路，由 mediator 以 include! 接线（共享其模块作用域，符号仍归属
// evidence_mediator 模块）。
//
// 分叉事实：钉住世代把目标成员记 `skip_reason`（按 成员 ID + checkout ID
// 匹配）⇒ 跳过支路——不建查询、不探库目录、零 CodeGraph 调用，但仍按既有
// 预算规则 `consume` 核算回包文本（用尽 ⇒ 既有 429）并审计零命中与原因；
// 普通支路把查询范围收缩为「ACL ∩ 该世代已覆盖成员」，被跳过成员的证据
// 不出现在普通目标的结果里。

use crate::product::logical_codebase::failure_copy::{
    MEMBER_SOURCE_LESS_CODE, member_source_less_evidence_copy,
};

/// 查询符号的字符上限——与 `evidence_index` 的 `MAX_QUERY_SYMBOL_CHARS`
/// 等价（该常量私有无法跨模块引用；两处必须同步，普通支路内层
/// `validate_symbol` 保持权威不变）。
const EVIDENCE_QUERY_SYMBOL_CHAR_LIMIT: usize = 512;

/// 入口级查询词校验（片8 M7：上提到分叉之前——跳过目标同样拒绝非法
/// 查询词；错误类型与 message 与 evidence_index::validate_symbol 逐字一致）。
fn validate_query_symbol(symbol: &str) -> Result<(), EvidenceError> {
    if symbol.trim().is_empty() {
        return Err(EvidenceError::InvalidQuery {
            reason: "query symbol must not be empty".to_string(),
        });
    }
    if symbol.chars().count() > EVIDENCE_QUERY_SYMBOL_CHAR_LIMIT {
        return Err(EvidenceError::InvalidQuery {
            reason: format!(
                "query symbol exceeds {EVIDENCE_QUERY_SYMBOL_CHAR_LIMIT} chars (got {})",
                symbol.chars().count()
            ),
        });
    }
    Ok(())
}

/// 目标被钉住世代判定为「已确认无源码、未纳入代码检索」时的事实；与
/// 「检索过但无命中」显式区分（REQ-COD-05）。与卡片/成员行告警同构。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NotSearchedEvidence {
    /// 恒为 `MEMBER_SOURCE_LESS_CODE`（同一事实同一码，与卡片/成员行同源）。
    pub code: String,
    /// 非空中文说明（码表登记的证据面工厂，与回包 `text` 同源）。
    pub message_zh: String,
}

/// 分叉判定：钉住世代按（成员 ID + checkout ID）把目标成员记为跳过。
fn pinned_generation_skips_target(
    snapshot: &AttemptTargetSnapshot,
    pinned: &AggregateIndexRecord,
) -> bool {
    pinned.member_snapshots.iter().any(|member| {
        member.logical_repository_id == snapshot.logical_repository_id
            && member.checkout_id == snapshot.checkout_id
            && member.skip_reason.is_some()
    })
}

/// 普通支路查询范围：ACL 成员 ∩ 钉住世代已覆盖成员（在册且未被记跳过）。
/// 该世代未覆盖的成员（含被跳过成员）不进查询 ⇒ 其证据不出现在结果里。
fn covered_member_dir_names(
    member_dirs: &[(LogicalRepositoryId, String)],
    pinned: &AggregateIndexRecord,
) -> Vec<String> {
    member_dirs
        .iter()
        .filter(|(member_id, _)| {
            pinned.member_snapshots.iter().any(|member| {
                member.logical_repository_id == *member_id && member.skip_reason.is_none()
            })
        })
        .map(|(_, dir_name)| dir_name.clone())
        .collect()
}

/// 目标成员别名（跳过支路说明文案点名用；成员记录已在锚定阶段校验过）。
fn target_member_alias(
    logical: &LogicalCodebaseStore,
    attempt: &CodingExecutionAttempt,
    snapshot: &AttemptTargetSnapshot,
) -> Result<String, EvidenceError> {
    let member = logical
        .load_member(&attempt.project_id, snapshot.logical_repository_id)
        .map_err(map_store_error)?
        .ok_or_else(|| EvidenceError::QueryFailed {
            code: "evidence_acl_target_member_missing",
            message: format!(
                "target member {} has no authority record",
                snapshot.logical_repository_id.0
            ),
        })?;
    Ok(member.alias)
}

/// 跳过支路（片8 REQ-COD-05）：零 CodeGraph 调用；文案取码表证据面工厂
///（text 与 message_zh 同源）；预算照常 `consume`（用尽 ⇒ 既有 429，不因
/// 跳过豁免，不用只读 remaining 冒充）；审计记零命中 + 未检索原因后返回
/// 200（`truncated=false`、`index_stale=false`）。
fn not_searched_response(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    input: &EvidenceQueryInput,
    target_alias: &str,
) -> Result<EvidenceQueryResponse, EvidenceError> {
    let text = member_source_less_evidence_copy(target_alias);
    let result_chars = text.chars().count();
    let budget_remaining = match EvidenceBudgetLedger::new(paths.clone())
        .consume(attempt, &input.query, result_chars)?
    {
        BudgetOutcome::Accepted { remaining } => remaining,
        BudgetOutcome::Exhausted => return Err(EvidenceError::BudgetExhausted),
    };
    append_evidence_audit(
        paths,
        attempt,
        &EvidenceAuditRecord {
            attempt_id: attempt.id.clone(),
            role: evidence_role_label(input.role),
            query: input.query.clone(),
            hit_count: 0,
            result_chars,
            snapshot_refs: Vec::new(),
            budget_remaining,
            timestamp: Utc::now().to_rfc3339(),
            // 片8：未检索原因码（与响应 not_searched.code 同源）。
            not_searched_reason: Some(MEMBER_SOURCE_LESS_CODE.to_string()),
        },
    )?;
    Ok(EvidenceQueryResponse {
        not_searched: Some(NotSearchedEvidence {
            code: MEMBER_SOURCE_LESS_CODE.to_string(),
            message_zh: text.clone(),
        }),
        text,
        truncated: false,
        index_stale: false,
        budget_remaining,
    })
}
