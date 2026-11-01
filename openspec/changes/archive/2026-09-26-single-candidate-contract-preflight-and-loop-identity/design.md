# Design: single-candidate-contract-preflight-and-loop-identity

> 依据：统一方案 `f51-53-unified-design.md` §5-C1（A/B/C 三节）；实证 `f51-diagnosis.md`/`f52-diagnosis.md`/`f56-diagnosis.md`；C0 已上线（canonical capabilities 可信）。

## Context

F-51：options 落库后校验上下文从 IR 反推（types.rs:78-86 无 options 字段）→ 三族校验不可触发；prompt 零教学。F-52：fingerprint=sha256(category, 自由文本)（fingerprint.rs:47-51）；reviewer 无前轮注入；Verification 未启用（9 轮全 Initial）；cycle key=reviewer node。F-56：AC 引用基线外路径无核对。TOP-04 契约已具备（Verification/scope/identity），本 change 使实现兑现契约并补齐 SC 形态与结构化 identity。

## Decisions

### D1 options threading（三生产路径显式）

`PlanCandidateValidationContext` 增存储 options 字段；三个构造点（workspace_engine/single_candidate.rs author / conversational_gate.rs revision / web run/single_candidate.rs 运行期）显式提供 plan record options；测试 fixture 显式构造（禁默认值丢意图）。三族校验复用 `WorkItemSplitValidator::validate` 语义（禁复制规则）；Error 适配 F5 mechanical ReviewVerdict 走既有 ingestion 回灌（contract_prerevision 先例）。

### D2 AC×基线树交叉核对

候选校验新增：解析 AC/verification_plan 中引用的仓库内路径（受限模式：反引号/引号内的 repo 相对路径、trusted_commands 与验证命令中的文件参数）；对 plan 的 fork base tree 逐一核对存在性。基线树来源=worktree fork base（与 coding 段基线一致）。不存在→Error finding（路径清单+三修复建议）。只核对显式文件路径形态，不做全文模糊匹配（防误报）。

### D3 identity 结构化（B 节口径）

机械 finding：确定性投影 canonical key。reviewer finding：受限提取 `WI-\d+|CT-\d+|AC-\d+`，排序剥下标规范化尾段。unstable 标记+fail-safe 人工。旧行为（自由文本）在 legacy schema 保留。fingerprint.rs 重写为 canonical 构造器；`seen_fingerprints` 存量不迁移（新身份在新 cycle 生效）。

### D4 前轮注入+Verification+cycle key

reviewer prompt 注入结构化前轮清单（canonical identity/category/required_action+当前 revision）。自动返修后以 `ReviewInvocationScope::Verification` 复评（original_fingerprints=本轮非 advisory 集）。`review_cycles` key 改 `sc:candidate:<source_revision_hash>`；同 revision 多节点同 cycle。cycle key 变化使既有运行中会话的旧 key 自然失效（按新 key 重建计数，不迁移——旧会话按 fail-safe 显示 unknown）。

### D5 prompt 教学（与校验口径逐字对齐）

options 镜像教学+AC 基线纪律进 author prompt（复用 C0/EARS 教学注入点 append_author_artifact_output_contract 系）；预算红线内净增，超线按 EarsImpl 方案 A 先例删语义冗余腾位。

## Risks / Trade-offs

- identity 重写影响判重存量：新身份与旧 seen_fingerprints 不匹配→首轮视为全新（可接受：旧身份本就不可信）；f52 实证 9 轮判重零触发，无有效存量可损失
- AC 路径核对的误报风险：只核对显式路径形态+Error 附修复建议（人工可裁决回灌）；fixture 用 F-56 真实形态（status.html 案例）
- Verification 启用后复评发现新 finding 走 VerificationNewFindings 人工路径——与 TOP-04 契约一致
- cycle key 变化对进行中会话：旧 key 计数遗留不迁移（显示 unknown），新轮次按新 key 正确累计

## Open Questions（实施首步）

- reviewer finding 受限提取的 field 尾段规范化规则细节（对齐 f52 诊断 §修法 L1 建议表）实施时以真实 9 轮 findings 做 golden 对照
