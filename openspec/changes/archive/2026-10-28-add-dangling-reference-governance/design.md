# Design: add-dangling-reference-governance

> **r2 修订说明（2026-10-28）**：本设计经两位独立审查者裁定「需修」后修订。修订不改变 owner 已拍板的三层结构与任何硬边界，只把**证据契约收紧**、把**不可实现的口径换成可实现的**、把**未经核实的事实换成实测结论**。逐条处置表见技术方案 §12。
>
> **r3 修订说明（2026-10-28，闭环复审后）**：本轮只做**文本级互斥消解**——消掉规范之间互相打架的表述，**不改任何 owner 已拍板的边界**。见下方「r3 追加的决策」第 18–25 条。

## 决策记录（owner 已批准，2026-10-27）

1. 三层做，**顺序固定**：① 读侧容错（先做）→ ② 巡检（先 dry-run，只报不改）→ ③ 按类型自动销账（**r5：服务启动时自动跑一次**）。
2. **①层边界严守**：只对旁支/记账类引用容错；主记录缺失语义一律不变（`story_spec_not_found` 等既有错误码与文案逐字不动）；只吞「目标已删」这一种错，IO/解析/权限等真实错误照旧上抛。
3. **②层判据按字段名**判定引用目标实体类型（不是 ID 前缀），再核对目标是否存在；dry-run 阶段绝不删任何东西。
4. **③层按类型定规则，不搞一刀切**：墓碑永不删；自动化 intent（story/design/plan **有 L1 墓碑**且满足四道前置条件）删；评审窗口/快照（attempt 两路皆无**且**窗口四条件成立）删；实体身份台账条目（plan **有 L1 墓碑**）删；**未归类一律只报不删**。
5. **三道闸**：① **r5 自动模式**——服务启动时自动销账，**无人工确认令牌环节**，删除判据 = 白名单类型 + 证据达标；② 按类型定规则；③ 清理动作幂等 + 留痕（何时 / 因哪个实体被删 / 清了哪条）+ 清理后再巡检一次自证归零。
6. **留痕进已有删除记录机制，不另造**。
7. **硬约束**：不改既有删除语义与既有级联覆盖面；不改主记录缺失语义；不删墓碑；不删用户业务数据；不新增依赖，生产目录操作用 `std::fs`；不动 provider 适配层 / `ProviderToolPolicy` / gateway envelope / `StreamingProviderInput`；不碰行数红线文件。

### r2 追加的决策（源自两轮独立审查，未改变 owner 边界）

8. **L1 收紧为四条**：墓碑存在 **且** `phase == deleted` **且** `deleted_at` 有值 **且** 无 pending failure。`purging` 墓碑与带 pending failure 的墓碑**不算 L1** → 只报不删。
9. **销账证据契约统一**：「L1 可删；L2 仅对**证据等级表**中标 L2 的目标类型且**该表给出的专属可算条件**成立时可删；其余 L2 一律只报不删；无法定义上游的类型降级只报不删」。证据等级表是**销账白名单的单一事实源**（其**等级列已写进 spec REQ-DRG-02**）。
10. **attempt 目标拆两路核对**：coding attempt / 规划面 workspace session，**两路都不存在**才算悬空。理由：规划面窗口把 workspace session id 写在 attempt 位上，单路核对会**误删活窗口**。
11. **窗口 L2 条件改为「从窗口记录自身可算」的四条**：终态 + 无租约 + 两路目标皆无 + worktree 已不存在。原先的「所属 plan 不可达」在事实层算不出来（attempt 记录无 `plan_id`、窗口记录无 plan 引用、墓碑 manifest 无 `attempt_ids`）。
12. **intent 销账加四道前置条件**：enrollment 文件锁内删除 + **锁内重读三项**（enrollment 状态 / generation intent 相位 / L1 墓碑证据；「已终止」以 `terminated_at` 有值为准，启用态一律不删）+ 只认 L1 墓碑（session / 授权引用缺失不构成依据）+ generation intent 相位必须为「已认领且未派发」。原「两者并发不报错、幂等即可抵消竞争」的论证**删除**（对取代一侧不成立）；**幂等语义只用于重入，不得替代锁语义**。
13. **清单摘要 = findings 规范化序列化的摘要**（不含巡检时间与令牌行）。**r5 起该摘要仅用于审计关联**（核对两次巡检是否同一现场），**不再是删除凭据**；`--apply --confirm <token>` 入口与人工 CLI 入口一并删除（REQ-DRG-13 / 22）。
14. **巡检新增「不可判定」隔离**：「读取失败按未悬空」只约束 enrollment 重新授权判定；巡检侧读失败进独立的不可判定计数，不为 0 不得宣称归零。
15. **归零口径收窄**：只统计「销账白名单内且本次可删」的类型；「只报不删残留」单列一栏，不参与归零断言。
16. **留痕如实表述**：既有删除记录机制实为**每 issue 级逐实体墓碑**，无全局根，且语义/基数/覆盖三重不匹配 → 落点改为新建的全局 JSONL 台账，**行形态**沿用既有根级 JSONL 审计规范。落点归属**需 owner 拍板**。
17. **巡检清单落点改 `cadence/analysis-docs/`**（按 `rule://document-storage` 的「分析报告」分类；原 `cadence/reports/` 与规则分类表不一致）。

### r3 追加的决策（闭环复审后的**文本级互斥消解**，未改变 owner 边界）

18. **L2 的口径统一为「该类型在证据等级表中给出的专属可算条件」**——不再用统一的「唯一可达上游不可达」表述（该表述只在 `Plan` 一类上可解析）；证据等级表的**等级列已写进 spec REQ-DRG-02**，使 spec 与技术方案同源。
19. **「不可判定」拆成两类且不再重载**：「**不可判定（读取失败）**」只用于读失败；「**证据不足 / 删除未完成**」（墓碑进行中、墓碑缺完成时间、待人工处置的删除失败事实、无墓碑、上游无法定义、降级类型）归入清单的**只报不删残留**节。两类在 spec / 技术方案 / proposal / 本文件 / tasks 五处口径一致。
20. **REQ-DRG-19 的「无 L1 不删」加限定**为「无 L1 证据**且不满足该类型 L2 专属条件**」——否则与「窗口簇按 L2 删」自相矛盾（窗口天生无 L1）。
21. **幂等语义只用于重入，不得替代锁语义**：删除 intent MUST 在 enrollment 文件锁内进行；「缺失视为已清」不得作为「锁外删除」的借口。
22. **留痕落点的表述去「既有」**：落在**状态根下单一全局追加式 JSONL 台账**，MUST NOT 写入 issue 级墓碑目录、MUST NOT 扩展 `DeletionEntityKind`；落点归属仍需 owner 拍板（U10）。
23. **attempt 存在性路二改为「枚举」**：窗口 / 快照记录不带 `project_id` / `issue_id`，故巡检遍历**全部 project × issue** 调 `list_workspace_sessions` 并在一次巡检内建 session id 集合；**禁用** `get_workspace_session`（读路径有写回副作用，破坏零副作用契约）与 `find_workspace_session_path`（`pub(super)`，新模块调不到，且 `workspace.rs` 零余量）；枚举失败一律归**不可判定**，MUST NOT 当成「不存在」。
24. **窗口簇删除粒度改为按 attempt 聚合**：既有清理器 `purge_for_attempts` 按 attempt id 批量删除该 attempt 名下**全部**窗口（含归档），故销账 MUST 先按 attempt 聚合，**该 attempt 的全部窗口都满足 L2 四条件才删**，任一不满足则一个都不删。
25. **摘要口径写死**：发现集合 = 本次判定为悬空的条目（**含「只报不删残留」条目本身**），排除巡检时间、令牌行、**不可判定条目**、**未归类条目**与各栏说明文字。

### r4 追加的决策（实施审查后，未改变 owner 边界）

26. **存在性判定是「三态」不是「两态」**：`Exists` / `Missing` / `Unknown`。**存在性数据源或所需作用域在本判定面不可用时 MUST 报 `Unknown`**（归「不可判定」，REQ-DRG-23），MUST NOT 报 `Missing`——否则该类型下**正常存在**的引用会被整批判为悬空（实测：member 记录的 `checkout_ids` 3/3 命中、aggregate-index 的 `member_snapshots[].checkout_id` 96/96 命中，技术方案 §2.4.4）。读取失败同样 MUST NOT 退化为 `Missing`，须原样上抛。
27. **U1 四类（checkout / aggregate_index / logical_member / enrollment）在 Task 2 判定面报 `Unknown`**：LC 三类记录落在 `logical-codebases/<lc_id>/`，读 API 已核实（`list_checkouts` / `load_checkout`、`list_members` / `load_member`、`AggregateIndexStore::get`、`IssueAutomationStore::get`）但**须先 `LogicalCodebaseStore::list(project_id)` 枚举 LC 再 `for_lc` 作用域化**；`LogicalCodebaseStore::new(paths)` 只落 legacy 根，直接读会「换个方式」继续误报。真实读属持有 LC 上下文的巡检（Task 4）。四类仍**只报不删**。
28. **「不可判定」定义扩为「读取失败 **或** 所需存在性数据源 / 作用域不可用」**；判为 `Unknown` 的条目**不计入悬空条目数、不计入归零、不进入摘要的发现集合**（REQ-DRG-17 / 22 / 23）。
29. **判据表末行（LC 三类）与实际记录形状的偏差 MUST 在巡检开工前核对**：单数 `checkout_id` / `repository_id` / `aggregate_index_id` ≠ 实际的 `checkout_ids[]` / `member_snapshots[].checkout_id` / `LogicalCodebaseManifest.active_aggregate_index_id`；`physical_repository_id` **不是** LC 成员 id（技术方案 §3.3.2 的偏差登记表）。

### r5 追加的决策（owner 拍板后，未改变任何删除边界）

30. **销账改为自动模式**：巡检 + 销账**只在服务启动时自动跑一次**；**删除人工确认令牌环节**与**人工 CLI 入口**（`aria inspect dangling-references`）。删除判据 = **白名单类型 + 证据达标**（L1 墓碑 / W-1..W-4 全成立），MUST NOT 比 REQ-DRG-12 更宽。清单**仍然写出来**供事后查（REQ-DRG-11 / 13）。
31. **清单摘要降级为审计关联**：计算口径逐字保留（findings 规范化序列化摘要），但**不再是删除凭据**（REQ-DRG-22）。
32. **①层容错接入产品读路径**（REQ-DRG-24）：产品里真的读旁支记录并顺着 id 去查目标的路径接入「跳过该条目 + 记一条①层告警」；只跳过「**已确认目标不存在**」，读失败照旧上抛；主记录缺失语义与 fail-closed 读路径（N5–N12）不变。

## 架构

````
┌── 判定能力（新增模块 src/product/dangling_reference/）───────────────────┐
│  reference_table：记录类型 → (引用字段, 目标类型) 静态表（九类目标）      │
│  resolve：目标存在性核对（复用既有 store 读 API）+ L1/L2 证据分级        │
│  warn：单一稳定码 dangling_reference_target_missing + 结构化字段 + 去重  │
└───────────────────────────────────────────────────────────────────────┘
        ▲                    ▲                        ▲
        │                    │                        │
┌───────┴────────┐  ┌────────┴─────────┐  ┌───────────┴──────────┐
│ listing（①层）  │  │ patrol（②层）     │  │ cleanup（③层）        │
│ 四个容错列举    │  │ 五分区遍历 + 清单  │  │ 三道闸 + 白名单销账    │
│ 只读 API        │  │ 纯只读，无删除分支 │  │ 复用既有物理清理器      │
└───────┬────────┘  └────────┬─────────┘  └───────────┬──────────┘
        │                    │                        │
        │                    │                        ▼
        │                    │            ledger：删除语义目录族下的单一 JSONL
        │                    │            台账（新建分区；行形态沿用既有根级
        │                    │            JSONL 审计规范；cleanup_id 幂等去重）
        │                    ▼
        │            report：清单渲染 + findings 规范化摘要（r5：仅审计关联）
        │            + 不可判定 / 只报不删残留两节
        │
        └── 既有 fail-closed 读路径一律不接入（零改动）

provider 适配层 / ProviderToolPolicy / gateway envelope / StreamingProviderInput：零改动
````

## Context

动机与范围见本 change 的 proposal（Why / What Changes）；已确认的详细方案是 `cadence/designs/2026-10-27_技术方案_悬空引用治理与读侧容错_v1.0.md`，本文只固化架构边界与权衡，不重新裁定 owner 决策。

接地基线为 `.worktrees/feat-b-0808-add-monorepo`（分支 `feat-b-0808-add-monorepo`）。设计基线是 brief 指定的 HEAD `d2744682`；**本次核对时点该分支已前移到 `dd62e5db`**——在途 `FixAutomationEnrollmentDanglingSource` 已提交并合并，故本文对「在途工作」一律按**已落地**表述，file:line 均为 `dd62e5db` 时点的实测值。已读并采信的事实（file:line 已逐一核对）：

- 今天的真 bug 现场（**修复前形态**）：后端精确校验 `validate_confirmed_source_refs`（`src/web/handlers/automation_enrollment.rs:470`），story 不存在 → `:486-489`、story 未确认 → `:492-495`、版本不符 → `:498-501`、design 不存在 → `:514-517`、design 未确认 → `:520-523`、版本不符 → `:526-529`、design 未引用所选 story → `:536-539`；调用点 `:108`（绑定）、`:190`（换代绑定）、`:375`（enable）。前端正常路径用当前 card 的 source（`web/src/components/lifecycle/useIssueLifecycleGeneration.ts:412-417`），修复前的重新开启路径发旧 source（同文件 `:464`）——该处在途工作已改为按当前 card 组装（现工作树 `:490` 起）。
- enrollment 的「终止」语义**已存在**：删实体时终止绑定它的 enrollment（`src/web/handlers/lifecycle/deletion.rs:532` 的绑定判定 + `:569` 的 `terminate_automation_bound_to_entity`，调 `IssueAutomationStore::terminate_plan`，同 revision 命令 id 稳定、重放幂等、终止失败返回 `automation_termination_failed` 且删除不执行）。缺口不在「终止」，在「终止后文件仍在、其 `source` 仍指向已删实体」与「intent 记录根本没人管」。
- 级联删除**不覆盖** enrollment 与 intent：`grep -n 'automation\|enrollment' src/product/entity_deletion_cascade.rs` 仅命中 `PlanSupersedeReason::ReEnablePrepareIntentRotation`（`:61`、`:73`），无 enrollment 清理调用；`grep -n 'automation\|enrollment' src/product/lifecycle_store/plan.rs` 零命中。级联闭包本体在 `src/product/entity_deletion_cascade.rs:113`（`purge_entity_closure`）与 `src/product/lifecycle_store/plan.rs:1318`（`purge_entity_owned_children`）。
- intent / enrollment / reviewer 资源 / 台账的落点：enrollment `{issue_root}/automation-enrollment.json`（`src/product/issue_automation_store.rs:55-66`）；intent 由同目录 `with_file_name` 派生（`automation-plan-intent.json` 在 `:380`、`automation-generation-intent.json` 在 `:499`）；reviewer 窗口根常量 `REVIEWER_WINDOW_STORE_DIR`（`src/product/reviewer_window_store.rs:44`，`windows` 子目录 `:669-671`）；reviewer 快照根常量 `REVIEWER_SNAPSHOT_STORE_DIR`（`src/product/reviewer_resource_purge.rs:30`，另见 `src/product/coding_workspace_engine/code_review.rs:368`、`src/product/workspace_engine/review/drive.rs:1583`），manifest 路径 `<root>/windows/<window_id>/manifest.json`（`src/product/reviewer_snapshot.rs:432-438`）；台账 `<root>/state/entity-identity/sessions/<session_id>.json`（`src/product/entity_operation_store.rs:408-411`、`:623-626`）。
- 目标存在性核对可复用的既有只读 API：`list_story_specs`（`src/product/lifecycle_store/spec.rs:217`）、`list_design_specs`（`:325`）、`list_workspace_sessions`（`src/product/lifecycle_store/workspace.rs:556`）、`list_live_plan_ids`（`src/product/entity_deletion_cascade.rs:1164`，legacy + schema-v2 两路）、`get_attempt_by_id`（`src/product/coding_attempt_store/attempt.rs:401`）。
- **代码库里不存在「ID 前缀 → 实体类型」的集中注册表**：`DeletionEntityKind` 只有 `Story`/`Design`/`Plan`（`src/product/entity_deletion_store.rs:26-42`）；各前缀散落在分配点（`story_spec_` 在 `src/product/lifecycle_store/spec.rs:162-168`、`design_spec_` 在 `:277-283`、`issue_work_item_plan_` 在 `src/product/lifecycle_store/plan.rs:89-94`、`coding_attempt_` 在 `src/product/coding_attempt_store/attempt.rs:320-322`、`aggregate_index_` 在 `src/product/logical_codebase/aggregate_index/operation.rs:742-744`）；`enrollment_id` 是**裸 UUID 无前缀**（`src/product/issue_automation_store.rs:1403`）；`checkout` 用强类型 `RepositoryCheckoutId`。故判据必须按字段名。
- 墓碑机制：删除记录路径 `{issue lifecycle root}/entity-deletions/{kind}/{id}.json`（`src/product/entity_deletion_store.rs:518-529`），单条 JSON 只含 scope/kind/id/phase/time（`:94-102`），`purging→deleted` 同条幂等更新（`:226-296`）。
- reviewer 资源的物理删除面：`ReviewerResourcePurger::purge_for_attempts`（`src/product/reviewer_resource_purge.rs:53`）已按 attempt 归属覆盖活跃 + 归档窗口记录、树级租约、通道闸门、违规 journal、快照基线目录及其归档目录，「缺失=已清（幂等）」；其模块文档明示**读取错误 fail-closed，损坏即报错不当空跳过**（`:16-18`）。
- 既有 fail-closed 读路径（不得接入容错）：窗口租约列举（`src/product/reviewer_window_store.rs:1150`，消费点 `src/product/reviewer_window.rs:884-887`、`src/product/reviewer_window_store.rs:1070-1073`、`src/product/coding_workspace_engine/code_review.rs:918-921`）、窗口列举的解析失败跳过语义（`src/product/reviewer_window_store.rs:1160-1192`）、台账按 owner 反查（`src/product/entity_operation_store.rs:400`，绑定记录 `WorkspaceSessionIdentityBinding` = flatten 的 `EntityScope`，`:635-639`）、scope 准入（`entity_operation_store.rs:154`/`:175`/`:198`）、墓碑复核（`entity_operation_store.rs:162-168`、`:377-395`）。
- 行数红线（`tests/it_core/large_file_guard.rs`，硬线 `MAX_PRODUCT_CODE_LINES = 1200`，祖父条款逐文件锁定）：`src/product/reviewer_window_store.rs` 1921/1921（零余量）、`src/product/coding_workspace_engine/code_review.rs` 1817/1817（零余量）、`src/product/lifecycle_store/plan.rs` 1683/1683（零余量）、`src/product/workspace_engine/prompts.rs` 1665/1665、`src/product/workspace_engine/lifecycle.rs` 2167/2167、`src/product/lifecycle_store/workspace.rs` 1730/1730、`src/cross_cutting/streaming_provider/mod.rs` 2062/2062、`src/web/workspace_ws_handler/run/provider_run.rs` 1677/1678；`src/product/issue_automation_store.rs`（1858，登记上限 1491）、`src/web/handlers/automation_enrollment.rs`（2928，上限 2493）、`src/product/entity_deletion_cascade.rs`（1215，硬线 1200）在该工作树**已超有效上限**（HEAD 基线亦超，本次只做行数与登记表比对，**未运行门禁测试**）。有余量的胶水落点：`src/product/mod.rs`（61）、`src/product/app_paths.rs`（280）、`src/cli.rs`（354）、`src/product/entity_operation_store.rs`（1050）。
- 既有 CLI 形态：单一 bin `aria`（`Cargo.toml:10-12`），命令分派在 `src/cli.rs:36-93`（`daemon status` / `daemon run` / `repl` / `task run` / `web`），输出形态 `CliOutput::Text` 或 JSON。

> **行号时效说明**：本文 file:line 均为本次核对时点（2026-10-28，HEAD `dd62e5db`）的实测值。工作树此后若再有改动，相关行号会漂移——实施时对引用行号的引用应以**符号名**为准重新核对。

### r2 实测证据（2026-10-28，只读 live 状态 `/tmp/aria-deploy/.aria` + 源码）

> 全部为真实输出；未能实测者一律标「待实现时核实」，**不得**当作事实。完整证据见技术方案 §2.4。

| 事实 | 实测结论 | 影响 |
|---|---|---|
| 12 条窗口/快照的 `attempt_id` 形态 | **12/12 为 `coding_attempt_<32 位 hex>`**（6 窗口 + 6 快照 manifest，3 个不同 attempt、6 个不同 window）；**无一条** session 形态 | 现场风险为零，但代码层存在规划面形态 → 两路核对仍为必须 |
| 三个 attempt id 的存在性 | 在整个 `.aria` 树内出现在窗口/快照记录里，**另**出现在根级 `tool-policy-run-audit/<attempt_id>/` 分区（provider 侧工具策略审计；**不在巡检五分区内**，且不构成 attempt 本体记录） | 12 条确为悬空；「无 attempt 本体记录」的结论仍成立 |
| 窗口状态 | 6/6 为终态（5 × `closed_accepted`、1 × `closed_invalidated`） | 窗口 L2 条件 W-1 成立 |
| 租约 | `reviewer-windows/leases/` 下**只有** `domain.lock`，无租约记录 | 窗口 L2 条件 W-2 成立 |
| worktree | 6/6 的 `identity.canonical_worktree` 路径**已不存在** | 窗口 L2 条件 W-4 成立 |
| 墓碑 | **12 份**（story 5 / design 2 / plan 5），**全部** `phase == "deleted"` 且 `deleted_at` 非空；`entity-operations/pending-failures/` **不存在** | L1 四条件在现场全部满足；**无 `purging` 样本**（待实现时核实） |
| 台账条目 | **16 个** `.json`（`.bind.lock` 为伴生锁不计）；**仅 5 条**的 `entity_id` 有 L1 墓碑，**11 条无墓碑** | 台账销账**只认 L1** → 5 可删 / 11 只报不删 |
| intent 记录 | 2 个文件（plan intent + generation intent）；各含 `enrollment_id` / `plan_id` / `session_id` / `source.*`；`plan_id` 与 `session_id` 是**由 `prepare_intent_id` 预派生**的 id | intent 的「引用目标不存在」**不是**已删证据 |
| generation intent 的相位 | **`provider_dispatched`** | 依 I-4 → **只报不删**（丢防双跑检查点） |
| enrollment 记录 | **不存在**（只有伴生锁 `.automation-enrollment.json.lock`） | 锁内重读判定走「记录不存在」分支 |
| plan / session 存在性 | `issue-work-item-plans/` 与 `work-item-revisions/` **均为空**；`workspace_session_auto_510c9278-…` **只出现在两个 intent 文件里** | 均为悬空引用，但**证据等级**决定动作 |
| 既有追加式审计范式 | 根级 `tool-policy-run-audit/<session>/<seq>.jsonl`，每行 `{"schema_version":1,…}` | 销账留痕**行形态**沿用此规范 |

## Goals / Non-Goals

**目标：**交付 REQ-DRG-01–23：统一的「目标已删」判定能力（按字段名 + 九类目标 + L1 四条件 + 证据等级表 + attempt 两路核对）；四个旁支记录的容错列举只读 API（跳过 + 告警，主记录缺失语义与真实错误不变）；五分区只读巡检与可确认的清单（含不可判定隔离）；按类型白名单的幂等销账（intent 四道前置条件 + 窗口四条件 + 留痕 + 清理后自证归零）。三层分步交付，每步可单独验收与回滚。

**非目标：**不改既有删除语义与既有级联覆盖面；不改主记录缺失语义；不删墓碑；不删用户业务数据；不新增依赖；不动 provider 适配层 / `ProviderToolPolicy` / gateway envelope / `StreamingProviderInput`；不碰行数红线文件；不新增常驻清理任务；不 bump `PRODUCT_DATA_SCHEMA_VERSION`；不设计 enrollment 的重新授权语义（归在途 `FixAutomationEnrollmentDanglingSource`）；**不删除任何不具备 L1 证据、且不满足证据等级表中该类型 L2 专属可算条件的旁支记录**；**不改动既有 enrollment 文件锁的持有方与顺序**（只借用其临界区）。

## Decisions

### 0. 与在途实现的判定口径对齐（先于一切设计决策）

在途 `FixAutomationEnrollmentDanglingSource` **已落地并合并**（commit `dd62e5db`）。其「旧 source 悬空」判定 `enrollment_source_dangling`（`src/web/handlers/automation_enrollment_reenable.rs:148`）的口径是：**只判存在性**——终止记录的原 source 引用的 story/design 任一已不存在即判悬空；实体仍在但版本/确认态漂移**不算**悬空（维持逐字段相等 409，防止误改授权范围）；**读取失败 fail-closed 按「未悬空」处理**（保持既有 409，不放宽）。store 侧新增 `compare_and_set_reenable`（`src/product/issue_automation_store.rs:103`），与 `compare_and_set`（`:88`）在账本重放 / revision 线性化 / 幂等上完全一致，仅放行 source 漂移。级联删除侧新增 `invalidate_automation_referencing_entity`（`src/web/handlers/lifecycle/deletion.rs:690`，接线 `cascade_delete_entity` ⑥.5 `:203-206`）：删 Story/Design 时把 **source 引用该实体**的 enabled enrollment 一并**失效**（同源 terminate CAS、稳定 command_id 幂等；已 terminated/disabled 零改写；失败只 warn 不阻断删除）。**失效 ≠ 删除——enrollment 文件仍在。**

补充三条已落地事实（本能力只引用，不重裁）：

- **真实比较维度**：terminated 重新开启路径只逐字段比对 **source / options / target** 三项（`selection_key` 是「新会话身份」不比对、`command_id` 只进命令账本 digest、`expected_revision` 独立线性化）——见 `src/product/issue_automation_store.rs:1668-1680` 与 `:1613-1614`。**只有 source** 可在「旧 source 悬空」时被放行；options/target 漂移一律 409。
- **store 管道形态**：普通 `compare_and_set` 委托 `compare_and_set_inner(..., false)`（既有调用点语义不变），`compare_and_set_reenable(..., original_source_dangling)` 委托同一 inner（`src/product/issue_automation_store.rs:88` / `:103` / `:120`）。悬空放行时**复用既有 generation 字段**留痕（旧代 `current` 连同其悬空 source 只读进 `previous`、`binding_version+1`），**未新造状态枚举、未新造字段**。
- **级联编排的两层落点**（`src/product/lifecycle_store/cascade_deletion.rs` **不存在**）：用户面三类实体删除入口 `src/web/handlers/lifecycle/deletion.rs::cascade_delete_entity`；子树清理集合 `src/product/entity_deletion_cascade.rs`；`LifecycleStore::purge_entity_owned_children` 在 `src/product/lifecycle_store/plan.rs:1318` 起。`entity_deletion_cascade.rs` 未被该改动触碰（自动化策略层在 `src/web/handlers/lifecycle/deletion.rs`）。

本能力的悬空判定**必须与该口径一致**：story/design 的悬空只判存在性。本能力自带的 **L1/L2 证据分级只服务于 ③ 层「删不删」的决策**，MUST NOT 参与「悬空与否」的判定——否则两侧口径漂移，同一条记录在一处被当成悬空、另一处被当成未悬空。

### 1. 判据按字段名的静态表，不按 ID 前缀

代码库里不存在前缀→类型的集中注册表，且 `enrollment_id` 是裸 UUID、`checkout` 是强类型——按前缀判定在事实层就不成立。owner 也已拍板按字段名。故判据落成**「记录类型 → (引用字段, 目标类型)」静态表**，表外记录类型一律归未归类。

**替代方案**：① 按 ID 前缀——被事实否定，且 `plan_` 前缀在样例中多义；② 运行时反射式推断——无既有元数据可依据，且不可审计；③ 每记录类型写一段专用解析——重复且易漂移，静态表可一张表看全。

**代价**：新增记录类型要维护表 → 用「巡检未覆盖类型清单」作为变更评审项兜住漏报（见 Risks R3）。

**与在途实现的关系**：在途实现自带 story/design 的存在性判定（见 Decision 0）。本能力的目标存在性核对**必须复用同一口径**，不得另立一套「更严」或「更宽」的悬空定义。

### 2. 「已删」证据分两级，读侧不区分、销账必须区分

读侧容错只做「跳过」，不区分「被删」与「从未存在」，风险低；销账是物理动作，必须区分。

**r2 收紧后的定义**：

- **L1 直证** = 四条**同时**成立：① 墓碑记录存在（`{issue_lifecycle_root}/entity-deletions/{kind}/{id}.json`）；② 墓碑的删除阶段为**已完成**（`DeletionPhase::Deleted`，磁盘取值 `"deleted"`；另一态 `Purging` = 删除进行中）；③ 墓碑带完成时间（`deleted_at`）；④ 该实体**没有**待人工处置的删除失败事实（`{issue_root}/entity-operations/pending-failures/{kind}/{id}.json`）。
  → 理由：`Purging` 墓碑对应的子树可能仍在被重入清理；pending failure 表示删除**已失败待人工处置**。两者都属「**证据不足 / 删除未完成**」（**不是**「不可判定」——「不可判定」只收**读取失败**或**所需存在性数据源 / 作用域不可用**），**只报不删**并归入清单的「只报不删残留」节。台账是级联删除重入的反查索引（允许 purging/deleted scope 查询、entity root 缺失仍可反查），对它们销账会造成「级联重试漏清 session 且无人察觉」。
- **L2 推定** = 目标记录不存在 **且** **该目标类型在证据等级表中给出的专属可算条件**成立。「唯一可达上游」只在 `Plan` 一类上可解析，其余类型按表降级。**L2 只在「证据等级表」标为 L2 的目标类型上可用**；其余一律只报不删。

**证据等级表**（销账白名单的单一事实源）：目标类型 → 存在性来源 → 唯一可达上游 → **L2 专属可算条件** → 可删等级。story / design / plan 为 L1；reviewer 窗口的 attempt 为 L2（条件由窗口记录自身给出，见 §2b 与决策 7）；session、checkout、aggregate_index、member、enrollment 为**只报不删**（前者缺失是正常中间态，中间三类上游未核实无法定义，enrollment 永不删）。**该表的等级列已写进 spec REQ-DRG-02**，使 spec 与本文件、技术方案同源。

**r1 契约矛盾的消解**：r1 的 REQ-DRG-02「MUST 只对具备 L1 证据的目标执行删除」与 REQ-DRG-12「L2 双条件成立即删」互相矛盾。r2 统一为「L1 可删；L2 仅对本表标 L2 的类型且其**专属可算条件**成立时可删；其余 L2 一律只报不删」。

**替代方案**：① 单一判据「记录不存在即已删」——会把「从未存在」误判为已删并据此删除；② 为 attempt/session/enrollment 新建墓碑——扩大既有删除语义与覆盖面，且不在本 change 范围（硬约束）；③ L2 不设专属条件——误删风险显著更高；④ **给所有目标类型都配 L2「上游不可达」双条件**——r1 的尝试，因「上游不可达」在多数类型上**没有数据源**而不可实现（见决策 12）。

### 2b. attempt 目标的两路存在性核对（r2 新增）

reviewer 窗口与快照记录的 attempt 引用位上，**规划面窗口填的是 workspace session id**（`role_run_window_id` 的文档注释「规划面(plan review)窗口同走本函数:attempt 位用 workspace session id(规划面无 coding attempt 概念)」，`src/product/reviewer_window_store.rs:55-56`；写入点 `src/product/workspace_engine/review/drive.rs:1769-1780` 的 `attempt_id: &self.session.session_id`）。

既有物理清理器 `ReviewerResourcePurger::purge_for_attempts` 按 `record.attempt_id` 匹配（`src/product/reviewer_resource_purge.rs:66-72`），对两种形态一视同仁；但**存在性核对若只查 coding attempt**，规划面窗口必然 NotFound → 活的规划评审窗口被误判悬空并**被物理删除**。

→ 存在性核对**两路都做**：路一 coding attempt（`get_attempt_by_id`），路二 workspace session；**两路都不存在**才算目标不存在。

**路二的数据来源 MUST 是「枚举」**（r3 更正，闭环复审 N-5）：窗口 / 快照记录**不携带** `project_id` / `issue_id`，而 `list_workspace_sessions(project_id, issue_id)` **必须**两者齐备（`src/product/lifecycle_store/workspace.rs:556`）。故巡检 MUST **遍历状态根下的全部 project × issue 组合**调列举接口，在一次巡检内建一份 session id 集合，再以集合成员关系判定路二命中与否。两条替代路径**都不可用**：

- `find_workspace_session_path`（`workspace.rs:1230`）是 `pub(super)`，**仅 `lifecycle_store` 模块内可见**，新模块 `src/product/dangling_reference/` 调不到；改其可见性会碰 `workspace.rs`（1730/1730 **零余量**）；
- `get_workspace_session`（`workspace.rs:582`）在读路径上有**写回副作用**（旧会话首读时补生成并持久化 `review_identity_id`，`:598-612`），会破坏巡检的**零副作用**契约（REQ-DRG-10）。

路二的枚举若因 IO / 解析 / 权限错误**无法完成**，MUST 记入**不可判定**（REQ-DRG-23），MUST NOT 把「未能枚举」当作「目标不存在」。

**实测（2026-10-28，live 状态 `/tmp/aria-deploy/.aria`）**：12 条窗口/快照记录（6 窗口 + 6 快照 manifest）的 `attempt_id` **12/12 为 `coding_attempt_<32 位 hex>` 形态**，**无一条 session 形态**；三个 attempt id 在全树出现在窗口/快照记录里，**另**出现在根级 `tool-policy-run-audit/<attempt_id>/` 分区（该分区是 provider 侧工具策略审计，**不在巡检五分区内**，也不构成 attempt 本体记录）→ 12 条确为悬空。→ **规划面形态在本现场无样本**，其现场行为**待实现时核实**（技术方案 §11 U11）。

### 3. 容错落点：新增「容错列举」只读 API，既有读路径零改动

一个读路径能容错，必须**同时**满足三条：(a) 输出面向列举/投影，跳过一条不改变任何一致性判定；(b) 跳过不产生 fail-open（不会让本应被拦住的写入/删除/占用放行）；(c) 该路径上不存在「主记录缺失」语义。按此判据，窗口租约列举、物理清理扫描、台账按 owner 反查、scope 准入与墓碑复核**全部不容错**（违反 b）。

实现上，新 API 是**包装既有只读 API** 的新函数（`listing.rs`），既有函数本身零改动。这同时解决了行数约束：`reviewer_window_store.rs` 1921/1921 与 `code_review.rs` 1817/1817 零余量，连一行胶水都加不得。

**替代方案**：① 注入既有 `list_windows` 内部——零余量文件 + 会改既有函数语义；② 注入 `list_workspace_session_ids_by_owner`——违反 (b)，级联删除会静默漏清（fail-open），比留残留更危险；③ 改既有列举的错误语义（把解析失败从「跳过」改为「上抛」或反之）——违反 REQ-DRG-04/05 的「既有语义不变」。

**代价**：①层短期内没有面向用户的既有消费面 → 收益是预防性的而非止血（见 Risks R1）。这是本设计最重要的诚实边界。

### 4. 巡检器只有读能力

巡检遍历器不含任何删除分支、不取删除准入 lease。dry-run 的最强断言是「巡检前后状态根逐文件校验和不变」。③ 层是与巡检**分离**的能力，不挂进级联删除闭包（硬约束：不得扩大既有级联覆盖面）。

**替代方案**：① 巡检器复用删除闭包的清理能力——违反硬约束且闸门无法独立；② 巡检器内置「顺手清理」——违反 dry-run 契约。

### 5. 销账规则表与巡检判据表同源但独立

两者都是「记录类型 → 引用字段 → 目标类型」，但销账表更严：只列**允许删除**的记录类型与**允许删除的引用字段**。判据表覆盖而销账表未覆盖的记录类型 → 只报不删（REQ-DRG-14）。这样「漏维护销账表」的后果是「不删」而不是「乱删」——失效方向安全。

### 6. 闸 1 的摘要口径（r2 重写，r5 降级为审计关联）

**r2 的口径**（计算部分 r5 逐字保留）：

- **摘要 = findings 的规范化序列化的 sha256**。findings = 本次巡检**判定为悬空**的条目（含其中「只报不删残留」的条目本身），每条七字段（记录路径 / 记录类型 / 引用字段 / 引用目标类型 / 引用目标 id / 证据等级 / 建议动作）；按记录路径升序排序（同路径再按引用字段、目标 id 升序）；每条一行紧凑 JSON，行间 `\n`。
- **不参与摘要**：巡检时间、清单正文的时间行、令牌行自身、**不可判定条目**、**未归类条目**、各栏（汇总 / 只报不删残留 / 不可判定）的标题与说明文字。**「只报不删残留」条目本身的七个字段值参与摘要**——只排除其所在栏目的说明文字，不排除条目本身（r3 写死，见 REQ-DRG-22）。
- **确定性**：同一现场两次巡检的 findings 序列化**逐字节一致**（清单正文含时间，不要求整份一致）→ 验收口径据此改写。

**r5 的降级**：owner 决定**不做人工触发**——销账在服务启动时自动跑一次，**删除人工令牌环节**与人工 CLI 入口。故摘要**仅用于审计关联**（核对两次巡检是否同一现场），**不再是删除凭据**；删除的唯一门槛是**白名单类型 + 证据达标**（REQ-DRG-13 / 22）。

**代价（明示）**：销账的物理删除**不可回滚**，而「先巡检给人看」的人工闸门已取消——保护只剩「白名单 + L1 / W-1..W-4 证据门」与③层的幂等 + 留痕 + 自证归零。清单仍然逐次落盘，供事后逐条核对。

### 7. 窗口簇删除复用 `ReviewerResourcePurger`，不另写删除逻辑

既有清理器已按 attempt 归属覆盖活跃 + 归档窗口记录、树级租约、通道闸门、违规 journal、快照基线目录及其归档目录，且「缺失=已清（幂等）」。自写一套必然漏删归档目录、漏收割无主闸门（其 `channels/*/` 收割逻辑是为修一个具体缺陷写的）。③ 层只需反查出已删 attempt 的 id 集合再调它。

**但粒度 MUST 按 attempt 聚合**（r3 更正，闭环复审 N-9）：`purge_for_attempts` 是按 `record.attempt_id` 匹配后**批量删除该 attempt 名下的全部窗口**（含 `window-archive` 中的归档窗口，`src/product/reviewer_resource_purge.rs:66-72`、`:85-110`），而 L2 四条件（W-1..W-4）是**按窗口**判的。若某 attempt 下有一个窗口仍持租约或非终态，传该 attempt id 会把**满足条件的窗口一并删掉** → 违反 REQ-DRG-12「缺一即不删」。故销账 MUST 先按 attempt 聚合该 attempt 名下**全部**窗口（含归档），**全部满足**四条时才调 `purge_for_attempts`；任一不满足 → 该 attempt 名下的窗口一个都不删、只报不删。

> **现场不暴露该风险**：本现场 6 个窗口恰好分属 3 个 attempt，且每个 attempt 名下的 2 个窗口都满足四条（§2.4.1 实测）→ 该缺陷只在「同一 attempt 混合状态」时显现，**待实现时核实**（技术方案 §11 U15）。

**替代方案**：自写「按 attempt 删窗口簇」——与既有清理语义分叉，漏删风险高。

### 8. 留痕落点：如实表述既有机制，落全局 JSONL 台账（r2 更正）

**先核实既有「删除记录机制」的真实形状**（r1 的表述不成立）：

- 既有删除记录机制 = **逐实体墓碑** `{issue lifecycle root}/entity-deletions/{kind}/{id}.json`，**每 issue 级**（`entity_deletion_record_path`，`src/product/entity_deletion_store.rs:518-529`）；**不存在全局级 `entity-deletions/` 根**（实测 `{state_root}/entity-deletions/` 目录不存在）。
- 墓碑是**单条 JSON**（scope / kind / id / phase / 时间 / 可选 trigger_reason），**不是日志**（`EntityDeletionRecord`，`:88-103`）。
- 既有**追加式 JSONL 审计**的既有范式在**根级分区**：`tool-policy-run-audit/<workspace_session_id>/<role_run_seq>.jsonl`，每行 `{"schema_version":1,…}`（`src/cross_cutting/tool_policy_audit.rs:22`；分区路径 `src/product/lifecycle_store/tool_policy_run_audit.rs:59-63`），实测现场有同形态样本。

**既有删除记录机制无法承载销账留痕**，三条硬理由：① **语义**——`EntityScope.entity_id` 是「被删实体 id」，而留痕要记的是「被清理的旁支记录路径」；② **基数**——一次清理可连带多条旁支记录（窗口簇 = 窗口 + 租约 + journal + 快照目录 + 归档目录），墓碑是一实体一记录；③ **覆盖**——全局分区（reviewer-* / state/entity-identity）的清理无 issue 归属，而墓碑根每 issue 级。

→ **落点**：`{state_root}/entity-deletions/dangling-reference/cleanup.jsonl`（**该目录由本 change 新建**——目录名沿用删除语义，落点本身是新分区，不再声称「不另造目录体系」）；**行形态**沿用既有根级 JSONL 审计规范（每行带 `schema_version: 1`）；字段含 `cleanup_id`（= hash(record_path + target_id + action)，幂等去重）、`cleaned_at`、`deleted_entity`、`record_path` / `record_type` / `target_kind` / `target_id`、**`evidence_level`**（r2 新增）、`action`、`run_id`。

**约束（与 spec REQ-DRG-16 一致）**：落点是**状态根下单一全局**的追加式 JSONL 台账——MUST NOT 分散为每 issue 一份，MUST NOT 写入 issue 级删除墓碑目录，MUST NOT 改写既有墓碑记录，MUST NOT 扩展 `DeletionEntityKind` 枚举形状。spec 与本文件对此**已统一口径**（r3 更正；r1/r2 的「落在既有删除语义的目录族下、MUST NOT 另造审计体系」表述**已删除**——该表述与 §3.5.1 的核实结论相悖）。

**为什么不是扩展 `DeletionEntityKind`**：① `EntityScope.entity_id` 的语义是「被删实体 id」，而一次实体删除可连带清多条旁支记录 → 一对多不匹配；② `trigger_reason` 被约定为稳定短码（`src/product/entity_deletion_store.rs:90-102`），装不下「清了哪条」的路径；③ 改既有枚举形状 = 触碰既有删除语义（硬约束）。

**为什么是单一全局台账而不是每 issue 一份**：全局分区（reviewer-*/entity-identity）的清理无稳定 issue 归属；单一台账可统一对账，也便于「清理后再巡检自证归零」按批次核对。台账落在 `state_root` 下 → 天然在 issue 目录之外，不会被 issue 级联删除误清。

**替代方案**：每 issue 一份台账——全局清理落点不明；新建独立审计目录——违反「不另造」；直接扩展既有墓碑结构——见上三条硬理由。

**开放项**：落点归属（新建 `state_root/entity-deletions/` 子树 vs 改归既有根级审计分区范式）**需 owner 拍板**。

### 8b. intent 销账的并发与中间态（r2 新增，替代 r1 的错误论证）

r1 写的「销账与取代逻辑竞争 intent 文件 → 用『缺失=已清』的幂等语义，两者并发不报错」**对取代一侧不成立**：

- 取代路径的删 intent 段是**两步非原子**：`stale.metadata().is_ok()` 之后 `std::fs::remove_file(&stale)`（`src/product/issue_automation_store.rs:1736-1753`），且该段**就在 enrollment 文件锁内**（`compare_and_set_inner` 的 `with_exclusive_lock(&path, …)`，`:149` 起）。销账若不取同一把锁，就能在两步之间插进去 → 取代侧 `remove_file` 报 `ProductStoreError::Io` → **取代 CAS 整体失败**。
- intent 的 `plan_id` / `session_id` 是**预派生 id**（`PreparedPlanIntent::from_enrollment` 由 `prepare_intent_id` 拼出，`src/product/models/automation.rs:432-448`），**plan 尚未创建时它们就已经写在 intent 文件里**；`ensure_plan_binding`（`src/product/issue_automation_store.rs:348-451`）**先写 intent、后建 plan/session**，文档明写「create 失败原样穿出锁外，**意图文件保留为下次补偿锚点**」→ 「引用目标不存在」在 intent 上**不是**已删证据。
- generation intent 的 `phase` 一旦离开 `Claimed` 就是**防双跑检查点**（`PlanGenerationPhase` 无终态变体；`claim_plan_generation` 在文件缺失时按 `Claimed` 新建，`:519`）→ 删掉它会让下一次认领**从头起跑**。

→ **四道前置条件**（缺任一条 → 只报不删）：① 在 enrollment 文件锁内删除；② **锁内重读三项**——enrollment 记录、generation intent 的 `phase`、目标墓碑证据，三者 MUST 在**同一把锁内**读取（MUST NOT 使用锁外读到的旧值）：enrollment 记录不存在 MAY 继续 / 仍启用 MUST NOT / **已终止**（`terminated_at` 有值）且身份一致 MAY 继续 / 身份不一致 MUST NOT；③ 只认 L1 墓碑（session 与授权引用缺失不构成依据）；④ generation intent 的 `phase` 必须为 `Claimed`。

> **「已终止」的口径**（r3 更正，闭环复审 N-12）：`Disable` 命令**只置 `enabled = false`、不写 `terminated_at`**（`src/product/issue_automation_store.rs:1506-1510`），故「`enabled == false`」**不等价于**「已终止」。本设计取**保守口径**：以 `terminated_at.is_some()` 为准；仅禁用未终止者按「仍为活授权」处理 → **只报不删**。

> **为什么相位与证据也要在锁内重读**（r3 更正，闭环复审 N-3b）：I-2 通过后写者已不能推进相位，但**外部先读到 `Claimed`、锁内已是 `ProviderDispatched`** 的窗口仍然存在；同理锁外的墓碑读数可能在锁内已变为进行中态。锁外读到的旧值一律不作数。

**实测（2026-10-28）**：现场 generation intent 的 `phase == "provider_dispatched"` → 依 ④ **只报不删**；现场 plan intent 的 `plan_id` / `source.*` 有 L1 墓碑且 enrollment 文件不存在 → ①②③④ 满足 → **可删**；intent 的 `session_id` / `enrollment_id` 引用 → **只报不删**。

### 9. 幂等靠「缺失=已清」而非补偿日志

销账每步的判定是「目标已删 + 记录存在 → 删；记录不存在 → 视为已清」。这与既有删除面的幂等语义同构（`ReviewerResourcePurger` 的「缺失=已清」、墓碑的「purging→deleted 同条幂等更新」）。台账追加是幂等的例外（append-only），用 `cleanup_id` 去重保证不重复追加。

> **幂等的适用边界**（r3 更正，闭环复审 N-3）：「缺失=已清」只用于**重入**（同一清理动作重复执行）。它**不得**替代 REQ-DRG-20 的锁语义——删 intent 仍 MUST 在 enrollment 文件锁内完成判定；「记录已不存在」也 MUST NOT 成为在锁外跳过判定的理由。

### 10. 文件窗口：全部新逻辑落新目录，既有文件只碰三处极小胶水

新增目录 `src/product/dangling_reference/`（8 个文件），既有改动仅：`src/product/mod.rs`（+1 行模块声明）、`src/product/app_paths.rs`（+约 8 行台账路径）、`src/cli.rs`（+约 15 行 `aria inspect dangling-references` 子命令）。`src/web/handlers/automation_enrollment.rs`、`src/product/issue_automation_store.rs`、`src/product/reviewer_window_store.rs`、`src/product/coding_workspace_engine/code_review.rs`、`src/product/reviewer_snapshot.rs`、`src/product/entity_deletion_cascade.rs`、`src/product/lifecycle_store/plan.rs` 零改动。

### 11. 实施期必须注意的三个既有机制

- **会话读取的两个坑**（r3 更正，闭环复审 N-5）：
  - `LifecycleStore::get_workspace_session`（`src/product/lifecycle_store/workspace.rs:582`）在 `#[cfg(test)]` 下记读并可 panic；且它在读路径上有**写回副作用**（旧会话首读时补生成并持久化 `review_identity_id`，`:598-612`）→ 巡检**禁用**它。
  - `find_workspace_session_path`（`workspace.rs:1230`）是 `pub(super)`，**仅 `lifecycle_store` 模块内可见**，新模块 `src/product/dangling_reference/` 调不到；改其可见性会碰 `workspace.rs`（1730/1730 **零余量**）→ 也**不可用**。
  - → 巡检的 session 存在性核对 MUST 走 `list_workspace_sessions`（`:556`）的**枚举**形态：遍历**全部 project × issue** 建 session id 集合（见决策 2b）。
- **intent 记录形状**：`PreparedPlanIntent` / `PlanGenerationIntent` 的 `source` 是 `EnrollmentSource`（`stories` / `designs` 两个 `Vec<SourceRevisionRef>`，`src/product/models/automation.rs:17-28`），旧代文件无 `target` 字段时读为 `LegacyUnbound`（`src/product/issue_automation_store.rs:1190-1258`）。容错列举 MUST 复用既有版本化读侧，不得把 serde 缺字段错误降级为「跳过」。
- **既有退役逻辑**：自动化取代时会删同目录的两个 intent 文件（`src/product/issue_automation_store.rs:1739-1753`）。销账删除 intent 文件与该路径可能竞争 → 销账 MUST **在 enrollment 文件锁内删除**（REQ-DRG-20 的第①条）；「存在才删、缺失视为已清」的幂等语义**只用于重入**，MUST NOT 替代锁语义、也 MUST NOT 免除锁内重读（闸 3 与决策 9）。

## Risks / Trade-offs

- **[①层当前没有面向用户的既有消费面]** → 如实标注：三个旁支记录类型今天都没有「面向用户的列举面」（intent 只有按固定路径的单条读、窗口只有启动补偿扫描消费、台账只有按 owner 反查消费），所以新增的容错列举 API 若不被调用，①层的「不阻塞用户流程」是**预防性**收益。真正的止血来自在途的自动化线修复。若 owner 要求①层立刻产生可观测收益，需先解出零余量文件的余量（见 R4）。
- **[L2 推定仍可能误判]**（r2 改写）→ L2 的四条窗口条件全部可从窗口记录自身复算，但「目标已不在」仍可能因人工移动或未登记的读路径而误判。**L2 只对证据等级表标 L2 的类型有效**；其余一律只报不删；闸 2 的类型白名单是最后一道防线；销账前必过闸 1（owner 凭清单确认）。
- **[L1 过宽会把「删除进行中 / 删除已失败」当成已删]**（r2 新增）→ L1 收紧为四条（墓碑存在 + `phase == deleted` + `deleted_at` 有值 + 无 pending failure）；`purging`、缺完成时间与带 pending failure 的墓碑归「**证据不足 / 删除未完成 → 只报不删残留**」（**不**归「不可判定」——「不可判定」只收**读取失败**或**所需存在性数据源 / 作用域不可用**）。**实测现场 12 份墓碑全为 `deleted`、无 `pending-failures/` 目录**；`purging` 样本**待实现时核实**（技术方案 §11 U13）。
- **[规划面窗口被误判悬空并被物理删除]**（r2 新增）→ 存在性核对**两路都做**（coding attempt / workspace session），任一路命中即判「仍在」。**实测 12/12 为 coding attempt 形态**，规划面形态无样本 → 现场行为**待实现时核实**。
- **[路二的数据源不可得 → 活窗口被误判]**（r3 新增，闭环复审 N-5）→ 窗口 / 快照记录不带 `project_id` / `issue_id`，`list_workspace_sessions` 必须两者齐备 → 巡检 MUST **遍历全部 project × issue 枚举**建 session id 集合；`find_workspace_session_path`（`pub(super)`，新模块调不到）与 `get_workspace_session`（读路径写回副作用）**都不可用**。枚举失败一律归**不可判定**，MUST NOT 当成「不存在」。
- **[同 attempt 混合状态时误删窗口]**（r3 新增，闭环复审 N-9）→ `purge_for_attempts` 按 attempt id **批量**删除该 attempt 名下全部窗口（含归档），而 L2 四条件按**窗口**判 → 销账 MUST **按 attempt 聚合、全部满足才删**（决策 7）。本现场 6 窗口恰好全满足 → 现场不暴露，**待实现时核实**（技术方案 §11 U15）。
- **[锁外读到的相位/证据已过期]**（r3 新增，闭环复审 N-3b）→ enrollment 状态、generation intent 相位、L1 墓碑证据**三项 MUST 在同一把锁内重读**；外部先读到 `Claimed`、锁内已是 `ProviderDispatched` 的窗口真实存在。
- **[「已终止」与「已禁用」被混同]**（r3 新增，闭环复审 N-12）→ `Disable` 只置 `enabled = false`、不写 `terminated_at` → 口径取保守的 `terminated_at.is_some()`；仅禁用未终止者一律**只报不删**。
- **[巡检自身读到的坏记录被静默跳过 → 假归零]**（r3 新增，闭环复审 N-11）→ 巡检扫描到的记录解析失败、分区目录不可读，同样记入**不可判定**并逐条列出，不计入悬空数、不计入归零。
- **[intent 补偿锚点被删导致自动链永久卡死]**（r2 新增）→ intent 的 `plan_id` / `session_id` 是**预派生 id**，缺失是**正常中间态**；销账**只认 L1 墓碑** + 四道前置条件；`session_id` / 授权引用缺失一律只报不删。
- **[generation intent 检查点被删导致重复派发]**（r2 新增）→ `phase != Claimed` 一律只报不删（`PlanGenerationPhase` 无终态变体）。**残余**：现场 generation intent 的 `phase == provider_dispatched` → 永远不会被本能力清理（属「只报不删残留」）。
- **[销账与既有取代路径的 TOCTOU]**（r2 新增，替代 r1 的错误论证）→ 取代路径在 enrollment 文件锁内做 `metadata().is_ok()` → `remove_file()` 两步；销账**必须持同一把锁**并在锁内重读。**r1「两者并发不报错、幂等即可抵消竞争」的论证已删除**（对取代一侧不成立）。
- **[巡检读失败被算成「未悬空」→ 自证归零 → 误报治理完成]**（r2 新增）→ 「读取失败按未悬空」**限定在 enrollment 重新授权判定**；巡检另设**不可判定**计数，不为 0 不得宣称归零。
- **[「归零」口径过宽]**（r2 新增）→ 归零**只统计销账白名单内且本次可删的类型**；enrollment、无墓碑的台账条目等「永远不归零的残留」单列「只报不删残留」栏。
- **[判据表人工维护，可能漏报]** → 巡检清单必须列出「本次覆盖的记录类型」与「本仓其它持久化记录类型」供 owner 核对覆盖率；新增记录类型时把「巡检未覆盖类型清单」列为变更评审项。漏报只漏报，不误删。**r2 补登记**：intent 的 `enrollment_id`、enrollment 的顶层 `plan_id`/`session_id` 与 `binding_history.current.{plan_id,session_id,source}`。
- **[启动补偿扫描无法接入容错列举]** → `code_review.rs` 1817/1817 零余量；悬空窗口会让该树停在「kept lock (fail-closed)」（`src/product/coding_workspace_engine/code_review.rs:1146-1161`）即持续占锁。**待实现时核实**：是否可通过拆分该文件或登记新上限解出余量再评估接入。本次不擅自改零余量文件。
- **[行数门禁在该工作树已报红]** → 本能力的新逻辑全部落新文件、`src/product/mod.rs` +1 行、`src/product/app_paths.rs` +8 行、`src/cli.rs` +15 行，不触及任何零余量/已超文件，不加剧。**待实现时核实**：门禁在 HEAD 的实际报红清单与处置归属（属在途工作与门禁治理，不属本 change）。
- **[单一台账长期增长]** → 有意为之（审计留存）；保留期与轮转策略待 owner 依既有审计留存惯例裁定。
- **[cleanup_id 哈希碰撞导致漏记]** → 概率可忽略；采用全量比对（而非仅 hash 比对）可彻底消除，台账量级小时成本可忽略。
- **[留痕台账落点是新分区，与「留痕进已有删除记录机制」的 owner 表述有出入]**（r2 新增）→ 既有删除记录机制经核实**无法承载**（语义/基数/覆盖三重不匹配，且墓碑根每 issue 级、无全局根）；已如实更正表述，**行形态**沿用既有根级 JSONL 审计规范。落点归属**需 owner 拍板**（技术方案 §11 U10）。
- **[U1 四类（checkout / aggregate_index / logical_member / enrollment）的存在性在判定面不可判定]**（r4 更正）→ 读 API 已核实：`LogicalCodebaseStore::list_checkouts`（`src/product/logical_codebase/store.rs:792`）/ `load_checkout`（`:777`）、`list_members`（`:751`）/ `load_member`（`:736`）+ `LogicalCodebaseManifest.member_ids`（`store.rs:32`）、`AggregateIndexStore::get`（`src/product/logical_codebase/aggregate_index/store.rs:72`）、`IssueAutomationStore::get`（`src/product/issue_automation_store.rs:69`）；但 checkout / aggregate_index / logical_member 的记录落在 `logical-codebases/<lc_id>/`，核对 MUST 先按 `LogicalCodebaseStore::list(project_id)`（`:249`）枚举 LC 再以 `for_lc` 作用域化读取，而 Task 2 的判定面**没有** LC 上下文（`LogicalCodebaseStore::new(paths)` 走 legacy 根，直接读会**换个方式**继续误报「不存在」）。→ 判定面对这四类 **MUST 报「不可判定」（`Unknown`）**，MUST NOT 报「不存在」；真实读属持有 LC 上下文的 Task 4。四类仍**只报不删**，不阻塞其余类型。
- **[巡检清单落点]**（r2 已消解）→ 按 `rule://document-storage` 的「分析报告 → `cadence/analysis-docs/`」分类统一落点，不再与规则分类表冲突。

## Migration Plan

新增能力不改既有数据格式：新增的是既有状态根下的一个分区（留痕台账），既有记录格式、既有删除语义、既有级联覆盖面均不变 → **无需数据迁移，不 bump `PRODUCT_DATA_SCHEMA_VERSION`**。

按三步独立提交：

1. **第一步：判定能力 + ①层容错列举**。纯新增文件 + `src/product/mod.rs` +1 行。回滚 = revert 该步提交；新增目录整体删除，无既有行为变化、无数据变更。
2. **第二步：②层巡检 + 清单产出**。新增文件 + `src/product/app_paths.rs` 路径方法。**r5：巡检改由服务启动时自动触发**，人工 CLI 子命令（`aria inspect dangling-references`）**已删除**。回滚 = revert 该步提交；无数据变更。
3. **第三步：③层销账 + 台账**。新增文件 + 启动期自动销账入口。回滚 = revert 该步提交（**代码层面**）；**已执行的物理删除不可回滚**——**r5 起人工确认令牌已取消**，保护只剩「白名单 + L1 / W-1..W-4 证据门」与③层的幂等 + 留痕 + 自证归零，实施 Plan 必须写明。台账行与清单可作为人工复核的线索，但删除本身不可撤销。

第二步与第三步之间存在**外部顺序约束**：第三步的销账**不得早于**在途 `FixAutomationEnrollmentDanglingSource` 合并（否则可能在 enrollment 语义未定时删 intent 造成中间态）。该前置条件已满足——在途变更已合并（commit `dd62e5db`），三步可即刻推进。

## Open Questions

- 告警条数上界与去重键的最终取值（REQ-DRG-07 只约束「有上界」与「同键去重」，不约束具体数值）——可由实施者依既有日志/事件上界惯例裁定，不改变 spec。
- checkout / aggregate_index / member 三类目标的读 API——**r4 已核实**（`list_checkouts` / `load_checkout`、`list_members` / `load_member`、`AggregateIndexStore::get`，均须 `for_lc` 作用域化且 LC 由 `LogicalCodebaseStore::list(project_id)` 枚举）；三类仍**只报不删**，判定面在无 LC 上下文时 MUST 报「不可判定」（`Unknown`）；实施期若能定义上游并可算，可提出升级提案（**不改变本 spec**）。
- 台账保留期与轮转策略——由 owner 依既有审计留存惯例裁定，不改变 spec。
- 启动补偿扫描是否接入容错列举——**r5 已定**：接入（`code_review.rs` 的窗口列举点走①层容错），且以**零行数增长**为约束（该文件在门禁登记表内零余量）。
- **（r2 新增，需 owner 拍板）销账留痕台账的落点归属**：既有删除记录机制经核实无法承载（每 issue 级逐实体墓碑，无全局根，语义/基数/覆盖三重不匹配）。当前方案落 `{state_root}/entity-deletions/dangling-reference/cleanup.jsonl`（**目录由本 change 新建**，行形态沿用既有根级 JSONL 审计规范）；是否改归既有根级审计分区范式（如 `tool-policy-run-audit/` 同级的新分区）由 owner 裁定——**不改变本 spec**（REQ-DRG-16 只约束「落在**状态根下单一全局**追加式 JSONL 台账」与行形态）。
- **（r2 新增，待实现时核实）规划面窗口的现场行为**：`attempt_id` 为 workspace session id 的窗口在现场的真实形态与判定结果（本现场无样本）——不改变本 spec（REQ-DRG-21 的两路口径已定）。
- **（r3 新增，r5 已定）巡检清单落盘的相对根**：清单落 `cadence/analysis-docs/`；**r5 起由服务启动时自动产出**，相对**服务的工作区根**（`<workspace>/cadence/analysis-docs/`）——技术方案 §11 U14 已由实施 Plan 裁定。
- **（r3 新增，待实现时核实）同一 attempt 下窗口状态混合时的实际形态**：`purge_for_attempts` 按 attempt 批量删除，本现场 6 个窗口恰好全满足四条条件 → 该形态**无样本**（技术方案 §11 U15）。
