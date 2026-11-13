// P15（Plan §2.2，A4 冻结 wire 合同）：SC 失败证据／修复包共享 DTO 的逐字段
// TypeScript 镜像。权威源为 Rust `src/web/workspace_ws_types/sc_failure_repair.rs`
// （serde 全 snake_case），字段名逐字一致。镜像规则：`u32`/`u64` → number；
// `Option<T>` → `T | null`（键恒在，None 序列化为 null，无 skip 后门）；
// `Vec<T>` → T[]；serde 单元枚举 → snake_case 字符串字面量联合。
// 本文件只镜像合同，不复制 Rust 的结构校验算法；证据正文一律经
// ScEvidenceRef 引用，缺引用是显式 Missing，没有 None／空字符串后门。

/** Rust `product::models::ProviderName`（serde snake_case）的逐字面量镜像。 */
export type ProviderName =
  | "claude_code"
  | "codex"
  | "pi"
  | "kimi_code"
  | "fake";

/** 拥有失败包的链的 workspace 域身份。 */
export type ScFailureScope = {
  session_id: string;
  project_id: string;
  issue_id: string;
  plan_id: string;
};

/** 修复包所描述的 single-candidate 链的身份。 */
export type ScFailureChainIdentity = {
  scope: ScFailureScope;
  chain_id: string;
  business_key: string;
  original_node_id: string;
  original_role_run_seq: number;
};

/** 链内一轮（原轮或教学轮）的身份。 */
export type ScRoundIdentity = {
  chain_id: string;
  round_id: string;
  node_id: string;
  round_ordinal: number;
  provider: ProviderName;
  workspace_session_id: string;
  role_run_seq: number;
  run_ref: string;
};

/**
 * 严格证据库所持证据字节的引用。内部 tag 为 `status`：`available` 携带
 * locator/sha256/bytes（sha256 是正文精确 UTF-8 字节的 64 位小写 hex，
 * bytes 是该字节长度）；`missing` 携带显式 code/detail/round_id 三元组。
 * 刻意没有第三种「存在但为空」状态。
 */
export type ScEvidenceRef =
  | { status: "available"; locator: string; sha256: string; bytes: number }
  | { status: "missing"; code: string; detail: string; round_id: string };

/**
 * 包级 `missing_evidence` 列表的一项。locator 仅在库知道证据本应存在的
 * 位置时非 null；无已知 locator 的错误保持 null。
 */
export type ScMissingEvidence = {
  round_id: string;
  kind: string;
  code: string;
  detail: string;
  locator: string | null;
};

/** 三类终局 SC 失败。 */
export type ScFailureClass = "compile" | "ir_preflight" | "structure_gate";

/**
 * 教学 prompt 源证据的来源（Plan §2.2 在此提前定义）：failure-source
 * resolver 与共享 source-fence 渲染都消费它，两片因此可并行，不由后片补定义。
 */
export type ScTeachingSourceOrigin =
  | "compile_split_output"
  | "ir_preflight_split_output"
  | "structure_revision";

/** 轮的已观察启动状态；只反映真实观察，绝不从 RoundBound/split 序数推导。 */
export type ScRoundLaunchState = "not_started" | "started" | "unknown";

/**
 * 单条教学 prompt 的处置。`unavailable` 的具体理由在该 prompt 的
 * ScEvidenceRef Missing 载荷里；已 prepared 未发送保持 `prepared`，
 * 不冒充 `sent`。
 */
export type ScPromptDisposition =
  | "prepared"
  | "sent"
  | "rejected_budget"
  | "unavailable";

/** 一条教学 prompt 及其证据引用与处置。 */
export type ScTeachingPromptEvidence = {
  failed_round_id: string;
  target_round_ordinal: number;
  builder_id: string;
  prompt: ScEvidenceRef;
  disposition: ScPromptDisposition;
};

/** 已作出教学判定的投影；不授予任何资格。 */
export type ScTeachingDecision =
  | "not_requested"
  | "ineligible"
  | "denied"
  | "drift"
  | "granted"
  | "interrupted";

/** 失败链单轮的证据。 */
export type ScRoundEvidence = {
  identity: ScRoundIdentity;
  launch_state: ScRoundLaunchState;
  failure_class: ScFailureClass | null;
  blocking_reasons: string[];
  repair_instructions: string[];
  raw: ScEvidenceRef;
  source: ScEvidenceRef;
  existing_failure_id: string | null;
  attempt_key: string | null;
  prompts: ScTeachingPromptEvidence[];
  teaching_decision: ScTeachingDecision;
};

/** 包是否完整；必须与 missing_evidence 相符，否则包无效。 */
export type ScPackageCompleteness = "complete" | "incomplete";

/** 末次失败的源证据引用。 */
export type ScFailureSourceEvidence = {
  source_round_id: string;
  source: ScEvidenceRef;
};

/**
 * 完整自含的 SC 失败修复包（A4 payload）。terminal_class 是三类失败的
 * snake_case 之一或 `teaching_interrupted`；原失败原因始终留在
 * terminal_reason／rounds，绝不用 operational 措辞覆盖。
 */
export type ScFailureRepairPackage = {
  schema_version: number;
  package_id: string;
  chain_id: string;
  session_id: string;
  project_id: string;
  issue_id: string;
  plan_id: string;
  terminal_class: string;
  terminal_reason: string;
  completeness: ScPackageCompleteness;
  rounds: ScRoundEvidence[];
  last_provider_output: ScEvidenceRef;
  last_failure_source: ScFailureSourceEvidence;
  teaching_prompts: ScTeachingPromptEvidence[];
  evidence_log_ref: string;
  evidence_digest: string;
  evidence_through_entry_key: string;
  missing_evidence: ScMissingEvidence[];
};

/** 严格 publisher 成功发布包之后发出的回执。 */
export type ScArtifactPublishedReceipt = {
  schema_version: number;
  package_id: string;
  chain_id: string;
  node_id: string;
  package_digest: string;
  version: number;
};
