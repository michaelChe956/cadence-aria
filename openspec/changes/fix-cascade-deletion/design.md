# Design

## Context

工作树唯一根：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`；动机/范围见本目录 proposal。权威评审：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/cadence/designs-reviews/2026-10-13_设计评审_实体级联删除与报告身份修复_v1.0.md`。owner 2026-10-13 最终选择完整物理删除＋无内容删除记录＋review 随实体删＋后置引用拒删＋分级确认，覆盖评审原保审计建议。

已核事实：
- `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/web/handlers/lifecycle/deletion.rs::delete_work_item_plan/delete_schema_v2_work_item_plan_with_cleanup` 两路没有 source purge，且先删根再 purge；legacy 逐 WI 检查 coding，后一个拒删前可能已删前一个。
- `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/product/work_item_plan_source_store.rs::record_path/put_immutable` 按 plan 隔离对象，但对象 flock 会创建父目录；已核`WorkItemPlanStore::purge_plan_artifacts`、`WorkItemRevisionStore::purge_plan_revisions`是NotFound幂等模式，绝对文件清单见Plan。
- `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/product/lifecycle_store/work_item.rs::list_work_items` 返回真实 `source_work_item_plan_id/work_item_set_id`；仅revision purge不会删除issue-wide WI。`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/product/lifecycle_store/workspace_timeline_artifacts.inc.rs::delete_workspace_sessions_for_entity`只按entity/type，不能安全处理同名WI的其他plan binding。
- `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/product/lifecycle_store/spec.rs::delete_story_spec/delete_design_spec` 已清版本和会话；`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/product/lifecycle_store/review.rs::append_provider_review_round`按session_id归属；`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/product/lifecycle_store/plan.rs::save_work_item_split_provider_run`档案无owner字段，只按refs核归属，不整issue清空。
- `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/web/workspace_session/registry.rs::remove`不关闭旧Arc；manager active_run/engine Mutex仅覆盖部分入口。`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/product/provider_workspace_runner.rs::run_next`是独立REST writer，compile/recovery/source/普通session写无统一entity互斥。
- `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/web/handlers/support_parts/coding_cleanup.inc.rs::cleanup_shared_worktree_by_routing` Logical分支全issue枚举删除，目标Plan删除不能继续调用宽作用域helper。
- `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/product/models/lifecycle.rs`三类记录只有当前状态；已核`LifecycleStore::update_spec_confirmation_status`不更新版本confirmed_by，旧Draft无法可靠证明曾确认。Main追加裁定：未知旧Draft一次确认；可证从未确认新路径零提示；不改确认CAS。
- `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/web/src/components/lifecycle/IssueLifecycleWorkbench.tsx::handleDeleteLifecycleCard`的Story/Design无弹窗，drawer group/WI总弹窗；`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/web/src/api/client.ts`三类DELETE只有status响应。此次局部删除行为变更不是工作台布局改造，不继承该历史布局change的“本次零API变更”实施限制。

## Goals / Non-Goals

**Goals:** 沿现 handler 协调、store 拥有路径/校验/purge 的分层完成拥有闭包；服务端事实是资格权威；全拒删前零破坏、接受后可重试、无迟到写复活、无 scope 误伤。

**Non-Goals:** 不实现软删/恢复内容，不清未删除实体历史，不改 generic `json_store::write_json`、trust、确认/发布 CAS；不自动取消 provider/coding，不建立跨 store 强事务，不重做报告/IR身份算法或工作台布局。

## Decisions

### 1. 物理拥有闭包，引用不等于拥有

优先复用 PlanStore/RevisionStore 整 plan purge，并为 SourceStore 增加带 scope/ID 校验的幂等 `purge_plan_sources`（拟新增）。Plan 删除保留上游 spec；Story/Design 删除自己的 versions/sessions/review，存活 Design/Plan/WI/published lineage incoming refs 统一拒删。读取失败/归属冲突不可当无引用。

删除 manifest 由真实 records、全部匹配 session binding、全部版本 refs、compile 创建项与 lineage 构成。优先 `source_work_item_plan_id`，其次一致的 `work_item_set_id`；legacy 摘要仅用于无显式归属的旧记录，并对其他 Plan 反向归属核验。不能把其他 Plan 同名 logical WI 加入集合。review 以 session_id 和明确 version refs 核归属；provider-run 无 owner 时用全部存活根/版本/专属验证配置反向引用核验。共享 run 如仍被存活后置引用，先以 `entity_referenced` 拒删，不删他人依赖，也不默默留目标独占内容。除issue provider-runs，已核`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/product/work_item_split_engine/parse.rs::work_item_split_provider_run_dir`的全局`work-item-split-runs/{session_id}`与`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/product/lifecycle_store/tool_policy_run_audit.rs`的`tool-policy-run-audit/{session_id}`均属session审计闭包，按真实归属删除，不清其他session分区。

备选整 issue 目录 purge 成本低但会误伤；软删可保内容但违反 owner 选择，均不采用。

一致性P1补齐advance闭包：已核`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/product/advance_store.rs::root/initialization_path/retries_root`分别拥有issue_root下advance-records（record含plan_id，初始化journal以plan_id命名）与advance-retries（通过advance_id join）；`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/web/advance_plan.rs`另以workspace_root/advance-engine-checkpoints/workspace-sessions/{session_id}存临时engine checkpoint。它们是Plan推进产物，D1.2c按plan/advance/session精确purge并提供list-by-owner，D2.1d接真实advance/retry/恢复准入。删除身份检查优先于旧command/plan幂等命中，`advance_is_ready_for_attempt`对purging/deleted恒false；未删除Plan的REQ-ADV-01/02幂等和coding许可不改。不得整删issue/global checkpoint根。

### 2. 最小删除记录，子依赖先删、拥有根最后删

拟新增 issue-scoped `EntityDeletionStore`，记录路径位于现 lifecycle root 下 `entity-deletions/{kind}/{id}.json`。单条 JSON 只含 project_id/issue_id/entity_kind/entity_id、phase=`purging|deleted`、started_at/deleted_at；不含标题、正文、digest、session 内容或 manifest。所有门禁通过后先记 purging，清理成功并删根后改为 deleted，同一记录幂等更新；这是操作事实，不是保留业务实体的软删。

顺序：关联review/run/split-run/tool-policy审计及advance retry/journal/专属checkpoint→各拥有session/WI/advance根→source/IR/report/provenance、draft/compile/outline、revision/publication→entity根→完成记录。各持refs根在依赖清理成功前不删；纯ID pending操作manifest在purge前落盘、成功complete前移除。长期仅原身份/纯ID占用索引和单条无内容删除事实，不保存正文/digest/恢复副本或第二删除审计；固定list-by-owner来源见Plan。

purging阶段仅允许删除重入，普通写/新启动拒。根最后解决通常失败；根已删而pending manifest已移除/record尚未完成时，D1.0身份归属索引list-by-owner反查所有session，D1.1枚举固定scope子树/issue-wide真实owner和version refs，D1.2c枚举plan advance/retry；不存在=已清，IO/JSON/归属冲突=错误，不当空集合。纯ID身份索引长期只作准入，不保内容；短暂manifest成功移除，不重建根。全部枚举来源与接口列于Plan。

### 3. 稳定 scope 操作锁，运行中拒删，不发明 stop 流程

拟新增entity/Plan scope操作准入，复用`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/product/coding_attempt_store/locking.rs`的flock/异步非阻塞模式并扩shared/try-exclusive能力。锁位于不会随实体子树purge删除的操作元数据目录；锁inode不在持有期unlink。键为`(project,issue,entity_kind,entity_id)`，所属WI/session/report写归约到owning Plan或Story/Design。

生成、修订、review、compile/recovery/advance实际启动前取共享run lease，覆盖子任务/耐久提交/收口；删除try-exclusive失败返回entity_busy，不wait/cancel。低层`with_live_write`必须try-shared，取不到立即返回Deleting（映射Conflict实体正在删除），绝不阻塞持session/engine/object锁的线程。scope集合在最外层排序获取，持session/engine Mutex或对象flock期间不得首次取得scope锁；嵌套内层仅消费已持lease的私有write helper，不删原CAS。顺序scope→稳定repo resource锁（涉及共享工作树时）→issue关系短锁→session/object；长run不持关系锁，删除scope独占后短锁复核incoming refs。真实嵌套入口枚举与接线owner见Plan，不以静态锁序口号代替闭包。

coding attempt创建虽不由本change中止，也必须消费同Plan scope共享准入：已核`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/product/coding_attempt_store/attempt_creation.rs`的creation guard，以及`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/product/coding_attempt_store/group_initialization.rs`/`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/product/coding_attempt_store/group.rs`的journal/direct create只持原创建/arbitration锁。D2.1c外套scope lease，删除独占时创建拒，创建先赢时删除busy或coding_workspace_exists；原coding admission/唯一性/运行合同不改。

接地P1的shared-worktree中央writer归D1.3c：独占`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/src/product/lifecycle_store/worktree.rs`只加外围准入，不改现CAS/锁/删除helper语义。记录无plan_id，按attempt.work_item_group_id/plan binding、WI.source_work_item_plan_id/work_item_set_id、current/last WI与target快照反推所属Plan，Unknown/矛盾不得猜唯一归属；稳定repo resource锁位于purge外，共享writer与独占cleanup均消费。D2.2只读使用该文件现`delete_repo_shared_worktree/delete_issue_shared_worktree`，target-scoped helper放coding_cleanup.inc.rs，不在D2.2改worktree.rs。共享基础设施仍被他Plan使用则保留；共享review/run业务内容外部引用则按REQ-GRP-01拒删，两种语义分开。

创建/attach/dispatch 必须核实体及 parent binding 身份。被删实体 ID 不复用；session 自动分配须保单调高水位，显式/派生 session 身份不得复用已分配给其他实体的身份。分配索引只保 ID/scope，不是被删内容；旧 session ID 在清理前进入身份高水位/已分配集合，旧连接不能误命中新实体。旧 Arc 标为失效且所有 dispatch/store submit 复核墓碑；失效对象不授予任何 trust/确认权。

备选只查 Running/engine Mutex/registry remove 改动小但 REST/late writer 不受覆盖；自动 abort/stop 会改变运行语义且现接口不能证明收口，均不采用。此 scope guard 是删除互斥，不是新的确认 CAS 或安全授权。

### 4. 服务端分级确认和兼容未知历史

三类记录拟增 `ever_confirmed: Option<bool>`，`serde(default,skip_serializing_if=Option::is_none)`；新创建 Some(false)，既有真实确认写在原确认成功根写入中单调设 Some(true)，状态回退/候选替换/快照恢复不能抹掉 true。不更改确认 CAS 期望值/nonce、发布条件、trust 判据。

分类：当前 Confirmed、确凿版本 confirmed_by、session批准/发布事实或 ever_confirmed=true → required；新路径 ever_confirmed=false 且无确凿确认事实 → none；旧字段 None 且无可证历史 → required。不能拿 Draft/无发布当“从未确认”。旧 None 不批量伪回填 false。

沿现 DELETE 路径加入可选 `confirmed=true` 请求参数（拟新增，无效值拒绝），未确认但 required 返回409 `deletion_confirmation_required`，details 只含 scope/kind/id/确认理由与清除范围类别。前端第一次 DELETE 对真正 Draft 直接成功；服务端要求确认时用现 window.confirm 恰一次，取消无第二请求，同意再 DELETE confirmed=true。coding/reference/busy 先于确认响应，所有拒绝零删除。该布尔是删除交互确认，不是安全/内容确认许可，服务端仍完整复核门禁。无需第二 preview 路由，不让前端列子对象当清理权威。

### 5. R/D1/D2 与跨线移交

R 消费 FixReportId 先行独立实现及验证/审查证据，不在此 change 重复写 report ID；content-addressed payload/两 writer/旧 immutable 校验规则保持。D1 拆存储事实与拥有 purge、低层准入、确认事实三个可独立验收包；D2 拆运行 lease、DELETE 协调、前端一次确认。每个包 exclusive files 和≤4h尺度见 Plan，共享文件严格串行。

默认R→SG3a.0→D1 source窗口；共享写面需真实移交。额外点名F1.2/SG3b compile与四fixture→D1.4/D2.1b，SG4a.0 workspace/human_gate/tests/manager→D1.1/3a/3b/D2.1a，SG4b及SG6a.3 workspace_single_candidate→D1.3a，SG7a lifecycle/durable_projection→D1.3b/D2.1a；trust/anchor最终移交仍在前。SG Plan §0.2只新增窄双向登记，任务/预算/范围不改；D1.3c在无SG写面交集窗口可开。

## Risks / Trade-offs

- 永久删除 Confirmed/发布审计不可恢复 → 一次全闭包警告、只留事实；这是 owner 明确选择，不做隐藏备份。
- 旧 Draft 曾确认信息已不可恢复 → Main 已裁定保守一次确认；新路径精确零提示，None 不伪造 false。
- 跨 store 失败与 crash → purging 先落、依赖先删、根最后、成功前完整范围核验；不宣称原子事务。
- 运行入口多且低层写能建目录 → 长期共享 lease＋低层同 scope 判据；测试必须用真实 REST/manager/compile/迟到提交链，不以 DTO/mock 模拟所有入口已接线。
- provider-run 归属只有refs、shared-worktree为 issue/repo共享 → 反向引用保护，目标专属清理；不整目录清空，仍被其他 Plan 使用的共享资源保留。
- 新锁与旧 session/对象锁可能死锁 → 固定锁序、删除try-exclusive fail-fast、不改原 CAS 逻辑；稳定 inode 不随purge unlink。
- SG/anchor 同文件改动 → controller 显式 task/commit/review/写面移交；checkbox/agent空闲不是移交证据。

## Migration Plan

仅先部署安全准入/删除事实/ever-confirmed兼容，再接完整 DELETE 和前端；在 D1/D2全闭包、独立审查与 owner 审阅完成前不得宣称物理删除能力上线。旧业务记录/immutable refs 不改字节回填；旧 None 使用保守确认。已删除内容无法回滚恢复，回滚只停止新删除入口并保留已落事实；不得撤销 tombstone 让旧 writer 复活。主规格 sync/archive 只在实施完成并真实验收后执行，本次仅提案与 Plan，不写实现、不 commit。
