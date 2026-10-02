# Design

## Context

动机见 [proposal.md](proposal.md)。本 change 只补缺陷 #8 的最终政策事实；执行仍受 `lc-root-initialization` 的 BOOT-03/BOOT-04 五步与自举凭据契约约束。

已核验的现状如下：

| 边界 | 实际代码与事实 |
|---|---|
| 政策事实 | `src/product/logical_codebase/policy.rs::AggregatePolicyArtifact` 保存正文、identity、revision、digest；`compute_digest` 对原始 UTF-8 字节算 `sha256:<hex>`，不是 JSON 摘要。`bootstrap` 固定 revision 1 和 `BOOTSTRAP_POLICY_TEXT`；`with_revised_policy` 已提供 revision +1 与新 `policy_id`，store 拒绝不递增 revision。 |
| 事实存储 | `AggregatePolicyArtifactStore::for_lc` 经 `store.rs::lc_scope_root` 读取 LC 权威子树 `aggregate-policy.json`；manifest 的 UUID `logical_codebase_id` 用于 policy identity/locator，产品 LC record id 用于 scoped store，两者不能互换。 |
| 五步与命令 | `coordinator_lifecycle.inc.rs::advance_remaining/run_provider_turn` 保持五步；生产 `production_dependencies.inc.rs::GatewayFactoryProviderTurnDriver::run_turn` 包裹每步 command audit，末命令在 `OpenspecAndExamples`。 |
| 现有收口 | `production_dependencies.inc.rs::try_finalize_root_recipe_receipt` 在末命令成功后读取当前 policy digest 和 `root_rule_digest`，再调用 receipt store；未发布最终正文，失败仅 warn。 |
| 双摘要 | `root_recipe_receipt_types.inc.rs::ROOT_RULE_ENTRY_FILE` 钉定 `AGENTS.md`；`root_recipe_receipt_store.inc.rs::root_rule_digest` 只哈希该入口原字节。`RootRecipeReceiptStore::finalize` 要求四条命令 Allowed、统一 canonical root，并持久化 policy/rule 两个独立摘要。 |
| 就绪消费 | `bootstrap_projector.inc.rs::project_rules_policy_step` 已检查 policy 可解析、operation 完成、receipt/root/policy/rule digest 一致；当前桩加有效 receipt 仍会完成。本 change 只增加桩识别。 |
| 已有回归资产 | `bootstrap_tests.inc.rs::readiness_fixture/finalize_root_receipt/policy_rule_digest_drift_keeps_planning_not_ready` 已提供三源 fixture；其中默认正文仍为 bootstrap，真正文负例需独立准备。 |
| 已核实 E2E | `cadence/reports/2026-10-02_验收报告_LC根初始化E2E终局_v1.1.md` §3.1 记录 stageF fixture 按 `policy_id` 物化正文；v1.2 §4 保留 #8。真实生成的 `/tmp/aria-t35b-e2e/lc-root/AGENTS.md` 引用 `.claude/rules`，只复制入口不能成为包含规则全文的聚合政策。 |
| 存量重跑阻断 | `coordinator_preflight.inc.rs::DeterministicAggregatePreflightService::inspect` 调 `AggregateRootPreflight::validate`；`registration_preflight.inc.rs::reject_owned_root_files` 对已存在 AGENTS/CLAUDE/.aria 无条件拒绝。新 key 虽能受理，Completed 根会在 AggregatePreflight 失败；本 change 已获授权只补 receipt 证明的 recipe-side 重跑预检。 |

## Goals / Non-Goals

**Goals:**

- 从同一 canonical root 的真实 recipe 规则构造确定性、可直接读取的最终政策；正文文件、scoped artifact 与最终 receipt 使用同一 digest。
- 用当前五步末命令收尾完成发布，保留四命令、bootstrap credential、取消与显式恢复语义。
- 把存量桩 LC 投影为等待，经产品入口明确重跑并重冻结；真正文按既有就绪条件继续使用。

**Non-Goals:**

- 不修改其他 readiness 判定或 resolver/admission/gateway/envelope；不解锁新的 provider 或改变 target/cwd/写根。
- 不新增 recipe step、命令、政策编辑 API、持久化操作状态机，不复制成员规则或改变单仓初始化。
- 不修改历史台账为虚构的产品修复成功；真实实现与 E2E 通过后才能追加 #8 关闭证据。

## Decisions

### 1. 复用末命令的生产收口

采用 `GatewayFactoryProviderTurnDriver::run_turn` 的既有末命令收口：`RuleAndMcpConfig` 生成根规则/MCP，`OpenspecAndExamples` 完成规则示例后，先保存命令 4 的原有审计 receipt，再完成确定性政策发布和最终 receipt，全部在该步骤 return/checkpoint 之前。不在第二/三条命令后前移当前政策，不新增 provider turn；`AggregateInitializationCoordinator::run_provider_turn` 继续按原顺序 checkpoint/complete。

政策发布或最终 receipt 失败通过现有 `AggregateInitializationError::ProviderTurn` 返回，coordinator 将最后一步落为 durable Failed；移除这一新收口的 warn-only 吞错。`handlers.inc.rs::create_aggregate_initialization_for_lc` 成功后的第二次写 finalize 一并撤掉：已成功末步必有 receipt，不在 operation Completed 后补发或创建新 revision。索引仍是 handler 的独立后续操作，索引缺失的 BOOT-03 等待语义保留。

### 2. 双摘要及持久化位置

保持现有 `policy_id = policy/{project_id}/{manifest.logical_codebase_id}/{revision}`；根正文位置为 `canonical_provider_context_root.join(policy_id)`，没有扩展名。内部权威 artifact 仍为 `lc_scope_root/aggregate-policy.json`；receipt 仍为同一 scope 的 `aggregate-recipe-receipts/{operation_id}.json`。不把内部 JSON 文件暴露为 provider 的替代 locator。

`policy_digest` 是最终政策完整正文原字节 SHA-256；`rule_digest` 保持当前 `AGENTS.md` 原字节 SHA-256。二者通常不同，以同一 receipt 关联，不更改现有 rule digest 消费语义。

### 3. 根规则确定性聚合为政策正文

沿用实际生成布局与通用入口，不引入外部模型总结或任意 Markdown 链接解析器。正文来源固定为同一 canonical root 的 `AGENTS.md` 原文及 `.claude/rules/` 子树下全部常规 `.md` 规则原文（递归限于该规则目录）；`AGENTS.md` 首先，其他来源按规范相对路径字节序排列。`.claude/rules/language.md` 必须在场，目录/规则不可读、非法 UTF-8、空入口、symlink 或越界即返回发布错误。条件规则的 front matter 与正文均保留，不重新解释其适用范围。

组合格式固定为中文标题 `# 聚合政策`，随后每个来源用 `## 来源：<canonical 相对路径>` 分节，节内附源文件完整原字节（不 trim、不改换行、没有模型改写）；标题与分隔使用 LF。同一来源路径/字节集产生相同政策正文与 digest；policy_id、revision、时间和绝对 root 不写进正文，避免无规则变化时 digest 因发布元数据变化。发布前复验各来源字节摘要与文件清单，不能在读取过程中混入不同版本。

`CLAUDE.md`、`.agents/rules` 与 `.omp` 的 provider 兼容投影不重复拼接；MCP/settings、skills、`cadence/project-rules/examples/` 模板不成为政策正文。`AGENTS.md` 已有自定义规则引用保持原样，仍通过该入口的明确引用发现，不把未启用模板或任意文档自动激活。policy reader 当前仅校验 artifact identity/revision/digest 并返回 String（`repository_routing_policy_reader.inc.rs::read_policy_text_for_reference`），此格式不需要修改消费者。

**权衡：**推荐上述固定根规则来源，改动与现有布局对应且可测试；只复制 `AGENTS.md` 成本更低，但现场入口只列规则名称/引用，缺规则全文；通用链接递归展开成本与激活歧义更大，并会把模板或外部材料当政策，排除。本 change 不重新定义项目自定义规则的启用机制。

### 4. revision、幂等与同 operation 的发布输出

沿用 `with_revised_policy` 和 store successor 校验：当前 revision 为 r 时，新实际执行的 recipe 发布 r+1；首次通常为 bootstrap 1→最终 2。已为最终政策的 LC 再显式跑新 operation，正文相同也形成新 revision/新 locator，digest 可以保持相同。`ensure_bootstrap` 继续保留 identity 校验与已有 artifact 原样返回，不能将真正文降回桩；不删除其现有 gateway/migration 调用。

为避免两次收口或部分失败制造多个 revision，拟在既有 operation-owned 输出目录保存不可变的 `aggregate-initializations/{operation_id}/policy-publication.json`。它是该步骤的输出材料，不是新操作状态机，拟记录 operation/project/产品 LC record id、manifest UUID、canonical root、基础 artifact 三元引用、候选完整 artifact、来源路径/字节 digest 与 rule digest。首次写入后不可重新生成/覆盖；同 operation 重入读取该材料并逐项核验，复用候选 revision 与 created_at。不得放在 `staging/`，现有 cancel/recover 会清 staging（`AggregateInitializationOperationStore::staging_path`）。

同一 LC 的政策写入使用 scope 内排他锁，复用 `coding_attempt_store/locking.rs::with_exact_exclusive_lock` 模式；生产从 `tokio::task::spawn_blocking` 调用同步发布，避免文件锁/读盘占住异步 worker。`policy.rs` 的 save/ensure_bootstrap 与 recipe 发布采用同一锁，内部保存复用非重入写函数，不能发布者持一把锁而其他 writer 仍无锁。候选持久化与 current artifact 更新在同一锁内核验基础版本。当前 artifact 仍等于基础引用才保存候选；已精确等于本 operation 的候选则视为已发布并只复验；其他版本变化或来源变化返回可诊断冲突，不覆盖、不悄悄换候选。revision 溢出 fail-closed。新 output/锁路径是拟新增材料；现有 store 读取、identity/digest/successor 契约不变。

现有 POST 同 key 对终态 operation 会报 conflict（handler/store 已有行为），本 change 保留；“幂等”指不重跑、不新增 revision、不改旧 receipt，并非把终态 POST 改成 202。旧操作/receipt 可从原查询入口读取。

### 5. 原文落盘、receipt 与副作用边界

拟在现有 `policy.rs` 增加 scoped store 接口 `AggregatePolicyArtifactStore::publish_recipe_policy(manifest, operation_id, canonical_root, created_at) -> Result<AggregatePolicyArtifact, ProductStoreError>`（名称/接口为拟新增）：self 已以产品 LC record id `for_lc`，manifest 提供政策 UUID/root，operation 提供同 scope 输出归属。方法负责构造、不可变输出、root 正文及 current artifact，不签 recipe receipt；调用前复验该 operation 属于 self scope 且正在末步。生产收口直接用其返回 digest 调 `RootRecipeReceiptStore::finalize`，不能从 operation.input.policy_digest（现 handler 为占位串）取最终 digest；初始 operation input 保留原义，不顺手修订幂等输入契约。

发布顺序是：冻结上述 operation 输出→在 `root/policy_id` 以原字节写正文→read-back 对比正文及 digest→保存当前 scoped artifact→再用该 artifact digest 与当前 `root_rule_digest` 冻结 receipt→末步 checkpoint/Completed。在 receipt 前重新核验 root、来源摘要、artifact 与正文，不能只校验文件存在。以现有 `write_receipt_durable` 的临时文件、文件 fsync 与父目录 fsync 模式保证完整落盘；原文目标采用不覆盖已有目标的原子发布，目标已存在同字节可复用，不同字节/非普通文件必须冲突，禁止普通覆盖式 rename。identity/path 的校验在写入前完成，父目录或目标 symlink 拒绝。

发布只能新增本次 `policy/{project_id}/{manifest UUID}/{revision}` 文件和其目录，保留旧 revision 正文；不改 AGENTS、规则来源、成员仓或 Git metadata。当前 `ROOT_RECIPE_ALLOWLIST` 未含 `policy`，因此选在四条 provider 命令审计窗口全部关闭之后由确定性产品代码发布，使用上述精确路径 guard 和 operation 输出证据，不把 `policy/**` 泛化加入 provider 命令 allowlist、不扩大 provider 写权限。最终 receipt 的 policy digest 与该发布证据关联，四命令 summaries 仍精确为四条；不把发布伪称第五条 provider 命令。

中断窗口逐项可解释：正文先落而 artifact 未落，current 仍是原版本且最新 operation 未完成；artifact 已落而 receipt 未落，最新 operation 未完成或 receipt 缺失，readiness 仍 fail-closed；receipt 已落而末步 checkpoint 未完成，投影仍以 operation 状态等待。恢复仅走既有显式继续/新 key 重跑与不可变输出复验，不创建后台政策迁移或删除用户冲突文件。

末步的显式 Continue 还必须区分“provider 命令未成功”与“命令已审计成功、产品发布失败”：生产 driver 在后者读取同 operation 的既有 Allowed command receipt，校验 canonical root 与规则来源仍匹配其末条 after snapshot（本 operation 精确政策输出造成的新增文件另行按发布证据核验），只续做发布/receipt，不再启动命令 4、不追加时间不同的第二条同 index receipt。若既有审计材料或来源漂移则冲突停等；不能通过重新执行命令覆盖旧审计。最终 receipt 的 finalized_at 使用不可变发布输出的持久时间（或精确复用已存在 receipt），不能每次取 now 制造同 operation finalize 冲突。验收分别覆盖命令审计后、正文后、artifact 后、receipt 后的中断，成功续进不增加 revision 或 provider turn。

### 6. 唯一桩谓词与可操作等待

拟在 `AggregatePolicyArtifact` 内增加小型 `is_bootstrap_placeholder` 判断（名称为拟新增接口），只比较完整固定桩正文或其 store 已校验 digest，不按 revision、文件年龄、关键字子串或新 migration 标记猜测。`project_rules_policy_step` 仍先沿原路径检查 reference、operation 生命周期与 receipt/root/policy/rule 一致性，只有原判定将要返回 Completed 时追加这一 guard；这样既有 Running、Failed 和材料缺失/漂移 reason 不被桩遮盖，存量“桩+旧有效 receipt”才成为新增等待事实。

命中返回 `WaitingForHuman`、稳定 reason `aggregate_policy_bootstrap_placeholder`、`planning_ready=false`、`Retry`（可保留 Revalidate）和中文 detail。detail/notice 明示真实 API `POST /api/projects/{pid}/logical-codebases/{lcid}/initializations` 与新 `idempotency_key`，提醒这是显式外部 recipe 重跑；GET 自身仍零写入、零 provider 启动。

已定点核验：`bootstrap_service.inc.rs::dispatch_action` 将 RulesPolicy 动作归 registration batch，通用 Retry 不会执行根 recipe；`AggregateInitializationCard.tsx::canStart` 仅 null/failed/cancelled 可见，Completed 没有启动按钮。用户裁决保持 dispatcher/UI 零改动，Completed 存量桩迁移通过已存在的产品 API；`useIssueLifecycleWorkbenchActions.ts::handleStartAggregateInitialization`/`api/aggregate-initialization.ts::startAggregateInitialization` 证明同一 API 使用新 UUID key，新 key 对应新 operation。不能把通用 Retry action 的返回或不存在的 Completed 按钮当作迁移成功。

### 7. receipt 证明的已有根重跑预检

用户已授权的最小 producer-only 补充：`DeterministicAggregatePreflightService::inspect` 在首次未生成根时仍走原 `AggregateRootPreflight::validate`；显式新 operation 遇到现有根入口时，只从**本 LC scope** 的旧 Completed operations 枚举对应最终 receipt/per-command receipts，选取同 canonical root 且四条固定命令均 Allowed 的证明，不能借其他 LC/root 或仅靠文件名标识归属。

复用既有 `RootRecipeReceiptStore::get/list_commands` 与 command `after_snapshot.entries.content_digest`：最终 receipt 的 `rule_digest` 必须等于当前 `AGENTS.md` 原字节摘要，并与末条 command 的 after snapshot 中 AGENTS 摘要一致；当前 `CLAUDE.md` 若存在，必须是普通文件且摘要等于同一旧 snapshot 中该文件摘要。AGENTS/CLAUDE 不要求彼此字节相同，`rule_digest` 本就只代表 AGENTS；CLAUDE 只能由该 snapshot 提供独立归属证明。receipt 缺失、四命令不齐/Rejected、root/operation 身份不符、文件变更/新文件或 symlink 一律维持 ownership conflict，不自动覆盖。

在 `registration_preflight.inc.rs` 内拟加只供 recipe 使用的窄入口/共享核心，接收上述已核验的 AGENTS/CLAUDE 文件集合，仅豁免它们的“用户已有文件”检查；原公开 `validate` 调用仍一律拒绝用户文件。非 Git root、成员 canonical/越界/symlink/worktree、重叠 root 等检查全部照常执行；`.aria` 冲突不在本次豁免中，无归属证明不放行。不向首次登记传布尔 bypass，不复用旧 member projection 替代当前成员重核验，不改 trust/credential/admission。

**权衡：**推荐证据限定的 recipe 专用入口，补齐既定显式迁移而不松动首次登记；允许任意已有根或搬走/删除用户文件会违反 REG-09，排除；只复用旧 preflight checkpoint 不能支持 Completed→新 operation，也不能重核当前成员身份，排除。

### 8. 工作包、exclusive 文件与接口

以下是交接边界，不是精确实施 Plan；每包可独立提交并随包落测试/必要说明，单次派工不超过 4 小时。实施前由 `writing-plans` 展开步骤和命令，不在本轮执行。

| 工作包 | 独占修改文件（均已定位） | 契约与可验收结果 | 估时 |
|---|---|---|---|
| 1.1 政策构造/发布原语 | `src/product/logical_codebase/policy.rs`；`cadence/designs/2026-09-30_方案设计_LC根初始化_v1.0.md` 的后续变更说明 | 固定根规则聚合、完整原字节 digest、r+1、operation 不可变输出、精确 locator 发布与冲突/幂等测试；提供发布与桩识别接口，后续包不再编辑它。REQ-BOOT-05/REQ-ENV-12。 | 2.5–4h |
| 1.2 唯一桩 guard 与迁移说明 | `src/product/logical_codebase/bootstrap_projector.inc.rs`；`src/product/logical_codebase/bootstrap_tests.inc.rs`；本 change 的 `proposal.md` 迁移说明 | 桩+有效旧 receipt 红→绿，真正文负例 ready，原有 drift reason 与只读性保留，detail 含 API/new key，dispatcher/UI 不改。REQ-BOOT-06。 | 1–2h |
| 1.3 已有根重跑预检 | `src/product/logical_codebase/coordinator_preflight.inc.rs`；`src/product/logical_codebase/registration_preflight.inc.rs`；`src/product/logical_codebase/registration_tests.inc.rs`；`src/product/logical_codebase/coordinator_tests_profile_trust.inc.rs` | receipt 证明的 Completed 根通过；无证据/摘要不同仍 conflict；首次注册保持原拒绝，不复用旧成员投影。REQ-BOOT-06。 | 1–2h |
| 2.1 生产末步接线 | `src/web/handlers/aggregate_initialization/production_dependencies.inc.rs`；`handlers.inc.rs`；`tests.inc.rs`；`tests_parts/trust_route.inc.rs`（其余三项同目录） | 调 1.1 发布接口，receipt 只冻结最终 digest；失败末步 Failed，移除重复写收口；同包接线测试保持五步/四命令、取消和索引独立。REQ-BOOT-05/REQ-ENV-12。 | 2–4h |
| 3.1 跨包真实验收与台账收口 | `cadence/reports/2026-10-02_验收报告_LC根初始化E2E终局_v1.2.md` | 产品 API fresh-LC 与 Completed 桩显式重跑；真实 CLI 无 fixture 政策物化证据，记录 #8 修复验收并保持 #10/#13 口径。已有 HTTP/noop 测试不能冒充此真实验收。REQ-BOOT-05/06/REQ-ENV-12。 | 1.5–4h |

1.1、1.3 文件面独立；1.1 完成后，1.2 与 2.1 可并行。3.1 等全部实现汇合，由实施主会话统一验收。合计 **8–16 小时，按 1–2 人日**；四家 gateway 矩阵由并行网关 change 在同一产品政策事实之上验收，不重复计入本 change 的 provider 接入工作量。每包随包落定向测试与需要的说明，3.1 只负责跨包系统验收，不能留前包单测债务。

## Migration Plan

1. 发布修复版本后，旧 scoped policy/operation/receipt 继续可读，不做启动迁移。原满足就绪条件但正文为桩的 LC 被唯一 guard 投影为等待；非桩 LC 保持原判定。
2. 等待 detail/notice 指向现有初始化 API 与新 key。用户明确调用后，新 operation 经原五步与 trust/credential 前置条件执行；AggregatePreflight 先验证同 LC/root 旧 receipt 与 AGENTS/CLAUDE 字节归属，证明成立才能复用已生成根，其余根/成员检查不变。旧 Completed 不 reopen、不修改旧 receipt；无证据或漂移继续 fail-closed。
3. 最后一步发布新 revision 与根正文，冻结新 receipt。投影仍按最新 `updated_at` operation 及原索引事实判断，材料齐全才 ready；失败保留当前可诊断事实，后续再次显式处理。
4. 回退软件版本时保留新政策、正文与 receipt 数据，不降 revision、不删除用户文件、不把新正文替换为桩。旧版本是否读得懂新规则语义须由回退方评估；不得把旧版本重新允许桩 ready 当作本 change 的成功验收。

## Acceptance / TDD

| 验收面 | 红例/负例与通过标准 |
|---|---|
| 桩迁移谓词 | 先构造桩 artifact+旧有效 receipt+另外四步全部完成：当前代码会 ready，要求测试先红；新增 guard 后 waiting/Retry/API detail 且 planning_ready=false。真正文使用相同三源与 active index fixture，仍 ready；不把原 fixture 的所有 bootstrap 批量替成真正文来掩盖红例。 |
| 生产重跑预检 | 红例：Completed 桩 LC 含产品生成 AGENTS/CLAUDE，new key 在现代码撞 ownership conflict；绿例：同 LC/root 的四命令 Allowed receipt + 入口摘要相符允许 AggregatePreflight。负例：无 receipt、错 LC/root、Rejected 命令、AGENTS/CLAUDE 摘要不符/未知用户文件仍拒；首次注册原冲突测试保持。 |
| 构造与发布 | 入口只是引用时，artifact 中仍含 `.claude/rules` 原文；文件排序稳定，UTF-8/换行原样，digest 重算一致。缺 language/空入口/非法 UTF-8/symlink/路径冲突/保存失败不签最终 receipt；重入不涨 revision，new operation 增 revision 且旧文件/receipt 不变。 |
| 生产收口 | 前三条命令不前移当前 policy；末条成功后 root policy 文件、artifact、receipt 逐字节/摘要一致，五步最终完成。发布失败 coordinator durable Failed，不 warn-only Completed；不从 handler 常量取最终 digest。命令审计后/正文后/artifact 后/receipt 后的中断通过显式 Continue 复用同 operation 输出与审计，不重复 provider 命令、不新增 revision/finalize 时间冲突。 |
| 全新真实 LC E2E | 新工作区、非 Git 根及两个真实成员仓，从产品登记/初始化 API 执行真实 Claude Code recipe，policy/receipt/index 不 seed、禁调用旧 stageF::materializePolicy；四 receipts Allowed、最终正文非桩、原生读取 root/policy_id 成功、三方 SHA 一致、索引就绪后 bootstrap GET planning_ready=true。成员主 checkout HEAD/status 与前基线一致。 |
| 存量实际迁移 | 保留已核实的旧桩+有效 receipt LC（负面测试可用 fixture，正向迁移不直写产品 JSON）。GET 为等待；POST 新 key API 实跑；新 operation/revision/receipt 在场，旧证据字节不变，最终 ready。Completed 无 UI 按钮如实记录。 |
| 边界回归 | recipe 成功但 index 未齐仍 waiting；原 root/policy/rule drift、同 key conflict、GET 零副作用、取消/恢复、单仓命令/cwd/GitFinalize、成员零写入保持既有契约。 |

本轮验证只校验规划工件；上述 TDD、构建、单测与真实 E2E 由实施主会话在全部工作包完成后统一执行，不把历史 fixture 补齐报告冒充新产品验收。

## Risks / Trade-offs

- 多文件无法一次 rename 原子切换：采用正文文件→scoped artifact→最终 receipt 的先后顺序，receipt 是收口事实；任何中断保留证据并等待显式恢复，不伪造三者全成。
- 桩的旧有效 receipt 会在升级后进入等待：这是用户裁决的显式迁移，不自动重跑外部 provider。
- 根规则子文件不是现有 rule_digest 的独立门控输入：发布时完整内容已冻结进政策，发布后直接改 `.claude/rules` 不等于更新权威 policy；本 change 不扩大 readiness 子文件重哈希，用户应显式重跑重冻结，消费者仍使用冻结政策。
- Completed 态没有 UI 重跑按钮：按裁决提供真实 API 指引与迁移 E2E，不虚称该交互已修复；UI 扩展不在范围。
- 旧 receipt 缺 per-command snapshot 或根文件漂移时不能证明已有根：维持 ownership conflict，不自动把用户内容视为 recipe-owned。

## 自审与实施门禁

本设计已核对 policy 原字节摘要、manifest UUID/LC record id 区别、五步/四命令、双摘要 receipt、scope/output 路径、生产重复收口、终态 same-key conflict、Completed UI 不可见及重跑 ownership preflight。新增发布 API/不可变 output/锁/窄预检入口明确标为拟新增，不把新接口称为已存在；AGENTS/CLAUDE 的不同字节归属分别有证据。proposal、REQ-BOOT-05/06、REQ-ENV-12 与工作包对应；未声明执行产品测试或真实 E2E。

无改变范围/架构/验收的开放问题。规划工件经 OpenSpec validate/status 校验后供用户审阅；只有契约获批并形成已确认 `cadence/plans/` Plan 后才可进入 apply，尚未实现的 tasks 保持未勾选。
