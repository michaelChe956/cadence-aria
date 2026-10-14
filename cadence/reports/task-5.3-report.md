# Task 5.3(B2b2:legacy outline 反馈的 full fresh)实施报告

- 实施:ImplAnchorB53(glm53-task worker)
- worktree:`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`(共享树)
- 基线 HEAD:`7035a193`(fix: B2b3 Draft Batch PlanRevision同业务代恢复;5.4 与本包按协调反序落地,见 §6)
- 状态:实现完成;anchor_t15 红(dispatch 拒收零 spawn)→绿(6/6);范围化回归全绿;独立 commit 已落(见 §5)

## 0. 写面与协调裁定(先读)

1. **gateway_start.rs 写面缺口裁定(Main IRC 批准,2026-10-12)**:验收(t15 端到端 fresh→原门)必须把 `lc_fresh_dispatch_kind` 的 `LcFreshRunKind::Kind(_)` typed 拒臂对 `legacy_outline_author`/`legacy_outline_revision` 两 tag 放行(5.1 占位注释自述「接管后翻覆盖面」);该文件不在 5.3 任务书清单,Main 裁定「验收权威于文件清单」。约束遵守:最小放行+注释翻覆盖面;5.4 的三 tag(draft/batch/plan_revision)由 5.4 自己放行(其提交 7035a193 先行落地);sc_author/sc_manual_revision 仍拒收归 5.5/5.6。
2. **prompts/outline_contract.rs 子文件**:outline prompt 渲染器族(初次/增量修订)本住 `src/product/work_item_split_engine/prompts/outline_contract.rs`(large_file_guard 拆分架构);fresh 返修渲染器是族第三成员,复用族内私有 helper(strict contract/最小正确示例/来源 ID 条节/write scope 规则),在 prompts.rs 重复整套渲染会造成合同双实现漂移。按与 gateway_start.rs 同类「清单 letter vs 结构必需」处理,偏离仅此一处子文件(同目录同 face)。
3. **与 5.4(B54)串行协调**:共享树下 provider_run.rs/tests/planning_resume.rs 双包清单互叠,IRC 约定 B54 先落 5.4 提交(7035a193),本包叠后(gateway 两 tag 放行/t15 追加/t13⑤ 换载),全程单一未提交态,以 git log 为准。B54 另在 provider_run.rs 提供共享 fresh 判别口 `lc_fresh_recovery_record_if_running`(pub(super));本包 include 臂保持自含判别(读 arm 既有 `lifecycle_for_run`,语义等价:FreshRunning+tag 判别),不交叉依赖未定型接口。
4. t13⑤ 的「未接管 tag」示例由 LegacyOutlineRevision 换 `sc_author`(B54 提醒:draft/batch/plan_revision 归 5.4 接管会翻红原示例)。

## 1. 交付内容

| 文件 | 变更 |
|---|---|
| `src/product/work_item_split_engine/prompts.rs` | ① 新引擎方法 `WorkItemSplitEngine::build_outline_fresh_revision_invocation(request,lifecycle,issue,business_repository,author,source_context,routing)`(接口字典冻结签名,pub(crate) 随文件惯例):legacy outline 三 tag(author/revision/rebuild)→ prompt 自足 full fresh 返修 invocation;非法 tag typed 拒(`lc_resume_recovery_context_unsupported`;socket 模块对 product 层私有,本地同值常量 `OUTLINE_FRESH_CONTEXT_UNSUPPORTED_CODE`,wire 值由用例钉住);revision/rebuild(outline_revision origin)durable 反馈为空注入默认返修文案(禁止无反馈出生 full);输出复用现 invocation 字段/nonce ② durable 装载助手(全只读,`OutlineFreshRevisionFacts`+三 loader):上一版 outline 全文(session artifact 版本按 run_context outline_version 钉选、缺省最新 WorkItemPlanOutlineCandidate,pretty JSON 全文)、review 记录(review_refs→node detail verdict 全文)、context resolutions(outline context index 的 blocker_resolutions 只读保留;capabilities/gaps 重算属出生路径职责,不在 fresh 面)③ re-export 新渲染器 |
| `src/product/work_item_split_engine/prompts/outline_contract.rs` | 新 `build_outline_fresh_revision_prompt_with_nonce`(§0.2):与初次 full 同源渲染完整 story/design/repository 材料+context_resolutions+strict contract(revision=true 语义),显式携带 [previous_outline]/[revision_feedback]/[review_feedback] 小节(缺项省略不伪造);preamble 声明「显式全新会话…不依赖任何历史会话」;既有初次/增量渲染零变化 |
| `src/web/workspace_ws_handler/run/provider_run/work_item_plan_legacy_author.inc.rs` | 臂内 fresh 消费:durable session record 读 `lc_resume_recovery`,仅 `FreshRunning` 且 run_context 为 outline 三 tag 时首分支走 `build_outline_fresh_revision_invocation`(否则原 delta/出生路径零变化);fresh run 全新节点(`begin_work_item_plan_outline_run`,不沿用中断 OutlineRun 节点)+`build_work_item_plan_streaming_input_fresh`(resume=None,旧 native id 不回捞);rebuild 的 worktree/inventory 注入与「snapshot 仅 provider start 成功后 commit」原样;AutoRevision 循环不变(fresh 首轮后同新会话增量,正常 resume delta 不变);prompt 事件标签三态 |
| `src/web/workspace_ws_handler/run/gateway_start.rs` | `lc_fresh_dispatch_kind`:B2b3 臂后、catch-all 前新增两 kind 放行臂(§0.1 裁定;形状与 5.4 臂一致);其余 tag 仍 typed 拒零 spawn |
| `src/web/workspace_ws_handler/tests/planning_resume.rs` | ① 新增 `anchor_t15_legacy_outline_fresh_retains_feedback_and_prior_outline`(端到端)+`T15OutlineFreshProvider` fake(record input→立即完成合法 outline 结构化输出;WorkItemSplitter author 输入不携带 structured_output_contract,从 prompt 提取 strict contract sentinel nonce 自行包装——part_03 review fake 先例的 author 输入同型适配)② t13⑤ 换载 sc_author(§0.4) |
| `src/product/work_item_split_engine/tests/prompt_contract.rs` | 新增 5 契约用例(anchor_t15_*):自足性(材料/旧稿钉选/反馈/review/resolutions/「不依赖任何历史会话」)/非 outline tag typed 拒(文案含 tag 短名)/rebuild origin feedback+旧稿不丢/空反馈默认文案/author tag 携旧稿且无 [revision_feedback] 小节;`OutlineFreshContractFixture`(lifecycle story/design/version+session+artifact 版本+review 节点+outline context index 全 durable 形态) |

## 2. 设计对照(tasks.md 5.3 验收)

- **outline feedback mismatch→waiting→fresh→原门**:mismatch→waiting=t13①-④(既有,回归绿);fresh→原门=t15 端到正路:claim→dispatch(放行)→臂内 full fresh→`complete_work_item_plan_outline_author_from_output`→`enter_work_item_plan_outline_confirm`(原 outline 确认门:AuthorConfirm stage+WorkItemPlanOutlineConfirm 节点+WaitingForHuman,不重臂不换门)。
- **prompt 有旧 outline 全文/feedback/review/current full material**:[previous_outline](钉选版本 pretty JSON 全文)/[revision_feedback](durable 冻结值,含审核意见)/[review_feedback](review_refs 的 durable verdict)/完整材料段(story/design/repository structure/gaps/resolutions);契约+t15 双面断言。
- **ID None**:fresh 分支 `build_work_item_plan_streaming_input_fresh`→`input.resume_provider_session_id.is_none()`(t15 断言)。
- **rebuild origin feedback 不丢**:origin=outline_revision 的 normalized_feedback 注入 fresh prompt(t13 rebuild 用例的 seed「重建前返修反馈」现被真实携带;契约用例独立钉住)。
- **snapshot 仅成功 start 后 commit**:rebuild 臂 `commit_rebuilt_snapshot_after_provider_start` 原样(start Err 不落盘)。
- **context resolutions 保留/正常 resume delta 不变**:fresh 面只读 index;delta/出生路径代码零改动(t13-④幂等/既有 outline 用例回归绿)。

## 3. 红→绿证据(本 worker 会话实测)

1. **契约红(接口)**:测试先行写入后 `cargo test --locked --lib anchor_t15` 在方法落地前不可编译(E0599 族;实现与用例同包收敛,编译红由 5.2 同型先例覆盖)。
2. **行为红(t15 端到端,dispatch 放行前)**:`cargo test --locked --lib anchor_t15` → **5 passed; 1 failed**,过滤命中 6 个新用例(非 0 tests);`anchor_t15_legacy_outline_fresh_retains_feedback_and_prior_outline` 在判据行原样 FAIL:`planning_resume.rs:2366 .expect("显式 fresh 授权后 outline 反馈族必须真实 spawn（非 typed 拒）")` 超时——dispatch 对 LegacyOutlineRevision 仍 typed 拒零 spawn(中间一次实测失败点前移至 2397 回门断言,定位为 fake 对 author 输入 contract=None 走裸 JSON 回退→「Outline 结构化输出解析失败」,按 part_03 先例修 fake 的 prompt nonce 提取,与本包实现无关)。
3. **绿(放行后)**:`cargo test --locked --lib anchor_t15` → **6 passed; 0 failed**(5 契约+t15 端到端终态断言:resume None/prompt 含旧稿全文+反馈+review+材料段/AuthorConfirm+OutlineConfirm/flow==Legacy/new_workspace_count==0/recovery Resolved+fresh_native_session_id=="t15-fresh-native"+operation Completed)。
4. t13 放行后按 §0.4 换载 sc_author → `anchor_t13` **4 passed; 0 failed**(未接管 tag 仍 typed 拒零 spawn 负例保留)。

## 4. 范围化回归(全绿,基线 7035a193+本包)

`anchor_t` 全族 65 passed(t7–t16);`anchor_t15` 6;`anchor_t13` 4;`anchor_t14` 2;`planning_resume::` 17;`tests::gateway_start` 17;`prompt_contract::` 30;`work_item_split_engine` 142;`workspace_ws_handler::tests` 182;`cargo check --lib --tests` 0 error,本包写面零告警(存量告警属他人面,含 5.4 提交内 planning_resume.rs:1907 unused_mut,非本包)。

## 5. 提交

```bash
git add -- src/product/work_item_split_engine/prompts.rs src/product/work_item_split_engine/prompts/outline_contract.rs src/web/workspace_ws_handler/run/provider_run/work_item_plan_legacy_author.inc.rs src/web/workspace_ws_handler/tests/planning_resume.rs src/product/work_item_split_engine/tests/prompt_contract.rs src/web/workspace_ws_handler/run/gateway_start.rs
git commit -m "fix: B2b2 legacy outline反馈的fullfresh"
```

- commit hash:`4620cb5d`(fix: B2b2 legacy outline反馈的fullfresh;6 files,+1374/−31,基于 7035a193)
- 未 stage:本报告(纪律)、他人面的存量未提交改动(context.rs/engine.rs/cross_cutting/provider_run.rs 三处 rustfmt 重排,非本包)。

## 6. 已知边界与移交

- **5.4 反序落地的 VCS 序**:plan DAG 5.3→5.4,共享树协调下 5.4(7035a193)先落、本包叠后;两包测试互含回归全绿(t16 用例在本包绿态下通过)。
- socket 常量对 product 层私有→本地同值常量(wire 值由 t13⑤/t15/契约用例钉住)。
- fresh 判别以 durable `FreshRunning`+tag(FreshRunning 窗口=dispatch CAS 至收口);与 5.4 draft/batch 臂各自独立消费同源信号。
- 移交 5.5/5.6:sc_author/sc_manual_revision 放行未接(t13⑤/t16-untaken 以 sc_author 为未接管示例)。
- B2b2 后下一包(5.5)按 tasks.md 串行;legacy_author include/prompts 面移交。
