// C-4 跨仓只读证据中介（EvidenceIndex）。
//
// Task 1（spike）已完成：在 2 成员仓临时聚合根（api/ + web/）上实测
// `codegraph query --json` 的真实输出，固化为 tests/fixtures/query_hit.json，
// 并有一条解析冒烟测试断言字段存在（见 tests/query_hit.rs）。
//
// Task 2 在此实现 `EvidenceHit { file_path, start_line, symbol }` 解析：
// file_path <- node.filePath（聚合根相对路径），start_line <- node.startLine，
// symbol <- node.name。
//
// `EvidenceIndexQuery` 封装既有 `CodeGraphCli::query_json`（aggregate_index/
// codegraph_cli.rs），解析出命中后按聚合根相对路径首段（成员目录名）过滤：
// 仅保留「首段 ∈ 成员目录名 且 != 目标成员目录名」的跨成员命中；命中上限
// MAX_EVIDENCE_HITS（200）条，超出截断（`truncated` 标记由 T6 响应层表达，
// 本层只做截断）。跨成员命中是 import 节点而非调用点——启发式符号引用可接受，
// 本 Task 照常按签名文本查询，不扩展。

use std::path::PathBuf;

use crate::product::logical_codebase::aggregate_index::{AggregateIndexError, CodeGraphCli};

/// 单条跨仓证据命中（来源成员仓文件中的一处符号引用/定义）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceHit {
    /// 聚合根相对路径（首段为成员目录名，如 `web/src/app.ts`）。
    pub file_path: String,
    /// 命中节点起始行（1-based）。
    pub start_line: u64,
    /// 符号文本：定义点取 `node.name`；跨成员 import 命中为 import 引用串（启发式）。
    pub symbol: String,
}

/// 证据查询/解析错误。错误码字符串与设计稳定码前缀 `evidence_` 对齐
/// （见方案设计 §5.2）：非法查询词 → `evidence_invalid_query`；
/// 索引不可用/查询执行失败 → `evidence_query_failed`（携带底层码，如
/// `codegraph_query_invalid_json` 透传，供 T6 映射 HTTP 状态与日志）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvidenceError {
    #[error("evidence_invalid_query:{reason}")]
    InvalidQuery { reason: String },
    #[error("evidence_query_failed:{code}: {message}")]
    QueryFailed { code: &'static str, message: String },
}

/// 单次查询返回的最大命中数，超出截断。
pub const MAX_EVIDENCE_HITS: usize = 200;

/// 查询符号的最大长度（字符），超出按非法查询词拒绝。
const MAX_QUERY_SYMBOL_CHARS: usize = 512;

/// 跨仓证据查询封装：包住 `CodeGraphCli::query_json`，按成员目录过滤并截断。
pub struct EvidenceIndexQuery {
    runner: CodeGraphCli,
    aggregate_root: PathBuf,
    member_dir_names: Vec<String>,
    target_member_dir: String,
}

impl EvidenceIndexQuery {
    pub fn new(
        runner: CodeGraphCli,
        aggregate_root: PathBuf,
        member_dir_names: Vec<String>,
        target_member_dir: String,
    ) -> Self {
        Self {
            runner,
            aggregate_root,
            member_dir_names,
            target_member_dir,
        }
    }

    /// 对 `symbol` 发起 `codegraph query --json`，解析命中并按成员目录过滤后返回。
    ///
    /// 仅保留 `file_path` 首段（首个 `/` 前）∈ `member_dir_names` 且
    /// != `target_member_dir` 的命中；结果截断至 `MAX_EVIDENCE_HITS` 条。
    pub fn query(&self, symbol: &str) -> Result<Vec<EvidenceHit>, EvidenceError> {
        validate_symbol(symbol)?;
        let value = self
            .runner
            .query_json(&self.aggregate_root, symbol)
            .map_err(EvidenceError::from)?;
        let mut hits = parse_hits(&value)?;
        hits.retain(|hit| self.is_cross_member_hit(&hit.file_path));
        hits.truncate(MAX_EVIDENCE_HITS);
        Ok(hits)
    }

    fn is_cross_member_hit(&self, file_path: &str) -> bool {
        let first_segment = file_path.split('/').next().unwrap_or("");
        first_segment != self.target_member_dir
            && self
                .member_dir_names
                .iter()
                .any(|name| name == first_segment)
    }
}

impl From<AggregateIndexError> for EvidenceError {
    fn from(error: AggregateIndexError) -> Self {
        match error {
            AggregateIndexError::Degraded { code, message } => Self::QueryFailed { code, message },
            AggregateIndexError::Failed { code, message } => Self::QueryFailed { code, message },
        }
    }
}

fn validate_symbol(symbol: &str) -> Result<(), EvidenceError> {
    if symbol.trim().is_empty() {
        return Err(EvidenceError::InvalidQuery {
            reason: "query symbol must not be empty".to_string(),
        });
    }
    if symbol.chars().count() > MAX_QUERY_SYMBOL_CHARS {
        return Err(EvidenceError::InvalidQuery {
            reason: format!(
                "query symbol exceeds {MAX_QUERY_SYMBOL_CHARS} chars (got {})",
                symbol.chars().count()
            ),
        });
    }
    Ok(())
}

fn parse_hits(value: &serde_json::Value) -> Result<Vec<EvidenceHit>, EvidenceError> {
    let entries = value.as_array().ok_or_else(|| {
        parse_failed(format!(
            "codegraph query result must be a JSON array, got {}",
            value_type_name(value)
        ))
    })?;
    let mut hits = Vec::with_capacity(entries.len());
    for entry in entries {
        let node = entry
            .get("node")
            .ok_or_else(|| parse_failed("codegraph query hit must contain a `node` object"))?;
        let file_path = node
            .get("filePath")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| parse_failed("codegraph query node must contain string `filePath`"))?;
        let start_line = node
            .get("startLine")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| parse_failed("codegraph query node must contain u64 `startLine`"))?;
        let symbol = node
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| parse_failed("codegraph query node must contain string `name`"))?;
        hits.push(EvidenceHit {
            file_path: file_path.to_string(),
            start_line,
            symbol: symbol.to_string(),
        });
    }
    Ok(hits)
}

fn parse_failed(message: impl Into<String>) -> EvidenceError {
    EvidenceError::QueryFailed {
        code: "evidence_query_parse_failed",
        message: message.into(),
    }
}

fn value_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests;
