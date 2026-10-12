# Plan 阶段基准分支解析缺口修复：多仓 plan 会话基线解析整体跳过（聚合根无 .git 缺口）

日期：2026-10-12 ｜ worktree：`feat-b-0808-add-monorepo` ｜ 定论：诊断冻结修法实施（诊断权威 `local://plan-basebranch-diagnosis.md`）

## 1. 背景与根因

多仓 LC（ttt/notify-center，聚合根 `/tmp/notify-center` 无 `.git`，成员仓
busi/api/gateway/frontend 各自有）design 确认后进入 plan、选手动模式，首个 plan 生成报
引擎错误卡：「基准分支解析所需 git 仓库不可用：git show-ref refs/heads/main exited with
128: not a git repository」+「该错误不支持安全重试」。

- **存量缺口，非 anchor/trust 两 change 回归**：缺口 = 09-25（`84617c69` 等，
  REQ-PIB-02/03 T2/T3 fail-closed 基线链接入，聚合跳过仅 Story|Design）× 10-10
  （`5a217fa8` S6 方案 B，多仓 plan WS create 锚聚合根视图）相交即坏；anchor/trust
  只是让验收链首次走得更远、首次踩中。
- 调用链：多仓 plan 会话 `repository_path` = 聚合根视图（合成，无 `.git`，
  `workspace_repository.rs:428`）→ 首个 plan author run
  `build_work_item_plan_streaming_input_with_session`（`prompts.rs:466`）→
  `append_author_baseline_teaching` → `resolve_issue_baseline_tree`
  （`baseline_teaching.rs:48`）跳过守卫 `is_aggregate_story_or_design` 只含
  Story|Design，WorkItemPlan 漏跳 → `resolve_effective_base_branch` 默认链在聚合根
  上跑 `git show-ref` → 128 → fail-closed（REQ-PIB-02）终止该轮 provider 运行。
- **两面口径分裂消除**：同一 issue 在 plan 阶段存在两个基线面——preflight 面把 LC 当
  Non-Goal 明确跳过（`plan_preflight.rs:126-127` repo_id=None），author 教学面只跳
  聚合 story/design。owner 设定「恒 null→服务端 main→master 默认链」的真实落地点在
  coding fork / advance 的 per-member 面（`coding.rs:70`/`advance.rs:744`，真 git
  仓），plan author 面对多仓本就是 Non-Goal，只是守卫没跟上。

## 2. 修法（冻结口径）

新增谓词 `is_aggregate_planning_session`（`prompts.rs`，紧邻
`is_aggregate_story_or_design`）：

```rust
fn is_aggregate_planning_session(&self) -> bool {
    matches!(
        self.session.workspace_type,
        WorkspaceType::Story | WorkspaceType::Design | WorkspaceType::WorkItemPlan
    ) && self.has_aggregate_scope_system_context()
}
```

- 接线：仅 `baseline_teaching.rs` `resolve_issue_baseline_tree` 的跳过守卫由
  `is_aggregate_story_or_design()` 换为 `is_aggregate_planning_session()`——author 两族
  / reviewer 族 / 修订面四面共享同一解析链，reviewer 面自动同修；doc 注释同步
  （聚合 Logical 规划会话 Story/Design/WorkItemPlan，Non-Goal 多仓差异基线）。
- **不得扩 `is_aggregate_story_or_design`**（保持两臂）：它同时驱动 structured output
  contract（`prompts.rs:353`），而 `aggregate_author_schema_for/name_for` 只实现
  Story|Design 两臂，直接扩会把 plan 会话错误接进 Story/Design 聚合回写合同。
- 判别子安全性：`workspace_context/builder.rs:145-149` 保证 Logical routing 的
  Story/Design/WorkItemPlan 一律注入聚合视野 marker（`## 聚合代码库成员清单`）；
  单仓 plan（Legacy routing）无 marker → 不跳过 → 仍按真 git 仓解析，单仓 SC 行为
  零变化。LC plan 会话即使 focus 唯一锚到成员仓也带 marker → 一并跳过（与
  `plan_preflight` repo_id=None 口径对齐）。
- 未采用 `.git` 探测兜底（备选方案）：会削弱单仓 fail-closed 不变量
  （REQ-PIB-02）；判别子已完备，探测冗余。
- 默认链在 coding/advance per-member 面不动。

## 3. TDD 证据

夹具扩展：`BaselineEngineFixture` 新增 `lc_plan_engine_on_bare_directory()`——
WorkItemPlan 会话 + issue（repo_id=None、base_branch=None，与生产 LC 建模口径一致）
+ `repository_path` = 裸目录（无 `.git`，模拟聚合根视图）；聚合视野 marker 由测试注入。

红（修复前，`cargo test --lib baseline_teaching`）：

```
test product::workspace_engine::prompts::baseline_teaching::author_baseline_tests::lc_aggregate_plan_session_skips_baseline_resolution_on_bare_directory_root ... FAILED
thread panicked at baseline_teaching.rs:393:18:
LC plan 会话基线解析必须跳过而非 fail-closed: "基准分支解析所需 git 仓库不可用：git show-ref refs/heads/main exited with Some(128): fatal: not a git repository (or any of the parent directories): .git"
（与生产报错同站点同文案；对照面 bare_directory_without_aggregate_scope_still_fails_closed 同轮 ok）
test result: FAILED. 6 passed; 1 failed
```

绿（修复后）：

```
test result: ok. 7 passed; 0 failed（含新增两条）
- lc_aggregate_plan_session_skips_baseline_resolution_on_bare_directory_root：
  共享解析链 resolve_issue_baseline_tree()=Ok(None)（author/reviewer/修订全族同源跳过）
  + 端到端 plan author 面 build_work_item_plan_streaming_input 不 fail-closed、
  baseline_tree=None、prompt 不含「基准分支基线」教学块
- bare_directory_without_aggregate_scope_still_fails_closed：同一裸目录无聚合
  marker（单仓/Legacy 形态）→ 默认链 git 128 仍 fail-closed（REQ-PIB-02 不变量不削）
```

## 4. 回归

| 回归面 | 命令 / 覆盖 | 结果 |
|---|---|---|
| 本模块四面（author 两族 / reviewer / 修订 + 新增 LC 反例与对照） | `cargo test --lib baseline_teaching` | 7 passed / 0 failed |
| 全部 baseline 命名面（issue_baseline 单元、plan_preflight 五例、LC/coding per-member D4、单仓 fail-closed 族） | `cargo test --lib baseline` | 56 passed / 0 failed |
| 聚合合同面（structured output 协议、Story/Design 两臂不被污染） | `cargo test --lib aggregate` | 190 passed / 0 failed |
| 聚合视野上下文注入面 | `cargo test --lib workspace_context` | 34 passed / 0 failed |
| fmt / clippy（改动文件） | `rustfmt --check --edition 2024`（两文件）；`cargo clippy --lib` 扫改动行 | fmt clean；无新告警（prompts.rs:1154 unused import 为他人 WIP 既有告警，非本次改动行） |
| **全量** | `cargo test --lib` | **4409 passed / 0 failed / 7 ignored（51.04s）**（= 既有 4407 + 新增 2） |

## 5. 提交

- `fix: 多仓 plan 会话基线解析跳过(聚合根无 .git 缺口)`
- 文件：`src/product/workspace_engine/prompts.rs`（新谓词 +13 行，仅本 hunk 入库；
  工作区他人 WIP 的无关 fmt 折叠 hunk 未纳入）、
  `src/product/workspace_engine/prompts/baseline_teaching.rs`（守卫换谓词 + doc 同步 +
  LC 反例夹具/两测，+133/-3）、本报告。
