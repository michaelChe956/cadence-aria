//! 确定性结构治理的 typed 失败与资格纯核心（F1.1，REQ-SG-01/02/05）。
//!
//! 本模块只承载封闭的失败 payload、授权事实快照与纯分类函数：
//! - `PlanningFailureKind`：typed 结构失败 payload。中文诊断与安全详情由
//!   持久 diagnostic record（`PlanningFailure::diagnostic_ref` 指向）与后续
//!   wire projection 承载；分类一律不读自由文本 message，也不取
//!   category/class_hint/severity 当许可（REQ-SG-01）。
//! - `StructuralAuthorizationFacts`：外层读取的耐久授权事实快照。类型本身
//!   不构成新的入站许可；`active/anchor_waiting/launch_unknown/attempt` 只是
//!   只读事实，不能替代 gateway admission/full fingerprint（REQ-SG-05）。
//! - `classify_failure_set`：把失败集合与授权事实确定性映射到五种恢复
//!   disposition。纯函数、无 I/O、不调用模型。
//!
//! serde 形状面向后续 SG 包的耐久冻结（SG3a.0/SG3b）与 wire projection
//! （SG7）。按 plan §0.5 哈希兼容纪律，后续新增字段必须采用
//! `Option` + `serde(default, skip_serializing_if)`；因此这里不使用
//! `deny_unknown_fields`，保证旧读取器遇到新字段仍可解析。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// typed 结构失败 payload 的封闭 enum。kind 携带判定所需的全部机器事实；
/// 每个变体由确定性 producer（F1.2 compiler、F2a canonical、F2b split/
/// preflight、F2c projection、SG1 有效图、SG4A 防回归）显式构造，旧 wire
/// 记录只能折叠为 `Unknown`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanningFailureKind {
    /// 非法二级 heading（REQ-SG-02 第414行场景）：保留 source 原行
    /// （1-based `line` + `raw_line`）、grammar `field` 与恰好一个可回喂
    /// `example`；修复必须经 author，不得按共享 unknown key 码删行。
    InvalidWorkItemHeading {
        line: usize,
        field: String,
        raw_line: String,
        example: String,
    },
    /// 有效图依赖环：`witness` 是真实有向路径，每一跳都确在有效图中并标
    /// 来源（canonical Inputs / change_order 自动边），末跳回到首跳起点。
    /// 解不唯一不影响 L1 资格；witness 不完整则整批 fail-closed。
    DependencyCycle {
        witness: Vec<PlanningCycleWitnessHop>,
    },
    /// canonical Inputs 引用缺口（F2a，REQ-PLN-09）：消费方引用的 provider
    /// 不在本 plan 内。producer 完整保留 provider/consumer/contract ID；
    /// 真实 Inputs 引用不因检出被删除，修复经 author 重组。
    UnknownProvider {
        provider: String,
        consumer: String,
        contract_id: String,
    },
    /// canonical Inputs 供需缺口（F2a，REQ-PLN-09）：provider 存在但未声明
    /// 被消费的契约。producer 保留 provider/consumer/contract ID。
    RequiredContractMissing {
        provider: String,
        consumer: String,
        contract_id: String,
    },
    /// canonical Inputs 能力缺口（F2a，REQ-PLN-09）：契约已声明但 required
    /// capability 未被逐字提供。producer 保留 provider/consumer/contract/
    /// capability 四元 ID，缺口精确到 capability 级。
    RequiredCapabilityMissing {
        provider: String,
        consumer: String,
        contract_id: String,
        capability: String,
    },
    /// target×成员归属不一致：逻辑 WI 的声明/解析归属与授权成员或 Plan
    /// Intent 解析归属不同构；真实 Inputs/AC 保留，由 author 重组。
    TargetMismatch {
        logical_work_item_id: String,
        declared_target: Option<String>,
        resolved_target: String,
    },
    /// 已解决结构事实的回归（REQ-SG-04A）：携带旧合法 refs、本次差异与
    /// 已解决事实 digest；修复 prompt 必须含这些事实，不能只靠模型记忆。
    StructuralRegression {
        resolved_fact_digest: String,
        last_valid_refs: LastValidStructureRefs,
        difference: String,
    },
    /// 工程/运行内部错（投影漂移、store 错误等）：不驱 author，沿原安全
    /// 等待/协议恢复通道处理；`stable_code` 是该内部错的耐久稳定码。
    Operational { stable_code: String, cause: String },
    /// 旧 wire 记录 / 未 typed 诊断：`code` 仅为可诊断投影保留，绝不产生
    /// L1 资格，也不得反推出 kind（不从 Display/消息拆码）。
    Unknown { code: String },
}

impl PlanningFailureKind {
    /// 稳定 diagnostic code，由 kind 单源映射。heading/cycle/target/regression
    /// 是固定常量；`Operational`/`Unknown` 投影 payload 内的耐久码。
    pub fn stable_code(&self) -> &str {
        match self {
            Self::InvalidWorkItemHeading { .. } => "invalid_work_item_heading",
            Self::DependencyCycle { .. } => "dependency_cycle",
            Self::UnknownProvider { .. } => "unknown_provider_logical_work_item",
            Self::RequiredContractMissing { .. } => "required_contract_missing",
            Self::RequiredCapabilityMissing { .. } => "required_capability_missing",
            Self::TargetMismatch { .. } => "target_mismatch",
            Self::StructuralRegression { .. } => "structural_regression",
            Self::Operational { stable_code, .. } => stable_code,
            Self::Unknown { code } => code,
        }
    }

    /// 稳定 failure category，由 kind 单源映射，供统计/呈现分组；不是
    /// review finding 的 class hint，也不能替代服务端 L1 资格证明。
    pub fn category(&self) -> PlanningFailureCategory {
        match self {
            Self::InvalidWorkItemHeading { .. } => PlanningFailureCategory::Grammar,
            Self::DependencyCycle { .. } => PlanningFailureCategory::Dependency,
            Self::UnknownProvider { .. }
            | Self::RequiredContractMissing { .. }
            | Self::RequiredCapabilityMissing { .. } => PlanningFailureCategory::Dependency,
            Self::TargetMismatch { .. } => PlanningFailureCategory::Attribution,
            Self::StructuralRegression { .. } => PlanningFailureCategory::Regression,
            Self::Operational { .. } => PlanningFailureCategory::Operational,
            Self::Unknown { .. } => PlanningFailureCategory::Unknown,
        }
    }
}

/// 依赖环 witness 的单跳：`from`→`to` 边及其全部来源。同一端点有两种来源
/// 时两种都保留（SG1 双来源边），不因删一来源就声称边消失。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningCycleWitnessHop {
    pub from: String,
    pub to: String,
    pub sources: Vec<PlanningEdgeSource>,
}

/// 有效图边的来源：canonical Inputs provider→consumer 边，或 change_order
/// 按仓序自动展开的前仓→后仓边（方向恒为前置→后置）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningEdgeSource {
    CanonicalInput,
    ChangeOrder,
}

/// 整组不可变旧合法 refs（source/IR/report），历史恢复与防回归的最小身份
/// 单元；缺任一 ref 不能凭版本号补造“旧评审 IR”。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastValidStructureRefs {
    pub source_ref: String,
    pub ir_ref: String,
    pub report_ref: String,
}

/// 稳定 failure category 封闭枚举（kind 单源映射，见
/// `PlanningFailureKind::category`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningFailureCategory {
    Grammar,
    Dependency,
    Attribution,
    Regression,
    Operational,
    Unknown,
}

impl PlanningFailureCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Grammar => "grammar",
            Self::Dependency => "dependency",
            Self::Attribution => "attribution",
            Self::Regression => "regression",
            Self::Operational => "operational",
            Self::Unknown => "unknown",
        }
    }
}

/// 失败检出来源层：同一 kind（如 DependencyCycle）可来自不同层，origin 保留
/// 该事实供指令与审计；classification 不因 origin 改变资格。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningFailureOrigin {
    /// parser/lower 的 grammar 层（F1.2 typed producer）。
    Compiler,
    /// canonical Inputs 供需/依赖合同（F2a）。
    CanonicalContract,
    /// options×items 预检（F2b）。
    SplitPreflight,
    /// target×成员归属核验（SG1/F2a）。
    TargetAttribution,
    /// Inputs∪change_order 有效图排序阻断（SG1）。
    EffectiveGraph,
    /// logical/physical 投影同构（F2c/SG3b Final Compile）。F2c 起
    /// projection 校验 finding 继承 canonical 依赖 kind（IDs 原样），其
    /// projection 内部 invariant 违反一律 Operational——同一 kind 可来自
    /// 不同层，origin 保留该事实，classification 不因 origin 改变资格。
    Projection,
    /// 系统/工程内部错（不驱 author 的 operational 源头）。
    System,
}

/// 五阶段产品载体。失败知道自己来自哪个阶段；coding/review 不虚造 WI
/// 规划图，story/design 只用各自真实 schema（REQ-SG-01）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningStage {
    Story,
    Design,
    Plan,
    Coding,
    Review,
}

/// 单条 typed 结构失败：kind payload + 身份/定位事实。`diagnostic_ref` 指向
/// 持久诊断记录（含中文人话详情），`source_ref/source_hash` 绑定不可变
/// source；分类不读任何 message 文本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningFailure {
    pub kind: PlanningFailureKind,
    pub origin: PlanningFailureOrigin,
    pub stage: PlanningStage,
    pub diagnostic_ref: String,
    pub source_ref: String,
    pub source_hash: String,
    pub evidence: PlanningFailureEvidence,
}

/// 失败的结构化关联事实：受影响逻辑 WI/target ID 与稳定 key→value 详情。
/// `affected_ids` 保持 producer 给出的真实顺序（witness 顺序），不排序去重
/// 吞事实；F2a 验收要求 dependency/self/unknown/provider 缺口不丢 ID。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningFailureEvidence {
    pub affected_ids: Vec<String>,
    pub details: BTreeMap<String, String>,
}

/// 一次交付全量校验产生的失败集合。集合是分类的最小单位：任何一个
/// 不合格失败（Unknown/Operational/事实不完整）都会让整批不自动。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningFailureSet {
    pub failures: Vec<PlanningFailure>,
}

impl PlanningFailureSet {
    pub fn is_empty(&self) -> bool {
        self.failures.is_empty()
    }
}

/// SG3a.0 持久有效图边的源定位（plan §0.6.2 冻结投影）：canonical 源内
/// 字段路径与 1-based 行号。与 engine 私有的 `PlanningStructureSpan`
/// 同形但独立公开——source store 与 wire 不得依赖 engine 私有图类型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedPlanningStructureSpan {
    pub field: String,
    pub line: usize,
}

/// SG3a.0 持久有效图边（plan §0.6.2）：`from`→`to` 及全部来源证据。
/// `sources` 固定保留全部来源（canonical_input→`contract_refs`/`spans`、
/// change_order→`order_indices` 为 `[前仓索引, 后仓索引]`），同端点多
/// 来源不合并丢弃；这是 SG1 纯图的单次冻结投影，读取方不得把它回读成
/// 图算法第二权威（不重派生边、不重算拓扑）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedEffectivePlanningEdge {
    pub from: String,
    pub to: String,
    pub sources: Vec<PlanningEdgeSource>,
    pub contract_refs: Vec<String>,
    pub spans: Vec<PersistedPlanningStructureSpan>,
    pub order_indices: Vec<usize>,
}

/// SG3a.0 immutable 结构报告 payload（plan §0.6.2）：SG1 纯报告的**单次
/// 冻结投影**，随 `PlanCandidateMechanicalReportRecord.structure_report`
/// 在同一 report-record/content_hash 中耐久保存。
/// - 身份绑定：`source_revision_id`/`source_hash`/`ir_id`/
///   `compiler_version`/`rule_version`/`context_digest` 与机械三 refs
///   同代；`report_digest` 由 store 单源函数封口（payload 除该字段外
///   全量 sha256），source/IR/context/report digest 任一漂移即拒。
/// - 结构事实：`effective_edges`/`cycle_witness`（每跳真实、可为空
///   合法 DAG）、`findings`/`structurally_valid`（合法报告不含未解决
///   阻断）。
/// - 治理引用：`protected_predicates`/`instruction_refs`/
///   `repair_audit_refs`（SG3a.1/SG6 后续消费；初轮无修复明确空）。
///
/// 旧记录缺该字段解析为 `None`，仅 Unknown/参考，不原位补造结构证据；
/// 生产 producer 在 SG3a.1/SG6 之前不写 `Some`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedPlanningStructureReport {
    pub source_revision_id: String,
    pub source_hash: String,
    pub ir_id: String,
    pub compiler_version: String,
    pub rule_version: String,
    pub context_digest: String,
    pub effective_edges: Vec<PersistedEffectivePlanningEdge>,
    pub cycle_witness: Vec<PersistedEffectivePlanningEdge>,
    pub findings: PlanningFailureSet,
    pub structurally_valid: bool,
    pub protected_predicates: Vec<String>,
    pub instruction_refs: Vec<String>,
    pub repair_audit_refs: Vec<String>,
    pub report_digest: String,
}

/// 耐久授权事实快照（REQ-SG-05）。全部字段由外层从既有 grant/turn/run/
/// role-run 读取；本类型不新增入站许可，也不能绕过 gateway admission。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralAuthorizationFacts {
    pub authorization_id: String,
    pub authorization_kind: StructuralAuthorizationKind,
    pub parent_id: String,
    pub source_hash: String,
    pub context_digest: String,
    /// 该授权当前是否活动（未终态）。
    pub active: bool,
    /// anchor/trust 等待中：优先沿原等待，不借 L1 启动 author。
    pub anchor_waiting: bool,
    /// 上次外部调用结果未知：等待，不盲目重播。
    pub launch_unknown: bool,
    /// 已消费的物理 attempt 序号（1-based；0 表示尚未开始）。
    pub attempt_no: u32,
    /// 同一授权的物理 attempt 上限。
    pub max_attempts: u32,
}

/// 授权载体 kind（REQ-SG-05 逐载体表）：SC 人工修订 turn、既有生成 run、
/// 持久 Revision node、legacy 整计划 revision、coding/review role-run。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuralAuthorizationKind {
    ScTurn,
    GenerationRun,
    RevisionNode,
    LegacyRun,
    CodingRoleRun,
}

/// 失败集合的确定性恢复 disposition（五值封闭，plan §0.5）：
/// - `StructuralAuthor`：L1——事实完整的确定性结构失败 + 活动授权 + 剩余
///   物理 attempt；机器生成靶向指令，交原授权 author 执行并全量复验。
///   解不唯一不影响资格；不含业务矛盾证据时绝不升级 L3。
/// - `TechnicalRoute`：L2——沿既有技术返修/语义 review 合同。空集合（无
///   结构失败）也走此路由；本核心不从结构事实合成 L2。
/// - `BusinessGate`：L3——只有显式业务矛盾证据才允许（REQ-SG-01）；
///   本模块的封闭 kind 均不携带该证据，`classify_failure_set` 不产生它。
/// - `OperationalWait`：含 Operational 失败或授权事实缺失/anchor 等待/
///   未知外部调用——沿原安全等待/协议恢复，不借 L1 启动 author。
/// - `UnknownStop`：fail-closed 停止面——含未 typed/事实不完整失败，或
///   同一授权物理 attempt 已耗尽（工程故障，不偷第 3 次物理 start）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningRecoveryDisposition {
    StructuralAuthor,
    TechnicalRoute,
    BusinessGate,
    OperationalWait,
    UnknownStop,
}

impl PlanningRecoveryDisposition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StructuralAuthor => "structural_author",
            Self::TechnicalRoute => "technical_route",
            Self::BusinessGate => "business_gate",
            Self::OperationalWait => "operational_wait",
            Self::UnknownStop => "unknown_stop",
        }
    }
}

/// 判定依赖环 witness 是否事实完整：非空、每跳至少一个来源、相邻跳首尾
/// 相接、末跳回到首跳起点（每跳确在有效图中，Kahn 残余节点列表不是
/// witness，不得画成 A→B→A）。
pub fn is_complete_cycle_witness(witness: &[PlanningCycleWitnessHop]) -> bool {
    if witness.is_empty() {
        return false;
    }
    if witness.iter().any(|hop| hop.sources.is_empty()) {
        return false;
    }
    if !witness.windows(2).all(|pair| pair[0].to == pair[1].from) {
        return false;
    }
    witness
        .last()
        .is_some_and(|last| last.to == witness[0].from)
}

/// 把失败集合与授权事实映射到恢复 disposition。纯函数：不读 message/
/// class_hint/severity，不做 I/O，不调用模型；同一 (set, facts) 恒等输出。
///
/// 优先级（混合集合整批裁决，不部分自动）：
/// 1. 任一 `Unknown` 或事实不完整的 L1 kind → `UnknownStop`（最保守，
///    诊断本身不可信时停止面优先于等待面）；
/// 2. 任一 `Operational` → `OperationalWait`；
/// 3. 授权不活动 / anchor 等待 / 未知外部调用 → `OperationalWait`
///    （REQ-SG-01：安全/授权事实缺失沿原安全等待/协议恢复）；
/// 4. 物理 attempt 耗尽（`attempt_no >= max_attempts`）→ `UnknownStop`
///    （typed 系统故障停止面，不偷借新 turn/第 3 次 start）；
/// 5. 其余（事实完整的确定性结构失败）→ `StructuralAuthor`。
pub fn classify_failure_set(
    set: &PlanningFailureSet,
    facts: &StructuralAuthorizationFacts,
) -> PlanningRecoveryDisposition {
    if set.is_empty() {
        return PlanningRecoveryDisposition::TechnicalRoute;
    }
    let mut has_operational = false;
    for failure in &set.failures {
        match &failure.kind {
            PlanningFailureKind::Unknown { .. } => {
                return PlanningRecoveryDisposition::UnknownStop;
            }
            PlanningFailureKind::Operational { .. } => {
                has_operational = true;
            }
            kind if !l1_facts_complete(kind) => {
                return PlanningRecoveryDisposition::UnknownStop;
            }
            _ => {}
        }
    }
    if has_operational {
        return PlanningRecoveryDisposition::OperationalWait;
    }
    if !facts.active || facts.anchor_waiting || facts.launch_unknown {
        return PlanningRecoveryDisposition::OperationalWait;
    }
    if facts.attempt_no >= facts.max_attempts {
        return PlanningRecoveryDisposition::UnknownStop;
    }
    PlanningRecoveryDisposition::StructuralAuthor
}

/// L1 kind 的机器事实完整性：缺原行/field/example 的 heading、缺 ID 的
/// target 归属错或缺旧 refs 的回归都不能生成精确指令，整批 fail-closed。
fn l1_facts_complete(kind: &PlanningFailureKind) -> bool {
    match kind {
        PlanningFailureKind::InvalidWorkItemHeading {
            line,
            field,
            raw_line,
            example,
        } => *line >= 1 && !field.is_empty() && !raw_line.is_empty() && !example.is_empty(),
        PlanningFailureKind::DependencyCycle { witness } => is_complete_cycle_witness(witness),
        PlanningFailureKind::UnknownProvider {
            provider,
            consumer,
            contract_id,
        }
        | PlanningFailureKind::RequiredContractMissing {
            provider,
            consumer,
            contract_id,
        } => !provider.is_empty() && !consumer.is_empty() && !contract_id.is_empty(),
        PlanningFailureKind::RequiredCapabilityMissing {
            provider,
            consumer,
            contract_id,
            capability,
        } => {
            !provider.is_empty()
                && !consumer.is_empty()
                && !contract_id.is_empty()
                && !capability.is_empty()
        }
        PlanningFailureKind::TargetMismatch {
            logical_work_item_id,
            resolved_target,
            ..
        } => !logical_work_item_id.is_empty() && !resolved_target.is_empty(),
        PlanningFailureKind::StructuralRegression {
            resolved_fact_digest,
            last_valid_refs,
            difference,
        } => {
            !resolved_fact_digest.is_empty()
                && !last_valid_refs.source_ref.is_empty()
                && !last_valid_refs.ir_ref.is_empty()
                && !last_valid_refs.report_ref.is_empty()
                && !difference.is_empty()
        }
        PlanningFailureKind::Operational { .. } | PlanningFailureKind::Unknown { .. } => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SC 修订 turn 的活动授权事实（attempt 1/2 已消费 1 次，剩 1 次）。
    fn active_sc_facts() -> StructuralAuthorizationFacts {
        StructuralAuthorizationFacts {
            authorization_id: "grant-sc-7f31".to_string(),
            authorization_kind: StructuralAuthorizationKind::ScTurn,
            parent_id: "turn-4140".to_string(),
            source_hash: "9f1c0a4d8b7e6253c0af1d2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2"
                .to_string(),
            context_digest: "ctx-5102beef".to_string(),
            active: true,
            anchor_waiting: false,
            launch_unknown: false,
            attempt_no: 1,
            max_attempts: 2,
        }
    }

    /// 第414行类非法二级 heading（REQ-SG-02 场景）：原行/field/example。
    fn heading_failure(line: usize) -> PlanningFailure {
        PlanningFailure {
            kind: PlanningFailureKind::InvalidWorkItemHeading {
                line,
                field: "work_item_heading".to_string(),
                raw_line: "## 验收说明".to_string(),
                example: "## Work Item WI-001: Levels API fixture".to_string(),
            },
            origin: PlanningFailureOrigin::Compiler,
            stage: PlanningStage::Plan,
            diagnostic_ref: "diag-9d02".to_string(),
            source_ref: "plan-source/rev-31".to_string(),
            source_hash: "9f1c0a4d8b7e6253c0af1d2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2"
                .to_string(),
            evidence: PlanningFailureEvidence {
                affected_ids: vec!["WI-005".to_string()],
                details: BTreeMap::from([(
                    "declared_heading".to_string(),
                    "## 验收说明".to_string(),
                )]),
            },
        }
    }

    /// R1 事故形态的依赖环（REQ-SG-01 场景“技术多解不要求用户填结构”）：
    /// WI-004→WI-005 为 canonical Inputs 边，WI-005→WI-004 为仓序自动边。
    fn r1_cycle_failure() -> PlanningFailure {
        PlanningFailure {
            kind: PlanningFailureKind::DependencyCycle {
                witness: vec![
                    PlanningCycleWitnessHop {
                        from: "WI-004".to_string(),
                        to: "WI-005".to_string(),
                        sources: vec![PlanningEdgeSource::CanonicalInput],
                    },
                    PlanningCycleWitnessHop {
                        from: "WI-005".to_string(),
                        to: "WI-004".to_string(),
                        sources: vec![PlanningEdgeSource::ChangeOrder],
                    },
                ],
            },
            origin: PlanningFailureOrigin::EffectiveGraph,
            stage: PlanningStage::Plan,
            diagnostic_ref: "diag-9d03".to_string(),
            source_ref: "plan-source/rev-31".to_string(),
            source_hash: "9f1c0a4d8b7e6253c0af1d2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2"
                .to_string(),
            evidence: PlanningFailureEvidence {
                affected_ids: vec!["WI-004".to_string(), "WI-005".to_string()],
                details: BTreeMap::new(),
            },
        }
    }

    fn unknown_failure() -> PlanningFailure {
        PlanningFailure {
            kind: PlanningFailureKind::Unknown {
                code: "legacy_wire_0414".to_string(),
            },
            origin: PlanningFailureOrigin::Compiler,
            stage: PlanningStage::Plan,
            diagnostic_ref: "diag-9d04".to_string(),
            source_ref: "plan-source/rev-31".to_string(),
            source_hash: "9f1c0a4d8b7e6253c0af1d2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2"
                .to_string(),
            evidence: PlanningFailureEvidence {
                affected_ids: vec!["WI-005".to_string()],
                details: BTreeMap::new(),
            },
        }
    }

    fn set_of(failures: Vec<PlanningFailure>) -> PlanningFailureSet {
        PlanningFailureSet { failures }
    }

    #[test]
    fn sg_f1_1_typed_disposition_heading_keeps_raw_line_field_example() {
        let failure = heading_failure(414);
        let PlanningFailureKind::InvalidWorkItemHeading {
            line,
            field,
            raw_line,
            example,
        } = &failure.kind
        else {
            panic!("fixture 必须是 InvalidWorkItemHeading");
        };
        assert_eq!(*line, 414);
        assert_eq!(field, "work_item_heading");
        assert_eq!(raw_line, "## 验收说明");
        assert_eq!(example, "## Work Item WI-001: Levels API fixture");
        // 关联 ID 与 source 身份随 payload 保留（不丢 ID / 可审计 refs）。
        assert_eq!(failure.evidence.affected_ids, vec!["WI-005".to_string()]);
        assert_eq!(failure.source_ref, "plan-source/rev-31");
        assert_eq!(
            failure.source_hash,
            "9f1c0a4d8b7e6253c0af1d2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2"
        );
    }

    #[test]
    fn sg_f1_1_typed_disposition_same_kind_stable_code_and_category() {
        // 同 kind、不同行/原文：code/category 必须一致（单源映射）。
        let a = heading_failure(414).kind;
        let b = heading_failure(88).kind;
        assert_eq!(a.stable_code(), "invalid_work_item_heading");
        assert_eq!(a.stable_code(), b.stable_code());
        assert_eq!(a.category(), b.category());
        assert_eq!(a.category().as_str(), "grammar");
        // 不同 kind 的 code/category 区分。
        let cycle = r1_cycle_failure().kind;
        assert_eq!(cycle.stable_code(), "dependency_cycle");
        assert_eq!(cycle.category().as_str(), "dependency");
        assert_ne!(a.stable_code(), cycle.stable_code());
        assert_ne!(a.category(), cycle.category());
        // 旧 wire 码仅投影保留，可诊断但不产生资格。
        let unknown = unknown_failure().kind;
        assert_eq!(unknown.stable_code(), "legacy_wire_0414");
        assert_eq!(unknown.category().as_str(), "unknown");
    }

    #[test]
    fn sg_f1_1_typed_disposition_unknown_in_batch_blocks_whole_set() {
        // 整批不自动：即使有事实完整的 heading，含 Unknown 也不 L1。
        let set = set_of(vec![heading_failure(414), unknown_failure()]);
        let disposition = classify_failure_set(&set, &active_sc_facts());
        assert_ne!(disposition, PlanningRecoveryDisposition::StructuralAuthor);
        assert_eq!(disposition, PlanningRecoveryDisposition::UnknownStop);
    }

    #[test]
    fn sg_f1_1_typed_disposition_operational_in_batch_waits_whole_set() {
        let operational = PlanningFailure {
            kind: PlanningFailureKind::Operational {
                stable_code: "projection_context_drift".to_string(),
                cause: "compile projection 与当前 context digest 不一致".to_string(),
            },
            origin: PlanningFailureOrigin::Projection,
            stage: PlanningStage::Plan,
            diagnostic_ref: "diag-9d05".to_string(),
            source_ref: "plan-source/rev-31".to_string(),
            source_hash: "9f1c0a4d8b7e6253c0af1d2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2"
                .to_string(),
            evidence: PlanningFailureEvidence::default(),
        };
        let set = set_of(vec![heading_failure(414), operational]);
        let disposition = classify_failure_set(&set, &active_sc_facts());
        assert_ne!(disposition, PlanningRecoveryDisposition::StructuralAuthor);
        assert_eq!(disposition, PlanningRecoveryDisposition::OperationalWait);
    }

    #[test]
    fn sg_f1_1_typed_disposition_missing_active_authorization_waits_whole_set() {
        let set = set_of(vec![heading_failure(414)]);
        // 同一失败集合、同一 source_hash/context_digest，仅授权状态翻转：
        // 分类读的是授权事实（active/anchor/launch），不是诊断文案。
        let inactive = StructuralAuthorizationFacts {
            active: false,
            ..active_sc_facts()
        };
        assert_eq!(
            classify_failure_set(&set, &inactive),
            PlanningRecoveryDisposition::OperationalWait
        );
        let anchor_waiting = StructuralAuthorizationFacts {
            anchor_waiting: true,
            ..active_sc_facts()
        };
        assert_eq!(
            classify_failure_set(&set, &anchor_waiting),
            PlanningRecoveryDisposition::OperationalWait
        );
        let launch_unknown = StructuralAuthorizationFacts {
            launch_unknown: true,
            ..active_sc_facts()
        };
        assert_eq!(
            classify_failure_set(&set, &launch_unknown),
            PlanningRecoveryDisposition::OperationalWait
        );
        // 活动授权 + 剩余 attempt 的对照：同一集合走 L1。
        assert_eq!(
            classify_failure_set(&set, &active_sc_facts()),
            PlanningRecoveryDisposition::StructuralAuthor
        );
    }

    #[test]
    fn sg_f1_1_typed_disposition_attempt_exhausted_typed_stop() {
        let set = set_of(vec![heading_failure(414)]);
        // SC 同 turn 物理上限 2 已全部消费：typed 系统故障停止面，不偷第 3 次。
        let exhausted = StructuralAuthorizationFacts {
            attempt_no: 2,
            max_attempts: 2,
            ..active_sc_facts()
        };
        let disposition = classify_failure_set(&set, &exhausted);
        assert_ne!(disposition, PlanningRecoveryDisposition::StructuralAuthor);
        assert_eq!(disposition, PlanningRecoveryDisposition::UnknownStop);
    }

    #[test]
    fn sg_f1_1_typed_disposition_multi_solution_cycle_complete_facts_is_l1() {
        // R1：canonical 边与仓序自动边互相等待。解不唯一（归属重组 / 历史
        // 恢复两个候选都合法），但事实完整 → L1，由 author 择解，不问用户。
        let set = set_of(vec![r1_cycle_failure()]);
        let disposition = classify_failure_set(&set, &active_sc_facts());
        assert_eq!(disposition, PlanningRecoveryDisposition::StructuralAuthor);
        // 没有业务矛盾证据不造 L3；也不走 L2 技术路由。
        assert_ne!(disposition, PlanningRecoveryDisposition::BusinessGate);
        assert_ne!(disposition, PlanningRecoveryDisposition::TechnicalRoute);
        // witness 事实完整：闭合、每跳有来源。
        let PlanningFailureKind::DependencyCycle { witness } = &set.failures[0].kind else {
            panic!("fixture 必须是 DependencyCycle");
        };
        assert!(is_complete_cycle_witness(witness));
        assert_eq!(
            set.failures[0].evidence.affected_ids,
            vec!["WI-004".to_string(), "WI-005".to_string()]
        );
    }

    #[test]
    fn sg_f1_1_typed_disposition_incomplete_facts_fail_closed() {
        let facts = active_sc_facts();
        // 空 witness：Kahn 残余节点列表不是 witness。
        let empty_witness = set_of(vec![PlanningFailure {
            kind: PlanningFailureKind::DependencyCycle { witness: vec![] },
            ..r1_cycle_failure()
        }]);
        assert_eq!(
            classify_failure_set(&empty_witness, &facts),
            PlanningRecoveryDisposition::UnknownStop
        );
        // 断链 witness（WI-004→WI-005 后接 WI-007→WI-004）。
        let broken = set_of(vec![PlanningFailure {
            kind: PlanningFailureKind::DependencyCycle {
                witness: vec![
                    PlanningCycleWitnessHop {
                        from: "WI-004".to_string(),
                        to: "WI-005".to_string(),
                        sources: vec![PlanningEdgeSource::CanonicalInput],
                    },
                    PlanningCycleWitnessHop {
                        from: "WI-007".to_string(),
                        to: "WI-004".to_string(),
                        sources: vec![PlanningEdgeSource::ChangeOrder],
                    },
                ],
            },
            ..r1_cycle_failure()
        }]);
        assert_eq!(
            classify_failure_set(&broken, &facts),
            PlanningRecoveryDisposition::UnknownStop
        );
        // 无来源跳：无法核验边来源。
        let unsourced = set_of(vec![PlanningFailure {
            kind: PlanningFailureKind::DependencyCycle {
                witness: vec![PlanningCycleWitnessHop {
                    from: "WI-004".to_string(),
                    to: "WI-004".to_string(),
                    sources: vec![],
                }],
            },
            ..r1_cycle_failure()
        }]);
        assert_eq!(
            classify_failure_set(&unsourced, &facts),
            PlanningRecoveryDisposition::UnknownStop
        );
        // heading 缺 example：无法生成精确指令，同样整批不自动。
        let mut no_example = heading_failure(414);
        let PlanningFailureKind::InvalidWorkItemHeading { example, .. } = &mut no_example.kind
        else {
            panic!("fixture 必须是 InvalidWorkItemHeading");
        };
        *example = String::new();
        let set = set_of(vec![no_example]);
        assert_eq!(
            classify_failure_set(&set, &facts),
            PlanningRecoveryDisposition::UnknownStop
        );
    }

    #[test]
    fn sg_f1_1_typed_disposition_empty_set_keeps_existing_route() {
        let set = PlanningFailureSet::default();
        assert!(set.is_empty());
        // 无结构失败：不造 L1/L3，沿既有技术/评审路由。
        assert_eq!(
            classify_failure_set(&set, &active_sc_facts()),
            PlanningRecoveryDisposition::TechnicalRoute
        );
    }

    #[test]
    fn sg_f1_1_typed_disposition_failure_set_roundtrip_preserves_payload() {
        // 耐久证据：整集 serde 往返保 kind 判别、payload、refs 与关联 ID。
        let operational = PlanningFailure {
            kind: PlanningFailureKind::Operational {
                stable_code: "projection_context_drift".to_string(),
                cause: "compile projection 与当前 context digest 不一致".to_string(),
            },
            origin: PlanningFailureOrigin::Projection,
            stage: PlanningStage::Plan,
            diagnostic_ref: "diag-9d05".to_string(),
            source_ref: "plan-source/rev-31".to_string(),
            source_hash: "9f1c0a4d8b7e6253c0af1d2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2"
                .to_string(),
            evidence: PlanningFailureEvidence::default(),
        };
        let set = set_of(vec![
            heading_failure(414),
            r1_cycle_failure(),
            unknown_failure(),
            operational,
        ]);
        let json = serde_json::to_string(&set).expect("serialize failure set");
        let back: PlanningFailureSet =
            serde_json::from_str(&json).expect("deserialize failure set");
        assert_eq!(set, back);
        assert_eq!(back.failures[2].kind.stable_code(), "legacy_wire_0414");
    }
}
