## Context

见 proposal.md - Why。补充关键技术现状：

- `work_item_split_engine/prompts.rs`（785 行）：prompt 内联巨型 JSON schema + nonce sentinel 协议 + 约 15 条否定规则 + 转义规则 + 巨型 JSON 示例——复杂度大头是「格式对抗」与「行为教学重复」
- `workspace_engine/review/routing.rs:100-191,270-315`：review verdict 直接调用 `enter_review_decision`/`enter_human_confirm`/`enter_work_item_generation_mode`，reviewer 与编排双向耦合
- `workspace_engine/decisions.rs:557-650`：通用 `HumanConfirm::RequestChange` 按当前 artifact/review trust/node route 动态猜返修目标，同一协议动作承担多种领域语义
- 已有可复用资产：`work_item_split_validator`（三层机械校验）、`workspace_engine/compile.rs:208-370`（原子事务）、`artifact_constraints.rs:233-307`（markdown heading/token 确定性校验基础，可扩展为 source linter）、coding 安全门 `coding_workspace_engine/gates/schema_v2.rs:174-194`（消费 typed write_policy）
- 目标仓库均经 Cadence-skills 注入 Superpowers/OpenSpec/项目规则（`/pre-check`、`/rule-config`、`/mcp-configuration`），author 会话会真实读取项目规则（交接文档 `bdecb32d` 死锁事件实证）

## Goals / Non-Goals

**Goals:**

- 对外可见状态压缩为 prepare/generate/evaluate/approval/completed 五类（另加吸收态 failed）
- 人和 LLM 只接触 markdown/EARS；机器只消费确定性编译的 typed IR
- reviewer 降级为证据提供者；typed outcome（valid/repairable/human_required/fatal）由中央策略层裁决（策略层与终态矩阵在阶段 1 `workitem-typed-outcome-policy` 落地）
- must_fix 最多 1 次聚合自动返修 + 1 次限范围复评；相同 finding 指纹重现进人工门（交互）或 stopped_needs_human（auto）；repair/transition budget 烧完即停
- 全系统唯一人工门，对话流式交互（输入框为主，行内选项只做纯选择）
- 运行策略 session 创建时固定并持久化：交互手动批准 / campaign 显式 `auto_if_valid`
- 验收：codex+pi 各 1 案例到 Confirmed 2/2、单案例 ≤12 分钟、初评 ≤1 次 + 复评 ≤1 次（总 ≤2）、自动返修 ≤1 次、阶段 1 的 14 条 classifier golden（rep2/3/4 的 9 条、rep1 round-1 的 2 条 Advisory、3 条人工标注 Repairable 变体）全部分类正确

**Non-Goals:**

- coding 段 engine/WS 重构（独立变更，待本变更验证后另行评估）
- 替换现有 revision store / compile 事务 / lifecycle 记录格式（第一阶段经适配层复用）
- 事件溯源存储替换（可新增事件记录，不在本变更内替换现有存储）
- story/design 段流程变更（已 40/40、48/48 绿，不动）
- Cadence-skills 项目自身改造（出交接建议在该项目单独执行）

## Decisions

### D1: 编译器模型（C′）——markdown 为源，typed IR 为编译产物

`work-item-plan.md`（EARS 句式 + 稳定 section）是唯一可编辑源；确定性编译器单向产出一个顶层 `PlanCandidateIr { source_revision_hash, compiler_version, items: Vec<PlanCandidateItemIr> }`。每个 `PlanCandidateItemIr` 固定为 `{ target_repository_id, contract: CanonicalWorkItemContract, verification_plan: WorkItemDraftVerificationPlan, trusted_commands }`；依赖关系由 `contract.depends_on` 携带，沿用既有派生，不新造 DependencyGraph 类型。compile adapter 的 `logical_targets` 必须与 `compile_support.rs` 现状对齐为 `Option<BTreeMap<LogicalRepositoryId, String>>`。`CompileStores`、`PreparedInitialPlanCompile` 的字段和最终 ownership 保留为**实现时以 `compile.rs` 现状定义，计划不锁定字段**。**freshness 信任边界固定在 publish**：发布前校验顶层 hash/version + typed validator 全绿，写入不可变 publication provenance；coding 只消费已发布的 immutable runtime binding（coding 零改动）。类比 `.rs → LLVM IR`：不存在双向同步问题。

- **备选 A（保留 JSON contract 让模型直出）**：否决。私有深层 JSON 无训练先验；15 条否定规则 + nonce 协议即为对抗成本的实证。
- **备选 B（coding 直接消费 markdown）**：否决。`write_policy`、trusted commands 控制 coding 文件权限与命令执行，是纯文本 linter 无法替代的安全边界；且实测四次失败全是语义层，非 JSON 解析错误——换格式治不了语义病。

### D2: 中央策略层先于流程合并

第一刀先把「质量判断」与「状态跳转」拆开：reviewer verdict/findings 不再直接调用阶段跳转函数，统一产出 typed outcome 交策略层裁决。不先做这刀，合并阶段只是把旧循环搬进新函数。

- finding 指纹 = sha256(class + 归一化 message + contract_field)，带长度前缀结构化序列化；同指纹重现 → 交互模式进人工门 / auto 模式 stopped_needs_human（不自动再试，不标 fatal）
- repair budget：自动返修 ≤1 次；manual 返修预算独立（默认 3）
- transition budget：阶段转换总次数上限，防任何未预见环路

### D3: reviewer 降级 + prompt 重校准

reviewer 继续跑、findings 完整落盘，但输出仅为 evidence。reviewer prompt 改为：must_fix 仅限机械漏网硬错误与明确自相矛盾；每条 finding 附归类建议（repairable/human_required/advisory）；归类最终由策略层（确定性代码）裁决，模型 severity 不直接驱动跳转。

### D4: 唯一人工门的阶段边界（阶段 3 实现）

本 change 不实现对话流式人工门。阶段 3 才在聊天流呈现普通人工门消息（finding、证据、已尝试次数、指纹），使复杂反馈走底部输入框、批准/终止走消息内行内选项，并下线承载长文本的卡片式弹窗。阶段 2 仅复用阶段 1 已持久化的人工门快照和终态矩阵；campaign 模式到达人工门仍是 `stopped_needs_human` 终态加完整诊断落盘。

### D5: 运行策略持久化

session 创建时写入 `RunPolicy`：`interactive`（手动最终批准）或 `auto_if_valid`（机械校验零 Error + 无未决 human_required + 无重复指纹 → 直接发布）。不允许客户端运行中猜测/临时变更。

### D6: prompt 分层职责

- 仓库（Cadence-skills 注入）教「怎么工作」：行为纪律单一来源，aria prompt 删除 B 层
- aria prompt 只教「产出什么」：任务上下文 + markdown 语法 + 边界约束 + 判例 few-shot（rep2/3/4 的 9 个真实 findings 与 rep1 round-1 的 2 条 Advisory suggestion 均可作素材；其分类断言归阶段 1 的 14 条 classifier golden，不能据此推定为 compiler diagnostic）
- 编译器/validator 管「对不对」：机械正确性

### D7: 四刀到四阶段映射（含回退保护）

1. **第一刀／阶段 1 — 拆耦合**：中央 outcome/policy 层 + 指纹 + budget + 终态矩阵（`workitem-typed-outcome-policy`；新旧路径并存，feature flag）。
2. **第二刀／阶段 2 — 合并可见流程与编译适配**：单仓 C′ 单候选事务、markdown→IR、`InitialPlanCompileInput` 适配；第一版内部 provider 调用可不合并，防弱模型单次大输出失败（本 change）。
3. **第三刀／阶段 3 — 对话流人工门**：最小 WS 协议、对话输入与行内纯选择；本阶段不包含于当前 change。
4. **第四刀／阶段 4 — 删旧并扩展多仓**：仅在 campaign 2/2、恢复测试、golden 集全过后，删除 generation-mode WS 决策、outline/draft/batch 中间确认消息、review_decision 两套选项语法，并处理多仓范围。

多仓 legacy fallback 只允许处于新路径副作用前的确定性 preflight。一旦持久化 markdown、IR、session/run history、事务状态，或已启动任一 provider，失败必须以该路径的 durable fatal/recoverable 终态收敛；禁止静默切换 `flow_kind` 回落 legacy。

## Risks / Trade-offs

| 风险 | 缓解 |
|---|---|
| markdown 方言变成另一种私有 JSON（规则越多弱模型越翻车） | 少量稳定 section；ID/EARS 用简单行语法；关键安全信息不依赖表格列位置；编译错误返回行号+字段名+一个修复示例；仅 grammar/lowering finding 做 compiler diagnostic golden，阶段 1 的 14 条 classifier golden 仍作 prompt few-shot 素材 |
| markdown 与 IR 漂移 | 单向编译；source hash + compiler version 绑定；人工改 markdown 必产生新 revision；复用现有 contract/projection hash 完整性检查 |
| 换格式后语义成功率不升（rep2-4 是语义矛盾） | author prompt 真实反例 few-shot；reviewer 归 advisory；语义校验在 typed IR 上继续跑；codex/pi A/B 基线分别统计 parse success / semantic validation / Confirmed / 时长 / token |
| 复杂度只是搬进编译器 | 每新增一种 outcome 须证明无法归入现有四类；编译器 golden 测试先行 |
| 合并阶段后弱模型单次输出过大 | 第一刀不动内部调用次数；内部 outline→draft 拆分保留为实现细节 |

## 审核裁决落地（2026-08-26 双交叉审核 + oracle 裁决，用户批准）

- **D-A 终态矩阵**：见阶段 1 change 的 spec REQ-TOP-03。要点：human_required 是正常业务结果；auto 模式以 stopped_needs_human 终态落盘（不 Fatal、不空转）；人工反馈不消耗自动返修预算但受 manual 预算（默认 3）约束；人工返修后同指纹重现回到同一人工门
- **D-B review 两阶段**：初评 ≤1 + 返修后限范围复评 ≤1（总 ≤2）；复评只验证原指纹重现 + 重跑机械校验。`ReviewFinding` 不含可确定性校验的 path/region，因此本 change 不保留 changed-path 归因分支；验收指标为 `initial_review_count≤1` / `verification_review_count≤1` / `repairs_used≤1`（服务端持久计数）
- **D-C freshness 落点**：固定在 publish 边界；coding 只信 immutable binding，不加编译器专属 gate；若 runtime binding 完整性有缺口，另行独立通用 change，不做 markdown 专属逻辑
- **D-D grammar 完备度**：完整 lowering 是发布硬前提；字段四来源矩阵（markdown 明写 / session 上下文 / 编译器派生 / 事务生成）；handoff 只描述 schema，运行时值由 coding 后 HandoffRevision 产生，不新增平行的 handoff_expectations；结构化区域未知字段失败关闭，仅 Notes/Rationale 区容忍自由文本
- **D-E compile 接入**：仅使用 `InitialPlanCompileInput`（legacy 从 store 组装、新路径从 IR 组装，事务语义不变）；先补 legacy parity 测试（规范化产物、failpoint、恢复一致）再接新路径。`advance` 不是阶段 2 的 workitem pipeline 接口：其签名/实现归阶段 3 定义，本 change 不定义。多仓回落仅限新路径副作用前的确定性 preflight；副作用开始后失败必须 durable 收敛，禁止静默回落 legacy

## Open Questions

（无——D-A~D-E 五项决策已于 2026-08-26 经 oracle 裁决、用户批准。）

## 架构简化裁决（2026-08-29，用户批准：砍 outline 与受信目录）

实测驱动（6.2 验收 r5–r15 十轮）：outline 双会话使慢 provider 固定开销翻倍（pi 三次硬超时）；outline 派生目录链（catalogfix/gatefix/重复命令教学/空目录规则）七轮修补暴露其为 legacy 仪式搬迁。裁决：

1. **单次 provider 生成**：删除 outline 阶段（轻量 outline prompt/解析/计数/selector 前置），完整 plan 一次生成；selector 保留为编译后内部诊断（按 IR item 数+provider profile 记录，不改变 REQ-WSC-01 的「内部选择、不对外决策」语义）。
2. **命令声明即授权载体**：plan 的 Verification 段 `command` 字段=唯一声明处；`PlanCandidateItemIr.trusted_commands` 由该 item 声明确定性投影（去重；cwd="."；purpose 由 check 语句确定性截断派生；source_ref 由 source revision hash 派生）。删除 outline→目录→lowering 校验链与 `WorkItemPlanSourceContext.trusted_command_catalog`。授权锚=plan 审批门（人工/auto 策略批准 plan 即批准其命令集）；执行边界=coding 安全门（消费 trusted_commands，不变）。
3. **不动**：编译器语法/lowering 的结构校验、三层 validator 规则本身、确定性编译/发布/恢复链、review/人工门、flow_kind/preflight。
4. **已知取舍**：失去「命令事前清单」审计形式（改为审批门时人工直接看到 Verification 段）；同 item 重复命令的编译规则保留为卫生规则。

## reviewer 能力覆盖投影增补（2026-08-30，用户批准 D1=B3）

- **依据**：r23 三 provider 实跑暴露「reviewer pass → canonical validator fail」系统性错位（codex 11 / kimi 5 条 `required_capability_missing`，如 CT-001 未声明 `GET /api/levels`）；reviewer prompt 现状不含任何 capability 覆盖数据面（`reviewer_output_contract` 签名无 DependencyContractGraph/capability 参数）。
- **决策**：B3=B1+B2。B1（author 教学，`e78094a9` 已落地）修生成源头；B2（本节）向单候选 reviewer context 注入只读能力覆盖投影，数据复用 `report_contract_requirements` 同源逻辑（不重述、不重实现，消除口径漂移），reviewer 对覆盖缺口必报 must_fix（contract_gap）。
- **取舍**：接受 review 输入变大/变长的成本（预算按上报式上调纪律吸收）；不要求 reviewer 重跑整个 validator，只核能力覆盖；canonical validator 保持 fail-closed 原样，reviewer 为前置防线而非替代。
- **边界**：legacy/story/design reviewer 零触碰；scope/digest/CAS 机制零触碰。
- **遗留张力（6.4 裁量登记）**：REQ-WSC-07 退役门要求「单案例时长 ≤12 分钟」，而 pi 实测全链（生成+返修+复评）超过 12 分钟（D2 已批准 r24 pi 硬超时 1800s）。若 pi 以 >12 分钟 Confirmed，退役门该子项是否放宽/改口径，由 6.4 验收时用户裁决，不影响本节实施。

## P2+P4 裁决（2026-08-30，用户批准）

- **依据**：r24 实跑暴露两类残留——(a) reviewer 结构性盲区：单 item 投影看不到跨 item handoff 消费关系，codex 死于 `unconsumed_required_handoff`；(b) 2026-08-29「砍 outline 与受信目录」裁决的未完成收尾：`trusted_verification_command_catalog_*` 旧规则仍在 SC 编译路径拦截（错误串含 outline 字样）。
- **oracle 裁决**：P2+P4，排除 P3 九组全量 checklist——其中一半属「教了 provider 也可能无视」的局部字段规则，堆 2KB 文本反而增加弱模型不合规风险；P3 留作 P2 重跑仍多类违规时的第二阶段预案。
- **决策**：P4 先行独立提交（故障面分离、可回滚）；只清 SC IR 路径，legacy draft 路径与测试零变化。P2 = author handoff 消费闭环教学（预算 16,200→17,000，留 ~800 字节余量）+ reviewer 覆盖投影扩展（依赖图/消费闭环/跨 item scope，同源复用 `dependency.rs` 共享逻辑）；canonical validator 对外行为除退役残留外零改动。
- **pi 方差**：`unknown_requirement_ref` 是 provider 无视已有教学（教学与清单均已存在），不扩 prompt，按有界重跑消化；连续复现另立议题。`needs_human` 为合法终态，不降级不伪造。
- **95% 验收（方案 b，用户裁决）**：6.2 按现判据收敛；95% 成功率验收为全流程完成后的专项测量轮，不在本轮判据内。
