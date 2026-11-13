# 设计：SC 失败证据先落盘，再自含教学

> 状态：设计契约，未实施。命名为「新增」的接口／文件均为本 change 拟引入，不表示当前仓库已有实现。集成 owner 已批准本文两项窄例外（§2）。详细人读图解见 `cadence/designs/2026-11-13_技术方案_SC失败修复包与完整失败源自含教学_v1.0.md`。

## 1. 接地与边界

先读 `cadence/analysis-docs/wig-product-flow-2026-11-13/a4a3-grounding.md`，再用 CodeGraph 与磁盘定向阅读核验。当前路径均带 `src/`：

| 已核实的现状 | 源码落点与设计后果 |
|---|---|
| 主循环只有 `first_round_failure`，三终局出口无修复包 | `web/workspace_ws_handler/run/single_candidate.rs:830-916,939-1089`，`run_single_candidate_author`；必须新增逐轮证据，不从第一／最后摘要反推历史 |
| compile／IR 失败未到 source revision 写点 | `product/workspace_engine/single_candidate.rs` 的 `complete_single_candidate_work_item_plan_author`；`work_item_plan_source_store.rs:182,229` 的 put/get；compile／IR 必须以 split 原始输出为耐久输入，结构类按失败 ref 回读 |
| 原始输出是 JSON 字符串，split 身份有真实 handle | `work_item_split_engine/parse.rs:626-630,650-662,752-782`；**真实分区为 `work-item-split-runs/{session_id}/{role_run_seq}/`**，不是接地笔记中写的 `work-item-split-provider-runs` |
| 非 LC 作者当前没有 split 身份；Markdown close 吞错 | `web/.../run/gateway_start.rs:496-515,551-562`；只在 SC caller 新增严格包装，不改通用 legacy 语义 |
| 结构拒收返回值只有 prompt，无失败 ref | `product/workspace_engine/single_candidate.rs:37-41` 的 `SingleCandidateAuthorCompletion::Rejected { repair_prompt }`；`single_candidate_structure_failure.rs:348-367` 内 seal 后的 `failure` 才持有 source ref；必须将 sealed failure 随 Rejected 返回，不能在 runner 读 session 的 last-valid ref 猜测 |
| 教学模板无 source、返回裸 String | `product/workspace_engine/prompts/sc_compile_teaching.rs` 的 `TeachingReredriveFacts`／`render_teaching_reredrive_prompt`；`web/.../single_candidate_compile_preflight.rs:773-829`；需新增 source 入参和显式 Result 错误传播 |
| 最相近先例：完整 source 围栏＋硬预算 | `work_item_split_engine/prompts/sc_revision.rs:69-108`；常量 `prompts.rs:39` 为 262,144 B |
| 工件 API 不能证明耐久成功 | `workspace_engine/controls.rs:297-338` 的 `update_artifact` 返回 ArtifactRef；`session_state/timeline.rs:575-579` 吞保存错误，且广播早于外部回读；A4 不能先调用它再补查 |
| `failure_source_ref` 恒 None 且不消费 | `work_item_plan_policy/types.rs:469-478,628-648`；`web/.../single_candidate_structure_reredrive.rs:95-117`；需真值＋读端，不仅填字段 |
| 既有尝试行身份有独立统计语义 | `lifecycle_store/candidate_attempts.rs:184-224,285-315,627-667`；不能改 attempt_key／failure_id 或塞入另一套轮次计数冒充 provider attempt |

`ast-grep outline` 在当前内建命令不可用，已用 CodeGraph＋定向 read；CodeGraph 部分大函数行号与磁盘不一致时以磁盘为准。本文不对未读代码声称可行；新增接口须由后续 Plan 按本契约实现并编译验证。

## 2. 五项裁定、优先级与窄例外

| 事项 | 裁定与理由 |
|---|---|
| A3 超预算 | 最终教学 prompt `as_bytes().len() > WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES` 即拒绝；常量单源 262,144。不截断 source，不静默退成指针或依赖记忆。与 fresh revision 的 fail-closed 一致 |
| A4 载体 | **新增 ArtifactPayload::ScFailureRepairPackage**，沿 ArtifactVersion／ArtifactRef／ArtifactUpdate；逐轮日志只作底座，session 不复制大历史。工件与运行已有版本、重连读取路径一致 |
| 逐轮留存 | 新增 SC 专属追加证据日志，失败事实生成时、窗口授权与下一次启动前写入完整原因。`candidate-attempts.jsonl` 保持原格式／身份／A1 审计，证据以旧 attempt_key／failure_id 作可选关联，不改统计 |
| 原始输出轮次绑定 | 每次 SC 物理作者调用启动前以真实 split handle 写 RoundBound；交付后严格保存／回读 raw，再写 FailureObserved。绑定不靠 ledger 末位、目录最新项或教学 attempt 序号 |
| failure_source_ref | **填真实值并新增消费**。compile／IR 指向受控 split locator，结构类为既有 canonical source_revision_ref；装载并核验 hash。旧 None 仅历史读取兼容，不再作为新教学请求的正常值 |

优先级：**现有安全／准入与 typed 资格 > 来源身份与完整性 > A4 完整留存 > A3 最终硬预算 > 既有窗口 claim／真实启动分账 > provider 启动**。A4 必须先完成底座，A3 消费它；不能为获得 source 创造额外模型调用。

明确批准的窄例外：

1. **仅 SC** 原轮和教学轮统一分配 split run 身份，包含非 LC 直连作者；非 SC legacy caller 不改，原本没有 split 身份的 legacy 逐段调用仍不创建。已有 Logical legacy 审计保持原状，不以本 change 删除已有审计。
2. **仅A4**增加严格工件发布入口；普通工件仍用原业务入口，但允许最小集中协调：store唯一save锁内保A4、append/controls/publisher在同锁分配max+1、mark_latest跳A4、过滤下移最终公开视图。不整体迁移普通writer，不把旧update_artifact返回值当A4耐久证据；分配/save原子边界及失败传播须在Plan明确，不能用内存len回退。
3. 新增 A3 来源／总预算前置失败面；**不修改** REQ-WSC-32 的 parse 事实块 ≤32,000 B、整条省略与不新增失败面的既有义务。两者计量对象不同，A1 不被反向扩权。
4. 新增失败包既不打开人工门、不批准候选，也不触发重跑；last-valid 候选 refs 不被失败 source 替换。安全、资格与已有预算仍为权威。

## 3. A4 数据契约与逐轮写序

### 3.1 身份与引用

新增 `ScFailureChainId = sha256(长度前缀编码的 session_id + current_sc_author_business_key + 原轮 node_id + 原轮 role_run_seq)`。业务 key 缺失是显式 `sc_failure_chain_identity_missing`，不使用 `no-launch` 伪值。恢复读取使用日志内 identity，不重新用「最新 ledger」生成另一链。`round_ordinal` 从 1 起，对每次拟调用分配；RoundBound 不是 actual-start 证据，保留实际启动状态（未启动／已启动／Unknown），计费和调用上限仍只认既有真实 ledger。`round_id = chain_id + '/' + ordinal`；同一 round 重放必须复用原 split handle，不新分配计数。显式重新开始生成是新业务链；旧日志／旧包只读保留。

每个 source／raw／prompt 证据均为显式二选一：`available { locator, sha256, bytes }` 或 `missing { code, detail, round_id }`，不能裸 None。SHA-256 为文本 UTF-8 原始字节的 64 位小写 hex，bytes 同一字节序列的长度，不计 JSON 转义或围栏包装。

- split locator 新格式：`sc-split-output/v1/{session_id}/{role_run_seq}`；由 `WorkItemSplitProviderRunHandle` 构造并由 LifecycleStore 解析，不直接接受磁盘路径，不解析不透明 run_ref 猜路径。
- revision locator：既有 `project/.../source_revision/...` 原样保留，由 `get_source_revision(SourceStoreScope, ref)` 验证域和内容。
- prompt locator 新格式：`sc-teaching-prompt/v1/{session_id}/{chain_id}/{round_ordinal}/{sha256}`；定位同会话证据分区内不可变 UTF-8 prompt 文件，原文不放新增A4事件 detail 字符串。**原轮与实际发送的教学 prompt 既有执行事件保持完整 `output` 及 node detail 重连回放，不能因新增 locator 移除全文；不可变文件是机器核验权威，locator-only 仅约束新增 A4「修复包已发布」成功事件。** 超预算未发送 prompt 不冒充既有已发送事件。

### 3.2 新增耐久底座（不是终局工件的第二权威副本）

在 `workspace_timeline_root_for_session(session_id)` 下新增 `sc-failure-evidence/{chain_id}/rounds.jsonl` 与 `prompts/{round_ordinal}-{sha256}.txt`。复用 LifecycleStore 的 session 域校验、live-write 准入、文件锁和原子 JSON／不可变文本写样板；删除边界沿会话父目录收敛，不新建跨域裸路径。新日志严格读：坏行、序号断裂、跨 scope、同键不同内容均显式报错，不能沿用现有尝试列表的「坏行跳过」语义。

证据事件与独立发布回执（schema_version=1，确定性entry_key／package_id，重复同内容零膨胀；不同内容冲突拒绝）：

| 事件 | 必填事实／时机 |
|---|---|
| RoundBound | chain／round／ordinal／node／provider；handle 的 session、role_run_seq、run_ref；原轮与教学轮均在 provider start 前写入并回读 |
| FailureObserved | round／failure_class（compile、ir_preflight、structure_gate）；**完整** blocking_reasons 数组；失败 source 证据；本轮 raw 证据；existing_failure_id／attempt_key 如实可选；在任何 claim、终局返回前写入并回读 |
| PromptPrepared 或 PromptUnavailable | 引发教学的 failed round；不可变全文 ref＋digest＋bytes＋builder_id，或明确原因（无资格、来源不可读、超预算、窗口已用）；不以 builder 名当正文引用；构造成功但超预算的全文仍审计存档 |
| ChainTerminal | 最后失败轮／最后物理轮、终局原因、证据完整度；用于组包，不在工作日志中登记「包已发布」 |
| ArtifactPublished | 独立不可变发布回执（不是往坏日志继续append）：写于 `sc-failure-evidence/{chain_id}/published/{package_id}.json`，含package_id/version/node_id/包digest；文件与引用回读通过后写读。读取入口只呈现持有效回执且摘要匹配的新A4工件 |

若 raw 保存／回读失败，仍以当轮内存中真实失败事实写 `missing` 状态及 source 摘要，拒绝下一次启动，不以重新复制内存 source 来假装耐久来源已可读。若证据日志本身写失败，立即 fail-closed，尽可能以已读前缀＋当轮真实事实生成 `incomplete` 包；若底层存储也使组包失败，返回组合错误，禁止声称完整历史已保存。

### 3.3 全部 blocking_reasons 的来源（不能用截断的呈现面）

- **compile**：`format_compile_failure_reasons(&diagnostics)` 的全部 `code:line:message`，另附该轮未截断 `instruction_lines`；不使用已 `cap_autorepair_instruction_lines` 的 reasons 拼串作为全集。A1 五字段独立审计保持，不改原 detail 的 hash 输入。
- **IR 预检**：`PreflightVerdict.operational_failures` 后接 `failure_reasons` 的完整数组（`presentation_reasons()` 当前不截断）；typed faults 不解析 message，不扩大既有资格。无文字却有 typed fault时须由该 typed fault逐项稳定渲染，不能空数组冒充无阻断。
- **结构门**：从本次 `StructuralAssessment::Rejected(failures, _)` 的原始 `PlanningFailureSet` 全集逐项同源渲染（保留顺序与 code、受影响 ID、人话事实），不是 `structure_gate_first_round_failure()` 固定一句。接地修正：当前 `author_structure_gate_failure` 实际构造 `PlanningCompileFailureKind::Operational`，并非 `Structure`；直接改 kind 会改变 `compute_failure_id` 的 hash 输入。集成 owner 裁定保留既有 sealed `PlanningCompileFailure`、failure_id、detail 逐字不变，在 `Rejected { failure: PlanningCompileFailure, structure_failures: PlanningFailureSet }` 之外带原 typed set；由 `complete_single_candidate_work_item_plan_author` 的 Rejected 臂传入 `route_author_structure_gate_failure(..., structure_failures: PlanningFailureSet, ...)`，seal 后两者一并返回。旁路 set 仅供 A4 全原因与 A3 教学，不参与 seal、资格、计数、路由，不从渲染文本反解析；source ref 始终取 sealed failure。

同轮重复原因原样保留，跨轮同原因不去重、不只存首／末轮；包按 round_ordinal 顺序折叠日志。同一个失败可在既有 candidate attempts 和新证据日志各有一条不同用途的事实，**不**新增第二条候选尝试行。两者不要求跨文件原子事务：旧流水失败仍显式 `candidate_attempt_log_write_failed`，新证据失败阻止后续教学；包记录关联缺失而不捏造 attempt_seq。

### 3.4 修复包 v1

新增 wire 独立键 `sc_failure_repair_package`（兼容 untagged enum 判别），内含：

| 字段 | 语义 |
|---|---|
| schema_version / package_id / chain_id | 版本 1；package_id=sha256(确定性字段序列化，不含易变写入时间)，同一终局内容重放复用 artifact version |
| session_id / project_id / issue_id / plan_id | 真实归属，与读取 scope 必须一致 |
| terminal_class / terminal_reason / completeness | 三类候选终局拒绝，或已有失败后教学被 operational 中断；complete / incomplete＋缺项列表 |
| rounds[] | 每轮 identity、failure_class、完整 blocking_reasons、raw/source 证据、prompt 证据与 teaching decision；真实 provider 失败且无交付的轮仍在，原因／raw 标明不可得 |
| last_provider_output | **最后物理调用轮**的 raw 引用及 hash／bytes 或明确 missing；不偷偷取上一轮冒充 |
| last_failure_source | 最后一次被机械校验的失败 source 的 locator／sha256／bytes；若最后物理轮无交付，显式列 `source_round_id`，不宣称属于最后物理轮 |
| teaching_prompts[] | 所有已准备教学全文的 ref／digest／bytes／builder_id、触发失败轮与目标轮；未准备时显式状态；已发与被拒区分 |
| evidence_log_ref / evidence_digest / missing_evidence[] | 严格读到的日志前缀及其摘要；损坏／缺失完整列出，不靠「complete=true」掩盖 |

触发范围：SC generate 作者链三类机械失败终局（首次即不获权、Denied／Drift、重驱仍失败），以及**本链已经观察到机械失败后**教学构造／存储／provider 被中断。尚无候选交付的 admission waiting、健康败者 Superseded、用户取消且无候选失败、成功 Candidate、`Routed` 不新建终局包。失败后取消可组包但不改变原取消状态。启动失败的拟调用轮保留 `not_started`，不得冒充物理轮；`last_provider_output` 只取真实已启动的最后一轮，未启动轮的缺项另列 rounds。实际启动 Unknown 时不补造物理身份，按既有等待/对账，不自动组包覆盖等待态。

### 3.5 A4 严格持久化发布屏障（独立验收工作包）

新增 `WorkspaceEngine::publish_sc_failure_repair_package`，返回 `Result<ArtifactRef, ScFailureEvidenceError>`。同一 `artifact_versions.json` 仍是唯一版本权威；不新增永久staging、overlay、高位版本域或current状态机，不用旧update_artifact后补查。

1. 重放先按package_id读raw持久表、核包digest，同文复用原version/created_at，仅补缺步骤；异文拒绝。新包在同一个现有文件锁同步closure内读raw→max(raw.version)+1→追加 `is_current=false` 的A4→集中保存。不能先算号解锁再save，禁len()+1。
2. 唯一save在同锁内窄合并：磁盘已有而调用列表缺席的A4条目逐字保留；同version异payload／同package_id异digest拒绝，已有A4不允许普通writer改写。普通条目全以调用列表为准，不复活普通删除/回滚。已持锁内层save不重取锁；append/publisher/controls分配共用该closure，不跨await持同步锁。
3. A4拟写列表按write_json pretty口径写入；同锁内回读原bytes逐字相等，再核全部成员/引用/hash/bytes，随后释放锁。屏障测试expected用独立手写小JSON fixture而非生产serializer回声，另测确定性和字节损坏。文件屏障后才写读active node ArtifactRef和独立ArtifactPublished回执；同包重放核真实文件/回执，不只因“有回执”而返回。
4. 回执通过才同步raw内存versions、发ArtifactUpdate及新增A4成功定位事件；A4永非current，session.artifact保持最后普通current。mark_latest reviewed/confirmed只跳A4、保留原最后非A4选择语义。既有原轮/实际发送教学prompt全文output不变。
5. 任一步失败保留原机械失败并附sc_failure_package_persist_failed/step；文件已核验但ref/receipt失败是publication_incomplete、无成功广播、无新增provider。两store公读保持raw，engine恢复/内部read-modify-write继续raw；过滤仅在session_state与公开REST版本视图，live ArtifactUpdate只在publisher成功后发送。无receipt不呈现成功，坏receipt/digest显式错；视图不回写文件，不把A4设current。
6. stale内存经update/mark_latest/finalizer/CAS回滚整表写入，集中save仍保A4；普通新版本同锁max+1，可有pending不可见导致的诚实空洞。同包恢复同version/created_at。广播失败不撤销磁盘事实、不重驱provider。

receipt不进入包evidence_digest；公开REST版本handler及WS session_state是最终视图屏障，socket::lc_recovery_source_stale内部判据保raw。最终普通错误边界已裁定：`update_artifact -> Result<ArtifactRef,String>`，锁/准入/raw/max溢出/merge/save失败先于engine内存修改上浮，无ref/成功ArtifactUpdate；13个生产String/typed Result入口显式处理，唯一void complete_assistant_message复用Error事件＋finish_active_run_with_failed_node后return，不能继续checkpoint/AuthorConfirm。PlanRepair桥接真实 `PlanRepairError::Store(ProductStoreError::Io(message))`，不造variant。原ref/broadcast best-effort不扩为全局strict；store直接save普通条目语义不变。7文件/14生产、12文件/19测试caller精确闭集见Plan P14c–f/t1–t4，不引入永久staging/overlay。

## 4. A3 完整失败 source 自含教学

### 4.1 装载与摘要

新增 SC 专用 source resolver，结果为 `{ text, failure_source_ref, sha256, bytes, origin }`，加载后用于 prompt 和窗口请求的是**同一份 text**：

| 失败类 | 权威装载步骤 | 核验 |
|---|---|---|
| compile | 从当轮 RoundBound 的 handle 严格读 run.json 与 structured_output.json，JSON 必须为 String；用 `prepare_author_delivery_for_compile(raw)` 既有路径确定性归一化＋去 preamble | raw hash／bytes 对 raw；失败 source hash／bytes 对归一化完整 source，必须与实际 compile_source 一致；不直接注 raw，不新建假 source revision |
| IR 预检 | 同 compile，绑定当前失败轮而非上一成功候选 | 同一 `compile_source` 摘要一致；不从 candidate attempts 的 None 回退 |
| 结构门 | 从 Rejected 携带的 sealed failure.source_revision_ref 调 `get_source_revision`，scope 为本 session 的 project／issue／plan | store 自带 scope／hash／content_hash 核验；再对 source 文本算 SHA-256／bytes，与本轮失败 source 一致；raw 仍独立保留当轮 split 引用 |

compile／IR 的 `failure_source_ref` 填 split locator，结构门填 canonical revision ref。它不参与旧 frozen digest 的格式扩写：内容权威仍为现有 `failure_source_sha256`；但在**首次请求和重放、claim／启动前**都通过 resolver 读取 ref 并验证同一 SHA-256／bytes。换 ref 即使同内容也须按当前 round scope 拒绝跨轮替换；不得只填字段却无读端。`ScTeachingWindowRequest.prompt_ref` 对新窗口改填真正可读的 prompt locator（builder 名单列 `builder_id`），`prompt_digest` 对同一正文计算；现有 verify_frozen_inputs 的逐字比较继续约束该locator，历史窗口不重写。历史 `None` 可反序列化，但旧活动教学若缺绑定则 `sc_failure_source_unavailable`，零新增启动，不猜“最新输出”补齐。

字节域明确：compile／IR 的source locator指向raw split输出，但source的sha256/bytes指向 `prepare_author_delivery_for_compile(raw).source`，故必须“回读本轮raw→核raw摘要→同源归一化/去preamble→核source摘要”。raw与source摘要可不同，不能把source摘要直接对raw比较。结构revision的source摘要直接对revision.source计算。

教学请求构造器由Plan冻结为 `sc_teaching_failure_request(binding, source: &ScResolvedTeachingSource, failure_typed_kinds: Vec<PlanningFailureKind>, prompt: &ScTeachingPromptEvidence, instruction_digest: String) -> Result<ScTeachingWindowRequest, ScFailureEvidenceError>`；不改typed kinds和instruction_digest原语义。compile digest仍对未截断instruction_lines.join("\n")、IR对presentation_reasons().join("\n")、结构对原有不含source的修复指令正文；结构纯渲染体提为同文件helper供旧指令摘要与新完整模板共同调用，不复制词句、不让source注入暗改instruction_digest。

### 4.2 注入点与格式

- `TeachingReredriveFacts` 新增必填借用 `previous_source: &str`，以及只读来源元数据；`build_work_item_plan_compile_reredrive_prompt`／`render_teaching_reredrive_prompt` 改返回 Result，compile 与 IR 两个 wrapper 全部迁移，不留无 source 的生产旁路。
- 结构门的 `author_structure_repair_prompt` 保留现有环 witness／两个修法／保护项／REVISION_TEACHING 单源；新增 previous_source 入参，使用同一个 source block renderer。读取由 runner 负责，不在纯模板里做 I/O；Rejected 返回 typed failure 使 runner 可构造最终 prompt。
- source 块在失败事实之后、最终完整输出指令之前。统一标签 `[previous_source]`，含 `source_ref`、`source_sha256`、`source_bytes`、origin。沿 `sc_revision.rs` 的 markdown 围栏模式，但**不调用 `.trim()`**：围栏内正文逐 UTF-8 字节保留，首尾空白也在摘要内。闭栏之前补的分隔换行属于包装，不算 source。
- 围栏字符使用反引号，长度 `max(3, source 内最长连续反引号数 + 1)`，语言标记 markdown；同长度闭栏。防 source 自含三反引号提前闭栏；不逃逸或删改正文。文案说明原文是待修数据、不是新增指令，不依赖 provider 历史会话，不要求工具额外读取。
- 来源失败时不构造一个省略 source 的“成功 prompt”；`PromptUnavailable` 明确轮次、locator／缺失原因，终局失败可观察。

新增装配契约：`ScPreparedTeachingPrompt { text, builder_id, source_ref, source_sha256, source_bytes, prompt_digest, prompt_bytes, budget_verdict }`。纯模板只负责完整渲染，最终 facade 在所有尾段追加后计算 bytes 与 `Allowed/RejectedBudget{required_bytes,max_bytes}`；调用方先不可变写读完整text，成功后对RejectedBudget返回稳定错误，对Allowed才构造窗口请求/claim。不得在内部模板的早期Err中丢失超预算全文。既有仅测试入口 `build_work_item_plan_ir_reredrive_prompt` 与三条 `single_candidate_ir_prompt_tests.rs` fixture同步传显式source，不保留无source的测试旁路误导生产契约。

结构返回类型必须同步去掉 `SingleCandidateAuthorCompletion` 现有 `Eq` derive（源码37行附近）：`PlanningCompileFailure` 目前只实现 PartialEq，不能在含typed failure后继续宣称Eq。保留Debug/Clone/PartialEq；调用点只调整Rejected模式，返回值不保留旧repair_prompt别名。纯模板Result用新增 `ScTeachingPromptError`，预算裁决留ScPreparedTeachingPrompt，避免三种各自吞Err的错误通道。

### 4.3 唯一最终预算检查

`WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES = 262_144` 单源在 `product/work_item_split_engine/prompts.rs:39`。不另定义数值相同但独立漂移的 SC MAX；若需要出口，仅再导出同一常量。计量对象为**最终交给 streaming input 的教学字符串**，包括 source 元数据／围栏／全文、typed 事实、A1 省略计数、合法 ID、IR 追加指令、保护项与最终输出句；是 UTF-8 字节，不是 char 数、source 字节或 token。

最终装配完成后、不可变全文审计保存后、窗口 claim 之前判定：`len <= 262_144` 可继续，`len > 262_144` 返回 `sc_teaching_prompt_budget_exceeded { required_bytes, max_bytes, round_id }`。IR wrapper 追加尾段后必须再次／最终检查，不能只检查内部 base。不得抬 23,500／32,000 等其它预算，不拿 owner 四条指令上限截 source。

超限全文仍有 prompt ref／digest／bytes，标 `rejected_budget`，不发送为 provider prompt、不 claim、不消耗真实启动或教学次数；本链失败进入 A4 包并显式显示拒绝原因。若正文审计写失败，报告 `sc_teaching_prompt_persist_failed`，同样不启动；任何失败都不静默改成截断、摘要或“请回忆上一轮”。

## 5. 容量账、拆分和所有权

以下为**设计估算**，不是已实施 diff；基线由本轮 `wc -l` 取得，所有新增文件标「新」。净正增必须在剩余容量内，顶格文件先腾挪后接线；不通过删注释／压行制造容量。文件一行 import 也记账。后续 Plan 应按实际行数校准，超估算须先调整任务分片而非突破门禁。

| 落点（均相对仓库） | 当前行 | 迁入／新增 | 迁出／删除 | 预计净增 → 目标 | 腾挪／责任 |
|---|---:|---:|---:|---|---|
| src/web/workspace_ws_handler/run/single_candidate.rs | 1199 | +60 接线 | -84 transport、至少-40终局消息装配 | **目标≤1180** | P01先搬81-164，P10将三段终局message match整体迁出再接线；硬线1200，禁止压行或加guard豁免 |
| src/web/workspace_ws_handler/run/single_candidate_structure_reredrive.rs | 865 | +35 来源／返回值接线 | -95 prompt wrappers／证据包装迁出 | **-60 →805** | A4 集成先腾挪，A3 后续串行接线 |
| src/web/workspace_ws_handler/run/single_candidate_compile_preflight.rs | 1180 | +12 全因出口 | -80 prompt 构造 wrapper／尾段迁出 | **-68 →1112** | 将约754-829段移至新 prompt facade；既有测试随接口最小更新 |
| src/web/workspace_ws_handler/run.rs | 221 | +6 模块登记 | 0 | **+6 →227** | 容量内 |
| src/web/workspace_ws_handler/run/sc_failure_transport.rs（新） | 0 | +240 含迁入驱动／SC严格split收口 | 0 | **+240 →240** | 不改 gateway_start 的通用 caller；新文件承接大文件职责 |
| src/web/workspace_ws_handler/run/sc_failure_evidence.rs（新） | 0 | +300 三类事实适配／终局集成 | 0 | **+300 →300** | driver 与 evidence 分离，<1200 |
| src/web/workspace_ws_handler/run/sc_teaching_source.rs（新） | 0 | +180 resolver／请求核验 | 0 | **+180 →180** | source来源集中、不复制业务解析 |
| src/web/workspace_ws_handler/run/sc_teaching_prompt.rs（新） | 0 | +160 含迁入 wrappers／最终硬闸 | 0 | **+160 →160** | 统一 facade；IR尾段后检查 |
| src/product/lifecycle_store/mod.rs | 57 | +1 | 0 | **+1 →58** | 注册证据模块 |
| src/product/lifecycle_store/sc_failure_evidence.rs（新） | 0 | +330 日志／prompt／split严格读 | 0 | **+330 →330** | 复用锁／准入／split dir，禁止复制全parse模块 |
| src/product/work_item_split_engine/parse.rs | 866 | +8 split目录受控 crate 可见访问 | 0 | **+8 →874** | 容量内；不迁移存储分区，不改变通用complete顺序 |
| src/product/lifecycle_store/workspace_timeline_artifacts.inc.rs | 242 | 约+90锁内merge/分配 | 0 | **目标≤400** | 两公读raw不变；唯一save保A4，append/publisher同锁max+1，详Plan P14 |
| src/web/workspace_ws_types/sc_failure_repair.rs（新） | 0 | +180 DTO／显式证据状态 | 0 | **+180 →180** | 独立类型文件；正文不进DTO |
| src/web/workspace_ws_types/artifact.rs | 235 | +6 enum及两个markdown匹配 | 0 | **+6 →241** | 独立 untagged key |
| src/web/workspace_ws_types/mod.rs | 30 | +2 | 0 | **+2 →32** | module＋export |
| src/product/workspace_engine/mod.rs | 244 | +5 | 0 | **+5 →249** | 严格发布／source块／summary helper模块与窄export，各片串行登记 |
| src/product/workspace_engine/sc_failure_artifacts.rs（新） | 0 | +240 组包／严格发布 | 0 | **+240 →240** | publisher拥有strict屏障，普通入口仅共享store协调，不复制业务路径 |
| src/product/workspace_engine/single_candidate.rs | 1046 | +5 typed Rejected字段 | -3旧String字段 | **约+2→1048** | Rejected携sealed failure＋原PlanningFailureSet；不改kind/hash，不猜last-valid |
| src/product/workspace_engine/single_candidate_structure_failure.rs | 386 | +20 source参数／typed返回 | -12 旧提前造prompt | **+8 →394** | 容量内 |
| src/product/workspace_engine/prompts/sc_compile_teaching.rs | 566 | +25 source／Result／文案 | -10 旧无source模板措辞 | **+15 →581** | 单测迁至独立新测试文件，不在此继续堆大测试块 |
| src/product/workspace_engine/sc_teaching_source_block.rs（新） | 0 | +80 共同围栏renderer | 0 | **+80 →80** | 两模板同源，不改fresh revision既有输出 |
| src/product/work_item_plan_policy/types.rs | 793 | +4 ref语义注释／必要核验接口 | -4 恒None注释 | **0 →793** | frozen digest 算法不变；IO核验在SC resolver |
| src/product/workspace_engine/prompts.rs | 1549 | +0 | 0 | **0 →1549** | 既有超1200文件不得增长；沿现有re-export改签名，新helper注册在engine/mod.rs |
| src/product/lifecycle_store/candidate_attempts.rs | 899 | 0 | 0 | **0 →899** | 数据与身份不改 |
| src/web/workspace_ws_handler/run/gateway_start.rs | 991 | 0 | 0 | **0 →991** | SC wrapper调用既有begin／complete／finalize；legacy行为不改 |
| src/product/workspace_engine/controls.rs / session_state/timeline.rs | 340 / 734 | 约+25 / +6 | 0 | **目标≤380 / ≤750** | 仅update原子分配保存／mark_latest跳A4；不迁移普通业务，详Plan P14b |

测试与前端完整消费闭集见 §7；以下补充逐落点行数账。既有接口fixture只最小同步；新增测试每文件≤400行，测试宿主不增加顶格 include 行。

| A4消费／测试落点 | 当前行 | 新增／迁出 → 净增 | 腾挪与目标 |
|---|---:|---|---|
| src/product/workspace_engine/session_state.rs | 1366 | 约+30视图/白名单，-97 summary函数 → **约-67** | 目标≤1300且比原1366净减，1457既有guard登记不改；不以祖父余量扩范围 |
| src/product/workspace_engine/sc_failure_artifact_summary.rs（新） | 0 | +115 → **+115** | 含迁入的全变体summary，新增A4分支；≤1200 |
| src/product/workspace_engine/review/tests_policy_routing.rs | 1200 | +1新match臂，-22 helper迁出 → **-21** | `artifact_variant` 迁新测试helper；目标1179 |
| src/product/workspace_engine/review/tests_policy_routing_artifact.rs（新） | 0 | +25 → **+25** | 测试helper由原文件path module接入；不提升1200上限 |
| web/src/api/types/work-item-plan.ts | 700 | +4 → **+4** | 新union/import，DTO正文放新文件，目标704 |
| web/src/api/types/sc-failure-repair.ts（新） | 0 | +140 → **+140** | A4 DTO镜像；由work-item-plan.ts再导出，不动总索引 |
| web/src/api/types/workspace.ts | 1198 | +5 alias/import，-40重复artifact联合 → **-35** | 用同文件命名wire alias收敛四处重复联合，保留原所有分支，目标1163 |
| web/src/hooks/workspace-ws-message-handler.ts | 1061 | +8 → **+8** | workItemPlanArtifactFromMessage识别新键，目标1069 |
| web/src/state/workspace-ws-store-helpers.ts | 655 | +12 → **+12** | normalizeWorkspaceArtifact和版本记录共用识别分支，目标667 |
| web/src/pages/ChatWorkspacePageParts.tsx | 387 | +7 → **+7** | normalizeWorkItemPlanArtifactResponse两种形态，目标394 |
| web/src/state/work-item-plan-artifact-summary.ts | 145 | +15 → **+15** | 专属摘要，不落findings兜底，目标160 |
| web/src/components/workspace/work-item-plan-projection-artifacts.ts | 46 | +2 → **+2** | 严格switch新增label，目标48 |
| web/src/components/workspace/WorkItemPlanArtifactPanel.tsx | 361 | +4 → **+4** | 失败包文案、default tab=json，目标365 |
| web/src/components/workspace/WorkItemPlanArtifactContent.tsx | 813 | 0 → **0** | 复用既有json tab，完整JSON只读；不新造概览组件 |
| src/product/workspace_engine/tests/single_candidate/author_structure_gate_reredrive.rs | 339 | +20 → **+20** | 更新Rejected接口并保留结构事实断言，目标359 |
| src/web/workspace_ws_handler/tests/single_candidate_teaching_window_fault_matrix.rs | 699 | +20 → **+20** | 新ref fixture与原计数断言，目标719 |
| src/web/workspace_ws_types/tests/artifact_version_roundtrip.rs | 200 | +35 → **+35** | 新variant往返，目标235 |
| src/product/workspace_engine/tests.rs | 80 | +2 → **+2** | 新A4 publisher测试模块，目标82 |
| src/web/workspace_ws_handler/tests.rs | 1167 | +2 → **+2** | 新SC failure矩阵/source测试模块，目标1169 |
| src/web/workspace_ws_types/tests.rs | 1154 | 0 → **0** | 沿现有roundtrip模块，不加宿主 |
| src/product/workspace_engine/tests/single_candidate.rs | 1551 | 0 → **0** | 只更新已include测试文件，不加新include |
| 新测试：engine/tests/sc_failure_artifacts.rs、handler/tests/sc_failure_package.rs / sc_teaching_source.rs、web/src/state/sc-failure-repair.test.ts | 0 | 各 +300（上界400） | 行为矩阵/故障注入/三个前端入口，独立新文件；对应module或vitest自动发现 |
| src/web/workspace_ws_handler/run/single_candidate_ir_prompt_tests.rs | 47 | +12 → **+12** | 同步仅测试builder显式source/Result，目标59，不迁移生产资格 |

## 6. 迁移、安全与失败矩阵

- 老 artifact／旧 candidate attempts／旧教学请求仍可反序列化；不回填旧 raw 轮次、不重算旧 failure_id、不迁移已归档产物。
- split／prompt locator 仅允许精确 session scope 与已绑定 round；拒绝 `..`、绝对路径、跨 project／issue／plan／session、未知版本、非法UTF-8或非字符串 structured_output；显式 `sc_failure_evidence_invalid`，不是拿空串继续。
- 原始输出／完整 prompt 属已有 provider 内容审计权限面，不对外新增任意文件读取工具、公开下载地址或跨 session API；artifact 只带受控引用，日志不复制 raw 大全文。
- `missing` 是可观察失败事实，不是教学回退：包可 `incomplete`，但任何 source 不可读／hash不符的教学都零启动。
- 写／回读失败不启动修复重试，组合错误保留原故障。若现有 terminal phase 持久化也失败，同错误明确列出；不能假称 Failed 已耐久保存。

## 7. 实施切片与验收映射

OpenSpec tasks 只列高层工作包，精确步骤留后续确认 Plan。推荐常规 ds-task／glm53-task／minimax-task，无 gated 高阶角色依赖；每片≤4h、独占文件≤3（module登记由单一集成片拥有），共享文件严格串行：

| 片 | 独占文件面（新文件标新） | 独立验收／估时 |
|---|---|---|
| C1 契约 | 本change四文件＋人读设计 | 两条OpenSpec严格验证；本次交付，仅文档 |
| A4-1 证据类型 | 新sc_failure_repair.rs、artifact.rs、types/mod.rs | 新旧wire往返、显式missing；2h |
| A4-2 存储底座 | 新lifecycle/sc_failure_evidence.rs、lifecycle/mod.rs、split/parse.rs | 同键幂等／冲突、raw准确读、日志完整且坏行报错；3h |
| A4-3 严格工件 | 新engine/sc_failure_artifacts.rs、engine/mod.rs、新测试sc_failure_artifacts.rs | 写→回读→引用→广播顺序；失败不谎报；3h |
| A4-4 transport抽取 | 新run/sc_failure_transport.rs、run/single_candidate.rs、run.rs | SC原轮／教学轮真实handle返回；非SC旧路径不变，主文件≤1200；3h |
| A4-5 结构事实透传 | engine/single_candidate.rs、single_candidate_structure_failure.rs、既有author_structure_gate_reredrive测试 | Rejected保留sealed failure／全部结构原因；2h |
| A4-6 失败集成 | 新run/sc_failure_evidence.rs、run/single_candidate.rs、run/structure_reredrive.rs | 三终局＋教学中断包；原attempt身份不变；3h |
| A3-1 来源装载 | 新run/sc_teaching_source.rs、policy/types.rs、新来源测试 | 三来源／hash／scope／缺项零启动；3h |
| A3-2 共同source块 | 新engine/sc_teaching_source_block.rs、engine/mod.rs、新块测试 | 原文无损、嵌套围栏、UTF-8边界；2h |
| A3-3 compile模板 | prompts/sc_compile_teaching.rs、新模板测试 | facts含source且输出完整；2h |
| A3-4 facade迁移 | 新run/sc_teaching_prompt.rs、compile_preflight.rs、run.rs | 最终IR尾段后预算，1180行文件净减；2h |
| A3-5 结构模板 | single_candidate_structure_failure.rs、既有结构教学测试 | source完整、保护项不丢、返回Result；2h |
| A3-6 caller收敛 | run/single_candidate.rs、structure_reredrive.rs、新接线测试 | ref有真实消费，所有生产callers无旁路，净负行；3h |
| V 门禁 | 独立新增矩阵／故障注入测试文件（每片≤3，注册在其对应测试入口） | 见spec每场景「判定测试」；每片≤3h |

已完成只读消费闭集核验：A4 必须更新 Rust `build_artifact_version_summary` 与 session_state artifact_versions 的projection白名单；前端必须更新 work-item-plan/workspace 两套wire类型、`workItemPlanArtifactFromMessage`、`normalizeWorkspaceArtifact`、`workItemPlanProjectionArtifactFromRecord`、`normalizeWorkItemPlanArtifactResponse`、summary和label严格switch。default tab设为现有json tab即可满足完整只读，不新增恢复动作。`ArtifactUpdateEntry.tsx`、`workspace-ws-store.ts`、`workspace-chat-rebuild.ts` 是泛型消费者，不需新分支；plan_repair同类识别明确不纳入本变体。

消费精确拆片以Plan为准：P14为store集中raw协调/receipt视图helper；P14b只REST handler＋timeline mark_latest；P14c–f为4个≤3文件生产Result迁移片，7文件14调用；P14t1–t4为4个≤3文件测试expect片，12文件19调用。store两读raw、socket内部判据不改；公开WS成功广播/session_state与REST均有屏障。唯一void provider_drive原Error/Failed节点收口、PlanRepair真实Store(Io)桥接，分配/save同步closure不跨await。P13/P16须验证mapping旧表覆盖以及普通更新/回滚交错后同包同版本恢复。

真实测试样板：`src/web/workspace_ws_handler/tests/single_candidate_teaching_window_fault_matrix.rs` 699行；`src/product/workspace_engine/tests/single_candidate/author_structure_gate_reredrive.rs` 339行（由1551行宿主现有include接入，不加include）；`src/web/workspace_ws_types/tests/artifact_version_roundtrip.rs` 200行。后续实现统一收口命令含 `cargo test --locked --test it_core large_file_guard`，门禁实现为 `tests/it_core/large_file_guard.rs`。

## 8. 当前交付的验证口径

本轮只能证明 OpenSpec 结构与主规格格式合法、文档关键裁定完整；后续 Rust／前端行为必须由对应确定性单测、store重建读回、驱动mock计数与故障注入验证。不得用本轮 `openspec validate` 绿色冒充运行时特性已实现；不承诺教学成功率或时间改善。
