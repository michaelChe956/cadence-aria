// ---------------------------------------------------------------------------
// R5:冷启动自动推进——drift 等待面不需要人工点击(owner 2026-10-26
// 「冷启动的触发也需要修好」)。
//
// 三条铁律:
// 1. 有界:同一「源指纹」只自动尝试一次,失败不自动重试(退化为
//    WaitingForHuman，可读原因落 durable attempt 记录)。
// 2. 幂等:attempt 记录以源指纹为键;同指纹不再调用发布。
// 3. fail-soft:自动推进失败只 warn，绝不让调用方的其它步骤/LC/issue 失败。
//
// 可观测:每次自动尝试都落 durable attempt(触发点/旧新 revision/指纹)并打
// tracing 事件，排障可查。
// ---------------------------------------------------------------------------

/// 自动推进的触发点标识(写入 attempt 记录与日志)。
pub(crate) const AUTO_REPUBLISH_TRIGGER_BOOTSTRAP_ACTION: &str = "bootstrap_action";
pub(crate) const AUTO_REPUBLISH_TRIGGER_PREPARE: &str = "prepare_admission";

/// 触发自动重新发布的 drift reason_code 集合(与 R2.4 的三个等待面一致)。
const AUTO_REPUBLISH_DRIFT_REASONS: [&str; 3] = [
    "rule_digest_drift",
    "policy_digest_drift",
    "policy_reference_digest_drift",
];

/// per-LC scope root 下的自动尝试记录文件名。
const POLICY_REPUBLISH_ATTEMPTS_FILE: &str = "aggregate-policy-republish-attempts.json";
/// attempt 记录保留条数上限(只留最近若干条，够排障即可)。
const POLICY_REPUBLISH_ATTEMPTS_KEEP: usize = 20;

/// 一次自动重新发布的 durable 尝试事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PolicyRepublishAttempt {
    /// 触发时的源指纹(8 个源的 digest 序列摘要) = 幂等键。
    pub(crate) source_fingerprint: String,
    pub(crate) trigger: String,
    pub(crate) outcome: String,
    pub(crate) republication_id: String,
    pub(crate) prior_revision: u64,
    pub(crate) revision: u64,
    pub(crate) detail: String,
    pub(crate) attempted_at: String,
}

/// attempt 记录容器(单文件覆盖式 durable 写)。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PolicyRepublishAttempts {
    #[serde(default)]
    pub(crate) attempts: Vec<PolicyRepublishAttempt>,
}

impl PolicyRepublishAttempts {
    /// 同指纹是否已尝试过(成功或失败都算——失败不得自动重试)。
    fn attempted(&self, fingerprint: &str) -> bool {
        self.attempts
            .iter()
            .any(|attempt| attempt.source_fingerprint == fingerprint)
    }

    fn push(&mut self, attempt: PolicyRepublishAttempt) {
        self.attempts.push(attempt);
        let excess = self.attempts.len().saturating_sub(POLICY_REPUBLISH_ATTEMPTS_KEEP);
        if excess > 0 {
            self.attempts.drain(..excess);
        }
    }
}

/// 自动推进的结果(fail-soft:调用方只看是否等待，不把它当错误)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AutoRepublishOutcome {
    /// 该步不处于 drift 等待，无需动作。
    NoDrift,
    /// 同源指纹已尝试过:保持 WaitingForHuman，不再自动重试。
    AlreadyAttempted { fingerprint: String },
    /// 自动重新发布成功(新 revision 已提交)。
    Published {
        fingerprint: String,
        prior_revision: u64,
        revision: u64,
    },
    /// 自动重新发布失败:保持 WaitingForHuman，原因已落 attempt 记录。
    Failed { fingerprint: String, detail: String },
}

impl AutoRepublishOutcome {
    /// 自动推进后该步仍停留在等待面。
    pub(crate) fn is_waiting(&self) -> bool {
        matches!(
            self,
            Self::AlreadyAttempted { .. } | Self::Failed { .. }
        )
    }
}

/// 源指纹:8 个源的相对路径 + digest 序列的 canonical SHA-256。
fn policy_source_fingerprint(sources: &[PolicySource]) -> String {
    let mut canonical = String::new();
    for source in sources {
        canonical.push_str(&source.relative_path);
        canonical.push('\0');
        canonical.push_str(&sha256_hex(&source.bytes));
        canonical.push('\n');
    }
    sha256_hex(canonical.as_bytes())
}

impl AggregatePolicyArtifactStore {
    fn republish_attempts_path(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        Ok(self
            .scope_root(project_id)?
            .join(POLICY_REPUBLISH_ATTEMPTS_FILE))
    }

    /// 读取自动尝试记录(纯读，缺失视为空)。
    pub(crate) fn republish_attempts(
        &self,
        project_id: &str,
    ) -> Result<PolicyRepublishAttempts, ProductStoreError> {
        let path = self.republish_attempts_path(project_id)?;
        if !path.try_exists().map_err(|error| {
            ProductStoreError::Io(format!("try_exists {}: {error}", path.display()))
        })? {
            return Ok(PolicyRepublishAttempts::default());
        }
        read_json(&path)
    }

    fn record_republish_attempt(
        &self,
        project_id: &str,
        attempt: PolicyRepublishAttempt,
    ) -> Result<(), ProductStoreError> {
        let mut attempts = self.republish_attempts(project_id)?;
        attempts.push(attempt);
        write_json_durable(&self.republish_attempts_path(project_id)?, &attempts)
    }
}

/// R5 主入口:RulesPolicy 处于 drift 等待时，用当前源自动重新发布一次。
///
/// `Ok` 只表示「自动推进没有把调用方拖垮」;是否仍在等待由
/// [`AutoRepublishOutcome::is_waiting`] 判定。调用方必须把它当 fail-soft
/// (推荐直接用 [`auto_republish_best_effort`])。
pub(crate) fn auto_republish_on_policy_drift(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
    trigger: &str,
) -> Result<AutoRepublishOutcome, ProductStoreError> {
    let projection =
        crate::product::logical_codebase::LogicalCodebaseBootstrapProjector::new(paths.clone())
            .project(project_id, logical_codebase_id)?;
    let rules = projection
        .steps
        .iter()
        .find(|step| step.step == crate::product::logical_codebase::LogicalCodebaseBootstrapStep::RulesPolicy);
    let Some(failure) = rules.and_then(|step| step.failure.as_ref()) else {
        return Ok(AutoRepublishOutcome::NoDrift);
    };
    if !AUTO_REPUBLISH_DRIFT_REASONS.contains(&failure.reason_code.as_str()) {
        return Ok(AutoRepublishOutcome::NoDrift);
    }

    // 幂等键 = 当前源指纹(同一批源字节只尝试一次)。
    let sources = collect_policy_sources(&projection.authority_root)?;
    let fingerprint = policy_source_fingerprint(&sources);
    let store =
        AggregatePolicyArtifactStore::for_lc(paths.clone(), logical_codebase_id.to_string());
    let attempts = store.republish_attempts(project_id)?;
    if attempts.attempted(&fingerprint) {
        return Ok(AutoRepublishOutcome::AlreadyAttempted { fingerprint });
    }

    let manifest = crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
        paths.clone(),
        logical_codebase_id.to_string(),
    )
    .load_manifest(project_id)?
    .ok_or_else(|| ProductStoreError::NotFound {
        kind: "logical_codebase_manifest",
        id: logical_codebase_id.to_string(),
    })?;
    let prior_revision = store
        .get(project_id)?
        .map(|artifact| artifact.revision)
        .unwrap_or(0);
    let attempted_at = chrono::Utc::now().to_rfc3339();
    let command_id = format!("auto-republish-{fingerprint}");
    let outcome = store.republish_policy(
        &manifest,
        &projection.authority_root,
        &command_id,
        attempted_at.clone(),
    );
    let (attempt, result) = match outcome {
        Ok((record, _)) => (
            PolicyRepublishAttempt {
                source_fingerprint: fingerprint.clone(),
                trigger: trigger.to_string(),
                outcome: "published".to_string(),
                republication_id: record.republication_id.clone(),
                prior_revision: record.prior_revision,
                revision: record.revision,
                detail: format!(
                    "自动重新发布成功:revision {} -> {},rule digest {}",
                    record.prior_revision, record.revision, record.rule_digest
                ),
                attempted_at,
            },
            AutoRepublishOutcome::Published {
                fingerprint: fingerprint.clone(),
                prior_revision: record.prior_revision,
                revision: record.revision,
            },
        ),
        Err(error) => (
            PolicyRepublishAttempt {
                source_fingerprint: fingerprint.clone(),
                trigger: trigger.to_string(),
                outcome: "failed".to_string(),
                republication_id: String::new(),
                prior_revision,
                revision: prior_revision,
                detail: error.to_string(),
                attempted_at,
            },
            AutoRepublishOutcome::Failed {
                fingerprint: fingerprint.clone(),
                detail: error.to_string(),
            },
        ),
    };
    tracing::info!(
        project_id,
        logical_codebase_id,
        trigger,
        outcome = %attempt.outcome,
        fingerprint = %fingerprint,
        prior_revision = attempt.prior_revision,
        revision = attempt.revision,
        "automatic policy republish attempt recorded"
    );
    store.record_republish_attempt(project_id, attempt)?;
    Ok(result)
}

// 递归保护:自动重发内部会重新投影(bootstrap)，投影路径不得再触发重发。
thread_local! {
    static AUTO_REPUBLISH_IN_FLIGHT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// fail-soft 包装:自动推进失败只 warn，绝不让调用方的流程失败(R5.3)。
pub(crate) fn auto_republish_best_effort(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
    trigger: &str,
) {
    if AUTO_REPUBLISH_IN_FLIGHT.with(|flag| flag.replace(true)) {
        return;
    }
    let result = auto_republish_on_policy_drift(paths, project_id, logical_codebase_id, trigger);
    AUTO_REPUBLISH_IN_FLIGHT.with(|flag| flag.set(false));
    match result {
        Ok(outcome) => {
            if outcome.is_waiting() {
                tracing::warn!(
                    project_id,
                    logical_codebase_id,
                    trigger,
                    ?outcome,
                    "policy drift remains: automatic republish did not converge, waiting for human"
                );
            }
        }
        Err(error) => tracing::warn!(
            project_id,
            logical_codebase_id,
            trigger,
            %error,
            "automatic policy republish skipped by an unexpected error"
        ),
    }
}
