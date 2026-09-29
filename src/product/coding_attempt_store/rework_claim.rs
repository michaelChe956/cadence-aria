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
        let path =
            self.rework_claims_path(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
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
