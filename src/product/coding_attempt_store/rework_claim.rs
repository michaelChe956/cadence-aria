//! C2 Task 7（REQ-GCE-C2-INSTR，#18／BYPASS-18）：返修指令单次消费事务。
//!
//! 两条 Coder 路径（首次 spawn 与 rework）统一消费口径：先以待消费的
//! rework instruction／context note 渲染完整 prompt 与执行上下文（全文
//! 非摘要），再一次可重放原子写入（attempt-scoped journal＋幂等消费
//! 标记）完成「认领 → 绑定渲染 digest／上下文 hash → 标记消费」，最后
//! spawn。渲染失败（含空渲染）禁消费；同一指令至多被一次 role run 消费；
//! 中断后重放同一次认领命中同一结果（`Replayed`），已记录的执行上下文
//! hash 不被覆盖为新含义。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

use super::locking::with_exclusive_lock;

/// C2 Task 7：返修指令认领记录（attempt-scoped journal 条目）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReworkInstructionClaim {
    pub claim_id: String,
    pub attempt_id: String,
    /// 被本次认领消费的 rework instruction／context note id（排序去重）。
    pub instruction_ids: Vec<String>,
    /// 实际发送 prompt 全文的 SHA-256（指令全文进入渲染结果）。
    pub rendered_prompt_digest: String,
    /// 绑定的执行上下文 hash（unit-run 渲染上下文 hash；无渲染上下文时
    /// 以 prompt digest 兜底，保证与实际 prompt 一致）。
    pub context_hash: String,
    pub consumed_at: Option<String>,
}

/// C2 Task 7：认领事务结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReworkClaimOutcome {
    /// 首次认领：渲染结果已绑定，指令已标记消费。
    Claimed { claim: ReworkInstructionClaim },
    /// 中断后重放：同一认领同一渲染结果命中同一 claim，不生成新 hash、
    /// 不二次消费（幂等补齐消费标记）。
    Replayed { claim: ReworkInstructionClaim },
    /// 渲染失败（含空渲染）：禁消费。
    RenderFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct ReworkClaimJournal {
    #[serde(default)]
    records: Vec<ReworkInstructionClaim>,
}

/// prompt 全文 SHA-256（渲染 digest 单源，供 engine 与测试共享）。
pub(crate) fn rework_prompt_digest(prompt: &str) -> String {
    hex::encode(Sha256::digest(prompt.as_bytes()))
}

/// 认领 id 由 attempt 与消费集合决定性派生：中断后重放同一集合命中同一
/// 认领，而不是生成第二条。
fn deterministic_claim_id(attempt_id: &str, instruction_ids: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(attempt_id.as_bytes());
    for id in instruction_ids {
        hasher.update(b"\n");
        hasher.update(id.as_bytes());
    }
    format!("rework_claim_{}", &hex::encode(hasher.finalize())[..16])
}

impl super::CodingAttemptStore {
    fn rework_claims_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<std::path::PathBuf, ProductStoreError> {
        validate_relative_id(attempt_id)?;
        Ok(self
            .attempt_dir(project_id, issue_id, attempt_id)
            .join("rework-claims.json"))
    }

    /// C2 Task 7：列出 attempt 的全部认领记录（只读；Task 12 投影
    /// `instruction_claim_interrupted` 等待项的数据源）。
    pub fn list_rework_instruction_claims(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<ReworkInstructionClaim>, ProductStoreError> {
        let path = self.rework_claims_path(project_id, issue_id, attempt_id)?;
        if !path.is_file() {
            return Ok(Vec::new());
        }
        Ok(read_json::<ReworkClaimJournal>(&path)?.records)
    }

    /// C2 Task 7：渲染→认领→消费 事务入口。
    ///
    /// 前置：调用侧已完成完整 prompt 渲染（指令／note 全文进入 prompt
    /// 与执行上下文）。本方法在 attempt 级文件锁内完成可重放原子写入：
    /// - 空 prompt（含空白）→ `RenderFailed`，禁消费；
    /// - 同一指令集合的既有认领＋同渲染 digest／context hash →
    ///   `Replayed`（同一 claim，幂等补齐消费标记，不二次消费）；
    /// - 同集合异 digest，或任一指令已属其他认领 → IdentityMismatch
    ///   fail-closed（已记录的执行上下文 hash 不被覆盖为新含义）。
    pub fn claim_and_consume_rework_instructions(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        rework_round: u32,
        rendered_prompt: &str,
        rendered_context_hash: Option<&str>,
        instruction_ids: &[String],
    ) -> Result<ReworkClaimOutcome, ProductStoreError> {
        let mut ids = instruction_ids.to_vec();
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return Err(ProductStoreError::Io(
                "rework_claim_empty_instruction_set".to_string(),
            ));
        }
        if rendered_prompt.trim().is_empty() {
            return Ok(ReworkClaimOutcome::RenderFailed);
        }
        let rendered_prompt_digest = rework_prompt_digest(rendered_prompt);
        let context_hash = rendered_context_hash
            .filter(|hash| !hash.trim().is_empty())
            .unwrap_or(&rendered_prompt_digest)
            .to_string();
        let claim_id = deterministic_claim_id(&attempt.id, &ids);
        let path = self.rework_claims_path(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        with_exclusive_lock(&path, || {
            let mut journal: ReworkClaimJournal = if path.is_file() {
                read_json(&path)?
            } else {
                ReworkClaimJournal::default()
            };
            if let Some(existing) = journal
                .records
                .iter()
                .find(|record| record.claim_id == claim_id)
            {
                if existing.rendered_prompt_digest != rendered_prompt_digest
                    || existing.context_hash != context_hash
                {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "coding_rework_instruction_claim",
                        id: claim_id.clone(),
                    });
                }
                // 认领 journal 先于消费标记落盘：中断窗口内标记可能缺失，
                // 重放时幂等补齐（不生成新 hash、不二次消费）。
                self.apply_rework_consumption_marks(attempt, node_id, rework_round, &ids)?;
                return Ok(ReworkClaimOutcome::Replayed {
                    claim: existing.clone(),
                });
            }
            if let Some(owner) = journal
                .records
                .iter()
                .find(|record| record.instruction_ids.iter().any(|id| ids.contains(id)))
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_rework_instruction_claim",
                    id: owner.claim_id.clone(),
                });
            }
            let claim = ReworkInstructionClaim {
                claim_id,
                attempt_id: attempt.id.clone(),
                instruction_ids: ids.clone(),
                rendered_prompt_digest,
                context_hash,
                consumed_at: Some(Utc::now().to_rfc3339()),
            };
            journal.records.push(claim.clone());
            write_json(&path, &journal)?;
            self.apply_rework_consumption_marks(attempt, node_id, rework_round, &ids)?;
            Ok(ReworkClaimOutcome::Claimed { claim })
        })
    }

    /// 幂等补齐消费标记：rework instruction 记 `consumed_by_node_id`，
    /// context note 记 `consumed_by_rework_round`。两类记录分目录存储、
    /// id 不重叠；两者都不存在则 fail-closed（认领集合必须可落账）。
    fn apply_rework_consumption_marks(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        rework_round: u32,
        ids: &[String],
    ) -> Result<(), ProductStoreError> {
        for id in ids {
            let instruction_path = self
                .rework_instructions_root(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .join(format!("{id}.json"));
            if instruction_path.is_file() {
                self.mark_rework_instruction_consumed(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    id,
                    node_id,
                )?;
                continue;
            }
            let note_path = self
                .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .join("context-notes")
                .join(format!("{id}.json"));
            if note_path.is_file() {
                self.mark_context_notes_consumed(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    std::slice::from_ref(id),
                    rework_round,
                )?;
                continue;
            }
            return Err(ProductStoreError::NotFound {
                kind: "coding_rework_instruction_or_note",
                id: id.clone(),
            });
        }
        Ok(())
    }
}

// ─── C-1b（oracle 裁决）：恢复重驱对账 rework-claims journal ───

/// C-1b：指令消费中断等待事实（对账落账；Task 12 投影 kind
/// `instruction_claim_interrupted` 的数据源）。
///
/// 语义：claim 存在（含消费标记）而其 node 无 role run 输出——消费标记
/// 落地后、spawn 前中断（Window B），指令已消费却从未进入任何 prompt。
/// 下一次 coder 重驱以 claim.instruction_ids 强制入渲染（不重写 claim、
/// 不二次消费），重放完成后对账清除本事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionClaimInterruptedFact {
    pub claim_id: String,
    pub attempt_id: String,
    pub instruction_ids: Vec<String>,
    pub consumed_at: Option<String>,
    pub detected_at: String,
}

/// C-1b：对账结果——被判定中断的认领及其指令／上下文备注内容（供重驱
/// 强制入渲染；内容由调用方渲染，本结构不承载渲染产物）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptedClaimRender {
    pub claim: ReworkInstructionClaim,
    pub instructions: Vec<crate::product::coding_models::CodingReworkInstruction>,
    pub notes: Vec<crate::product::coding_models::CodingContextNote>,
}

impl super::CodingAttemptStore {
    fn instruction_claim_interrupted_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<std::path::PathBuf, ProductStoreError> {
        validate_relative_id(attempt_id)?;
        Ok(self
            .attempt_dir(project_id, issue_id, attempt_id)
            .join("instruction-claim-interrupted"))
    }

    /// C-1b：列出指令消费中断等待事实（只读；Task 12 投影数据源）。
    pub fn list_instruction_claim_interrupted_facts(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<InstructionClaimInterruptedFact>, ProductStoreError> {
        let root = self.instruction_claim_interrupted_root(project_id, issue_id, attempt_id)?;
        if !root.is_dir() {
            return Ok(Vec::new());
        }
        let mut facts: Vec<InstructionClaimInterruptedFact> = Vec::new();
        for path in super::json_file_paths(&root)? {
            facts.push(read_json(&path)?);
        }
        facts.sort_by(|left, right| left.claim_id.cmp(&right.claim_id));
        Ok(facts)
    }

    /// C-1b 对账恢复：逐条审视 attempt 的认领 journal——
    /// - claim 存在而其 node（指令 `consumed_by_node_id` 归因）无
    ///   ProviderPrompt 事件（消费标记后、spawn 前中断），且无任何后续
    ///   coder run 已把 prompt 发出（`started_at` 晚于 claim 消费时间且带
    ///   ProviderPrompt）→ 落 `instruction-claim-interrupted/{claim_id}.json`
    ///   等待事实，并返回该认领的指令／备注供重驱强制入渲染；
    /// - 反之（认领自身的 run 或任一后续 run 已发出 prompt）→ 清除等待
    ///   事实（若在），返回空。
    ///
    /// 不重写 claim journal、不做二次消费标记：强制回放的指令 id 不进入
    /// 重驱路径的新认领集合。纯 context note 认领（无 node 可归因）不参与
    /// 对账——备注为辅助上下文，BYPASS-18 的消费主对象是返修指令。
    pub fn reconcile_interrupted_rework_claims(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<InterruptedClaimRender>, ProductStoreError> {
        use crate::product::coding_models::CodingRoleRunEventType;
        use std::collections::HashMap;

        let claims = self.list_rework_instruction_claims(project_id, issue_id, attempt_id)?;
        if claims.is_empty() {
            return Ok(Vec::new());
        }
        let instructions_by_id: HashMap<
            String,
            crate::product::coding_models::CodingReworkInstruction,
        > = self
            .list_rework_instructions(project_id, issue_id, attempt_id)?
            .into_iter()
            .map(|instruction| (instruction.id.clone(), instruction))
            .collect();
        let notes_by_id: HashMap<String, crate::product::coding_models::CodingContextNote> = self
            .list_context_notes(project_id, issue_id, attempt_id)?
            .into_iter()
            .map(|note| (note.id.clone(), note))
            .collect();
        // 每个 role run 是否已发出 prompt（ProviderPrompt 事件在 provider
        // 启动前落账——存在即证明 prompt 已真实发出）。
        let mut run_prompted: HashMap<String, bool> = HashMap::new();
        for run in self.list_role_runs(project_id, issue_id, attempt_id)? {
            let prompted = self
                .list_role_run_events(project_id, issue_id, attempt_id, &run.id)?
                .iter()
                .any(|event| event.event_type == CodingRoleRunEventType::ProviderPrompt);
            run_prompted.insert(run.id.clone(), prompted);
        }
        let role_runs = self.list_role_runs(project_id, issue_id, attempt_id)?;
        let fact_root =
            self.instruction_claim_interrupted_root(project_id, issue_id, attempt_id)?;

        let mut renders = Vec::new();
        for claim in claims {
            // journal 落盘但消费标记缺失（Window A'）：重放命中同一认领时
            // 幂等补齐标记，不走对账强制回放。
            let Some(consumed_at) = claim.consumed_at.as_deref() else {
                continue;
            };
            let claim_nodes: Vec<&str> = claim
                .instruction_ids
                .iter()
                .filter_map(|id| {
                    instructions_by_id
                        .get(id)
                        .and_then(|instruction| instruction.consumed_by_node_id.as_deref())
                })
                .collect();
            if claim_nodes.is_empty() {
                continue;
            }
            let own_prompted = role_runs.iter().any(|run| {
                run.node_id
                    .as_deref()
                    .is_some_and(|node| claim_nodes.contains(&node))
                    && run_prompted.get(&run.id).copied().unwrap_or(false)
            });
            // 解析失败按未重放处理（重复投递优于静默丢失）。
            let consumed_time = chrono::DateTime::parse_from_rfc3339(consumed_at).ok();
            let later_prompted = role_runs.iter().any(|run| {
                run_prompted.get(&run.id).copied().unwrap_or(false)
                    && consumed_time
                        .zip(chrono::DateTime::parse_from_rfc3339(&run.started_at).ok())
                        .is_some_and(|(consumed, started)| started > consumed)
            });
            let fact_path = fact_root.join(format!("{}.json", claim.claim_id));
            if own_prompted || later_prompted {
                // 已发出 prompt（认领自身或后续重驱回放）：清除等待事实。
                if fact_path.exists() {
                    std::fs::remove_file(&fact_path).map_err(|error| {
                        ProductStoreError::Io(format!("remove {}: {error}", fact_path.display()))
                    })?;
                }
                continue;
            }
            let fact = InstructionClaimInterruptedFact {
                claim_id: claim.claim_id.clone(),
                attempt_id: claim.attempt_id.clone(),
                instruction_ids: claim.instruction_ids.clone(),
                consumed_at: claim.consumed_at.clone(),
                detected_at: Utc::now().to_rfc3339(),
            };
            std::fs::create_dir_all(&fact_root).map_err(|error| {
                ProductStoreError::Io(format!("create {}: {error}", fact_root.display()))
            })?;
            write_json(&fact_path, &fact)?;
            let instructions = claim
                .instruction_ids
                .iter()
                .filter_map(|id| instructions_by_id.get(id).cloned())
                .collect::<Vec<_>>();
            let notes = claim
                .instruction_ids
                .iter()
                .filter_map(|id| notes_by_id.get(id).cloned())
                .collect::<Vec<_>>();
            renders.push(InterruptedClaimRender {
                claim,
                instructions,
                notes,
            });
        }
        Ok(renders)
    }
}
