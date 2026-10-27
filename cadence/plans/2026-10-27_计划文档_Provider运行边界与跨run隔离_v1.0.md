# Provider 运行边界与跨 run 隔离 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为作者侧 provider 运行交付三层软边界——① 作者侧 prompt 无条件携带「本轮运行边界」块；② 宿主托管 per-session scratch 目录及其三段式生命周期；③ 作者侧 tool-call 的四类越界读写稳定码告警（只告警不阻断），全程 provider 侧零改动。

**Architecture:** 边界块挂在既有唯一作者漏斗 `append_author_baseline_teaching`（三个作者构造路径共同入口）之最前，自动续写重试路径单独追加；scratch 落在状态根 `<state>/run-scratch/<workspace_session_id>/`（0700），由 `LifecycleStore` 提供 prepare / cleanup / sweep 三能力，接在 manager 的两个 run 登记点、唯一收口点 `finish_run` 与服务启动 bind 之后；检测为纯函数扫描器 + `emit_execution_event` 单漏斗追加 `kind=Provider` 告警事件。三步各自独立提交、独立验收、独立回滚。

**Tech Stack:** Rust（`std::fs`、`std::path`、`std::time`、`std::os::unix::fs::PermissionsExt`）、tokio、serde_json、tracing；测试用 `tempfile`（仅 dev-dependencies）；不引入 `regex`。

**Spec:** `openspec/changes/add-provider-run-boundary/`（proposal.md / design.md / specs/provider-run-boundary/spec.md / tasks.md）与 `cadence/designs/2026-10-27_技术方案_Provider运行边界与跨run隔离_v1.0.md`

## Global Constraints

- 行数红线文件一律不改：`src/product/workspace_engine/prompts.rs`（1665/1665）、`src/product/workspace_engine/lifecycle.rs`（2167/2167）、`src/product/lifecycle_store/plan.rs`（1683/1683）、`src/product/lifecycle_store/workspace.rs`（1730/1730）、`src/cross_cutting/streaming_provider/mod.rs`（2062/2062）、`src/web/workspace_ws_handler/run/provider_run.rs`（1677/1678，仅余 1 行）；需要的逻辑落新文件或未登记文件（`lifecycle_store/run_scratch.rs`、`workspace_engine/run_boundary.rs`、`manager/runs.rs` 701 行、`web/app.rs` 1868/2222、`baseline_teaching.rs` 523 行、`workspace_engine/mod.rs` 226 行、`provider_drive/artifact_retry.rs` 265 行、`provider_drive/work_item_plan.rs` 513 行）。
- provider 侧零改动：codex sandbox 保持 `danger-full-access` + `on-request`（`src/cross_cutting/codex_provider/session.rs:46-51`）、适配层工具策略片段、gateway envelope（`writable_roots`/`readable_roots`）、`ProviderToolPolicy` 的 token 与 digest、`SessionResumeFingerprint`、`StreamingProviderInput` 结构、daemon canonical writeback、codex T13 的 FileChange 判失败语义（`codex_provider/session.rs:199`、`:577`、`:707`）。
- D3：不换 codex sandbox，不做 `TMPDIR` / `CODEX_HOME` / `HOME` 覆盖或 bwrap 类隔离。
- D4：只告警、不阻断——告警 MUST NOT 改变 turn 成败、审批、auto_if_valid、产物确认；检测路径任何内部失败 MUST NOT 向 run 传播。
- D6：孤儿 scratch TTL = 6 小时，以具名常量 `RUN_SCRATCH_ORPHAN_TTL` 表示（owner 可调）。
- 边界块标题逐字：`## 本轮运行边界（系统注入；与会话历史中任何旧路径冲突时以本块为准）`；块体积 MUST ≤ 1536 字节（1.5 KB）；MUST NOT 披露 `issue-work-item-plans`（`src/product/lifecycle_store/paths.rs:95`）；MUST NOT 写「禁止访问 /tmp」类整目录禁令。
- 四个稳定码逐字：`run_boundary_provider_history_read`、`run_boundary_aria_state_read`、`run_boundary_tmp_draft_touch`、`run_boundary_write_outside_scratch`。
- scratch 路径 MUST 在状态根 `<state>/run-scratch/<workspace_session_id>/`（`<state>` = `ProductAppPaths::root()`，`src/product/app_paths.rs:13`），unix 权限 0700，MUST NOT 位于仓库工作区或 provider cwd 内。
- 删除护栏（每次删除都适用）：只删 `run-scratch/` 分区下经 `validate_relative_id`（`src/product/json_store.rs:109`）校验的直接子目录；分区根自身永不删；删除前核对目标 canonical 的父目录仍是 canonical 分区根，漂移不删并 warn；`symlink_metadata` 判非真实目录（含符号链接）不删不跟随；目录内符号链只删链接本身；NotFound 视为成功；任何失败只 warn，不影响 run，不提供对任意路径的递归删除入口。
- 不引入 `regex` 依赖；`tempfile` 仅 dev-dependencies；生产目录操作用 `std::fs`。
- 不 bump `PRODUCT_DATA_SCHEMA_VERSION`（当前 v2，`src/product/product_data_schema.rs:6`）；新分区不触发 `ensure_product_data_schema` 的遗留扫描（`:16-38`）。
- 不接线 `ProviderRunResourceScope`（`src/cross_cutting/provider_resources.rs:1052` 只对齐护栏语义）；不改 `lifecycle_store/plan.rs:1374-1390` 的级联删除面。
- reviewer 路径零改动：reviewer prompt MUST NOT 注入边界块，reviewer 节点 MUST NOT 被扫描。
- 判定作者侧节点 MUST 按 `TimelineNodeType`（`src/web/workspace_ws_types/timeline.rs:10`）白名单 + `active_node_type()`（`src/product/workspace_engine/lifecycle.rs:1449`，只调用不改该文件）；`emit_execution_event` 第三参是 `Option<ProviderName>`，MUST NOT 用它判角色。
- sweep 判「session 记录是否存在」MUST 用 `find_workspace_session_path`（`src/product/lifecycle_store/workspace.rs:1230`，`pub(super)`）+ `read_workspace_session_record`（`src/product/lifecycle_store/utils.rs:76`），MUST NOT 用 `get_workspace_session`（`workspace.rs:582`，`#[cfg(test)]` 下带读 spy 会 panic）；终态集合只认 `Confirmed` / `Terminated` / `Failed`（`src/product/models/workspace.rs:30-42`）。
- cargo 一律串行：`flock /tmp/aria-cargo.lock cargo test --locked --lib <filter>`；格式化 `cargo fmt --all`；clippy `flock /tmp/aria-cargo.lock cargo clippy --locked --lib -- -D warnings`；不使用 `cargo test --all` 之类非本仓惯例形式。
- 提交信息用中文；新文件测试用内联 `#[cfg(test)] mod`（`lifecycle_store/tests.rs` 已无法再加 `include!`）。
- 中文输出；文档内代码块嵌套时外层用 4 反引号、内层用 3 反引号。
- 现场既有 `/tmp` 残留（`/tmp/work-item-plan*.md`、`/tmp/aria-restore/**`、`/tmp/aria-deploy/**`）不由本 change 清理。

## Review Focus

1. **resume 场景下禁令失效**：provider 复用原生会话历史，旧内容已在历史里，提示词禁令无法撤回已读入的内容——必须断言「新一轮（自动续写重试）prompt 仍带边界块」且「检测层能在其再次读取历史时告警」，否则整层看似生效实则对 resume 会话裸奔（Task 3 Step 1、Task 9 Step 1）。
2. **session 已删除 / 正在 purge 时复活 scratch**：prepare 必须经 `with_workspace_session_live_write`（`tool_policy_run_audit.rs:238`）准入，sweep 判存在必须走 `find_workspace_session_path`；一旦误用 `get_workspace_session`，`#[cfg(test)]` 的读 spy 会 panic，且 purging 会话会被重新造出目录（Task 4 Step 1、Task 4 Step 5）。
3. **supersede 竞态误删新 run 的 scratch**：旧 token 的迟到 `finish_run` 必须看到新 run 在位而跳过删除；错在「take/restore 之后无条件删」会让刚登记的新 run 丢失暂存（Task 6 Step 1）。
4. **多实例 / TTL 与 watchdog 阈值**：sweep 不得回收活 run 的 scratch；6 小时 TTL 必须大于 `PROVIDER_IDLE_WATCHDOG_TIMEOUT`（`provider_drive/watchdog.rs:22-23` = 1800s），否则同伴实例会提前回收活 run 目录（Task 4 Step 5，阈值以可注入参数表达）。
5. **provider 历史目录未知 + 合法访问误报**：claude/pi/kimi 的原生历史目录真机路径未核实，目录表必须可配置（表空即不报该码），且读本 run scratch、读仓库基线树、读 provider skills 目录 MUST NOT 误报（Task 9 Step 1、Task 9 Step 5）。

---

## 分步导航与回滚约定

| 步 | 任务 | 交付 | 契约 |
|---|---|---|---|
| ① 边界块注入 | Task 1–3 | 作者侧 prompt 无条件带边界块（本步暂不含 scratch 条目）+ 自动续写重试路径 | REQ-PRB-01 / 02 / 03 |
| ② per-session scratch | Task 4–8 | 分区路径、护栏、prepare/cleanup/sweep 三能力；登记 prepare、收口条件删除、启动 sweep；块内 scratch 条目 | REQ-PRB-04 / 05 / 06 / 07 / 08 |
| ③ 检测告警 | Task 9–10 | 纯函数扫描器 + 四稳定码 + `emit_execution_event` 单漏斗接线 | REQ-PRB-09 / 10 / 11 |
| 贯穿验证 | Task 11 | 三步统一回归 + provider 侧零改动核对 + 红线核对 + 残余风险 | REQ-PRB-11 / 12 |

**回滚约定（三步均适用）**：每个任务的提交都是纯增量，落地时整树可编译、既有测试全绿；同一步内每个提交都能在不修改契约、不修改既有测试断言的前提下回滚。第①步的基座提交是 Task 1（Task 2/3 消费其函数）、第②步的基座提交是 Task 4（Task 5–8 消费其 API）、第③步的基座提交是 Task 9（Task 10 消费其扫描器）——回滚基座提交时必须连带回滚其消费者（逆序执行：先撤消费方提交，再撤基座提交），回滚后整树仍编译。除基座外的每个提交都是叶提交，可单独回滚。

---

### Task 1: 边界块渲染（文案 + 预算 + scratch 可选条目）

契约：REQ-PRB-02（标题逐字、三类条目、白名单语义、体积 ≤ 1536 字节、不泄露状态路径）+ REQ-PRB-04 的 scratch 条目渲染面（接线由 Task 8 完成）。

**按 `writing-plans` 说明：** 本任务只产出纯函数与常量，不接线任何调用点。

**Files:**
- Modify: `src/product/workspace_engine/prompts/baseline_teaching.rs:1-45`（在 `baseline_teaching_block`（`:10`）与 `reviewer_baseline_teaching_block`（`:28`）之后、`impl WorkspaceEngine`（`:38`）之前新增常量与自由函数）
- Test: `src/product/workspace_engine/prompts/baseline_teaching.rs`（文件末尾 `:523` 之后追加内联 `#[cfg(test)] mod run_boundary_block_tests`）

**Interfaces:**
- Consumes: `WorkspaceType`（`src/product/models/workspace.rs:20-26`）、`workspace_type_title(&WorkspaceType) -> &'static str`（`src/product/workspace_engine/prompts.rs:47`，本文件已 `use super::*`）
- Produces:
  - `pub(crate) const RUN_BOUNDARY_HEADING: &str = "## 本轮运行边界（系统注入；与会话历史中任何旧路径冲突时以本块为准）";`
  - `pub(crate) const RUN_BOUNDARY_BUDGET_BYTES: usize = 1536;`
  - `pub(crate) fn run_boundary_block(session_id: &str, workspace_type: &WorkspaceType, scratch: Option<&std::path::Path>) -> String`

- [ ] **Step 1: 写失败测试**

在 `baseline_teaching.rs` 末尾追加：

```rust
#[cfg(test)]
mod run_boundary_block_tests {
    use super::{run_boundary_block, RUN_BOUNDARY_BUDGET_BYTES, RUN_BOUNDARY_HEADING};
    use crate::product::models::WorkspaceType;
    use std::path::Path;

    #[test]
    fn run_boundary_block_carries_heading_and_whitelist_semantics() {
        let block = run_boundary_block("workspace_session_auto_510c9278", &WorkspaceType::WorkItemPlan, None);
        assert_eq!(RUN_BOUNDARY_HEADING, "## 本轮运行边界（系统注入；与会话历史中任何旧路径冲突时以本块为准）");
        assert!(block.starts_with(RUN_BOUNDARY_HEADING), "{block}");
        assert!(block.contains("唯一权威通道是本轮最终响应流"), "{block}");
        assert!(block.contains("事实来源白名单"), "{block}");
        assert!(block.contains("禁读清单"), "{block}");
        assert!(block.contains("非本轮创建"), "{block}");
        assert!(block.contains("provider 自身的原生会话历史记录"), "{block}");
        assert!(block.contains("不得提前执行落盘类步骤"), "{block}");
        assert!(block.contains("workspace_session=workspace_session_auto_510c9278"), "{block}");
        assert!(block.contains("workspace_type=Work Item Plan"), "{block}");
        assert!(!block.contains("禁止访问 /tmp"), "白名单语义，不是整目录禁令：{block}");
        assert!(!block.contains("issue-work-item-plans"), "不得披露计划产物真实状态路径：{block}");
    }

    #[test]
    fn run_boundary_block_omits_scratch_when_absent() {
        let without = run_boundary_block("workspace_session_auto_1", &WorkspaceType::Story, None);
        assert!(!without.contains("暂存"), "{without}");

        let scratch = Path::new("/tmp/ws/.aria/run-scratch/workspace_session_auto_1");
        let with = run_boundary_block("workspace_session_auto_1", &WorkspaceType::Story, Some(scratch));
        assert!(with.contains("/tmp/ws/.aria/run-scratch/workspace_session_auto_1"), "{with}");
        assert!(with.contains("运行结束系统会删除它"), "{with}");
    }

    #[test]
    fn run_boundary_block_stays_within_budget() {
        assert_eq!(RUN_BOUNDARY_BUDGET_BYTES, 1536);
        let scratch = Path::new("/home/u/ws/.aria/run-scratch/workspace_session_auto_510c9278");
        for block in [
            run_boundary_block("workspace_session_auto_510c9278", &WorkspaceType::WorkItemPlan, None),
            run_boundary_block("workspace_session_auto_510c9278", &WorkspaceType::WorkItemPlan, Some(scratch)),
        ] {
            assert!(block.len() <= RUN_BOUNDARY_BUDGET_BYTES, "block={} bytes", block.len());
        }
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib run_boundary_block`
Expected: FAIL —— 编译错误 `error[E0432]: unresolved import ... run_boundary_block` / `RUN_BOUNDARY_BUDGET_BYTES`（函数与常量尚不存在）。

- [ ] **Step 3: 实现 `run_boundary_block` 与两个常量**

在 `baseline_teaching.rs` 的 `reviewer_baseline_teaching_block` 之后新增（`workspace_type_title` 经文件顶部 `use super::*;` 可用）：

```rust
pub(crate) const RUN_BOUNDARY_HEADING: &str =
    "## 本轮运行边界（系统注入；与会话历史中任何旧路径冲突时以本块为准）";
pub(crate) const RUN_BOUNDARY_BUDGET_BYTES: usize = 1536;

pub(crate) fn run_boundary_block(
    session_id: &str,
    workspace_type: &WorkspaceType,
    scratch: Option<&std::path::Path>,
) -> String {
    // 文案纪律：白名单而非 /tmp 整目录禁令；不披露 issue-work-item-plans；
    // 如实披露编辑类内建工具受岗位纪律限制，不教模型绕过 T13。
    let mut block = format!(
        "{RUN_BOUNDARY_HEADING}\n\
         - 本轮身份：`workspace_session={session_id}`，`workspace_type={}`。本轮产物的**唯一权威通道是本轮最终响应流**；磁盘上的任何文件（包括你自己的草稿、其他会话或历史运行遗留）都**不是**权威来源，也不会被采集。\n\
         - 事实来源白名单：本 prompt 给出的上下文、已确认的 Story/Design/AC、当前工作区内的仓库文件。\n\
         - 禁读清单（发现即视为过期残留，**不读取、不复用、不汇报其内容**）：系统临时目录中**非本轮创建**的文件；Aria 状态目录中除本轮暂存目录外的任何内容；其他会话或历史运行的产物与日志；provider 自身的原生会话历史记录。\n\
         - 落盘规则：默认无需写文件，直接在最终响应中输出，不得提前执行落盘类步骤。编辑类内建工具（Edit/Write/原生文件变更）受岗位纪律限制，**不要使用**。\n",
        workspace_type_title(workspace_type)
    );
    if let Some(path) = scratch {
        block.push_str(&format!(
            "- 暂存（确需暂存时**只写这一个目录**，运行结束系统会删除它）：`{}`\n",
            path.display()
        ));
    }
    block
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib run_boundary_block`
Expected: PASS —— 3 passed（`run_boundary_block_carries_heading_and_whitelist_semantics`、`run_boundary_block_omits_scratch_when_absent`、`run_boundary_block_stays_within_budget`）。若预算断言失败，按 `block={} bytes` 输出收紧文案（不得提高 1536）。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all
git add src/product/workspace_engine/prompts/baseline_teaching.rs
git commit -m "feat(run-boundary): 新增本轮运行边界块渲染与字节预算"
```

回滚：本提交是第①步基座（Task 2/3 消费 `run_boundary_block`），单独回滚需先回滚 Task 3、Task 2；仅回滚本提交时除这两处外无其他引用，整树仍可编译。

---

### Task 2: 作者侧漏斗无条件注入

契约：REQ-PRB-01（无条件注入、覆盖聚合/LC 与无基线）、REQ-PRB-03（三种构造路径覆盖、reviewer 不注入、至多一次）。

**Files:**
- Modify: `src/product/workspace_engine/prompts/baseline_teaching.rs:38-90`（`impl WorkspaceEngine` 内新增 `append_author_run_boundary`；在 `append_author_baseline_teaching`（`:77`）的**首行**调用它，即在 `resolve_issue_baseline_tree()` 之前）
- Test: `src/product/workspace_engine/prompts/baseline_teaching.rs:523` 之后（新增 `#[cfg(test)] mod author_run_boundary_tests`，复用 `baseline_teaching_fixture`（`:106`）与 `author_baseline_tests`（`:272`）已有夹具）

**Interfaces:**
- Consumes: `run_boundary_block`（Task 1）；`WorkspaceEngine.lifecycle_store: Option<LifecycleStore>`、`session.session_id`、`session.workspace_type`
- Produces: `pub(super) fn append_author_run_boundary(&self, prompt: &mut String)`（无返回值；无 store 时直接返回）；`append_author_baseline_teaching` 签名不变：`fn (&self, &mut String) -> Result<Option<BaselineTreeRef>, String>`

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod author_run_boundary_tests {
    use super::baseline_teaching_fixture::{baseline_engine, lc_plan_engine_on_bare_directory};
    use super::RUN_BOUNDARY_HEADING;
    use crate::product::models::{ProviderName, WorkspaceType};
    use crate::product::workspace_engine::types::{AuthorPromptMode, SessionMessage};

    #[test]
    fn author_run_boundary_injected_for_all_four_author_types() {
        for workspace_type in [
            WorkspaceType::Story,
            WorkspaceType::Design,
            WorkspaceType::WorkItem,
            WorkspaceType::WorkItemPlan,
        ] {
            let fixture = baseline_engine("main", None, workspace_type.clone());
            let input = fixture
                .engine
                .build_streaming_input("生成", AuthorPromptMode::FullConversation)
                .expect("author prompt");
            assert_eq!(input.prompt.matches(RUN_BOUNDARY_HEADING).count(), 1, "{workspace_type:?}");
        }
    }

    #[test]
    fn author_run_boundary_injected_for_aggregate_session_without_baseline() {
        let mut fixture = lc_plan_engine_on_bare_directory();
        fixture.engine.session.messages.push(SessionMessage {
            id: "msg_aggregate_scope".to_string(),
            role: "system".to_string(),
            content: "## 聚合代码库成员清单\n- busi\n- api".to_string(),
            checkpoint_id: None,
            created_at: "2026-10-27T00:00:00Z".to_string(),
        });
        let input = fixture
            .engine
            .build_streaming_input("生成", AuthorPromptMode::FullConversation)
            .expect("LC aggregate session prompt");
        assert_eq!(input.prompt.matches(RUN_BOUNDARY_HEADING).count(), 1);
        assert!(!input.prompt.contains("基准分支基线（issue 基线 ="), "聚合会话无基线教学块：{}", input.prompt);
    }

    #[test]
    fn author_run_boundary_injected_once_with_prior_session_history() {
        let mut fixture = baseline_engine("main", None, WorkspaceType::WorkItemPlan);
        fixture.engine.session.messages.push(SessionMessage {
            id: "msg_prior".to_string(),
            role: "assistant".to_string(),
            content: "上一轮草稿已写入 /tmp/work-item-plan.md，可复用。".to_string(),
            checkpoint_id: None,
            created_at: "2026-10-27T00:00:00Z".to_string(),
        });
        let input = fixture
            .engine
            .build_streaming_input("继续", AuthorPromptMode::FullConversation)
            .expect("author prompt");
        assert_eq!(input.prompt.matches(RUN_BOUNDARY_HEADING).count(), 1, "至多一次");
    }

    #[test]
    fn in_memory_engine_prompt_omits_run_boundary_block() {
        // 无持久 store 的内存态 engine：session 记录由临时 store 铸造，engine 不持有 store
        // （`WorkspaceEngine::new` 的 `lifecycle_store: None`，src/product/workspace_engine/lifecycle.rs:306-314）。
        let tmp = tempfile::tempdir().expect("tmp");
        let store = crate::product::lifecycle_store::LifecycleStore::new(
            crate::product::app_paths::ProductAppPaths::new(tmp.path().join(".aria")),
        );
        let record = store
            .create_workspace_session(crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "story_spec_0001".to_string(),
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::Codex),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("create session");
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
        let engine = crate::product::workspace_engine::types::WorkspaceEngine::new(
            std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                tmp.path().join("checkpoints"),
            )),
            event_tx,
            crate::product::workspace_engine::types::WorkspaceSession::from_record(record),
        );
        let input = engine
            .build_streaming_input("生成", AuthorPromptMode::FullConversation)
            .expect("in-memory prompt");
        assert!(!input.prompt.contains(RUN_BOUNDARY_HEADING), "{}", input.prompt);
    }

    #[test]
    fn reviewer_prompt_omits_run_boundary_block() {
        let fixture = baseline_engine("main", None, WorkspaceType::Story);
        let mut prompt = String::new();
        fixture
            .engine
            .append_reviewer_baseline_teaching(&mut prompt)
            .expect("reviewer baseline");
        assert!(prompt.contains("基准分支基线（issue 基线 = main）"), "{prompt}");
        assert!(!prompt.contains(RUN_BOUNDARY_HEADING), "{prompt}");
    }
}
```

配套无需新增夹具：`baseline_engine`（`:117`）与 `lc_plan_engine_on_bare_directory`（`:183`）已足够；内存态用例在用例内自建临时 store 与 `WorkspaceEngine::new`（`WorkspaceSession` 不是 `Clone`，不能复用夹具的 `engine.session`）。

- [ ] **Step 2: 运行测试确认失败**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib author_run_boundary`
Expected: FAIL —— `assertion failed: ... matches(...).count() == 1`（当前 prompt 无边界块），5 个用例中前 3 个失败、后 2 个「不包含」断言本就通过。

- [ ] **Step 3: 实现注入**

在 `baseline_teaching.rs` 的 `impl WorkspaceEngine` 内新增，并在 `append_author_baseline_teaching` 首行调用：

```rust
    /// REQ-PRB-01/02：本轮运行边界块（无条件注入，先于基线解析）。
    /// 无持久 store 的内存态 engine 整块省略（与基线教学块跳过面同构，
    /// 保证既有 prompt 预算断言字节不变）。
    pub(super) fn append_author_run_boundary(&self, prompt: &mut String) {
        if self.lifecycle_store.is_none() {
            return;
        }
        prompt.push_str(&run_boundary_block(
            &self.session.session_id,
            &self.session.workspace_type,
            None,
        ));
    }

    pub(super) fn append_author_baseline_teaching(
        &self,
        prompt: &mut String,
    ) -> Result<Option<crate::cross_cutting::streaming_provider::BaselineTreeRef>, String> {
        self.append_author_run_boundary(prompt);
        let baseline_tree = self.resolve_issue_baseline_tree()?;
        // …既有实现不变…
    }
```

三个作者漏斗（`src/product/workspace_engine/prompts.rs:373`、`prompts.rs:505`、`src/product/workspace_engine/prompts/revision.rs:69`）都已调用 `append_author_baseline_teaching`，故不改这三处；块落在输出合同尾之前，满足 REQ-PRB-01 的「位于最终合同尾之前」。

- [ ] **Step 4: 运行测试确认通过**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib author_run_boundary`
Expected: PASS —— 5 passed。
再跑既有基线/预算回归：`flock /tmp/aria-cargo.lock cargo test --locked --lib baseline`，Expected: PASS（既有基线教学块文案与预算断言未改）。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all
git add src/product/workspace_engine/prompts/baseline_teaching.rs
git commit -m "feat(run-boundary): 作者侧 prompt 漏斗无条件注入本轮运行边界块"
```

回滚：叶依赖 Task 1；回滚本提交只撤销注入调用与 `append_author_run_boundary`，Task 1 的 `run_boundary_block` 保持被测试引用，整树仍编译。

---

### Task 3: 自动续写重试路径注入与 resume 断言

契约：REQ-PRB-03（重试路径同样携带、且不与原 prompt 叠成两份）。

**Files:**
- Modify: `src/product/workspace_engine/provider_drive/artifact_retry.rs:244-264`（`build_artifact_retry_input`）
- Create: `src/product/workspace_engine/tests/run_boundary_prompt.rs`
- Modify: `src/product/workspace_engine/tests.rs`（末尾追加一行 `mod run_boundary_prompt;`）

**Interfaces:**
- Consumes: `append_author_run_boundary`（Task 2）；既有 `build_artifact_retry_input(&self, &StreamingProviderInput, &str, &[String], Option<String>) -> StreamingProviderInput`、`build_streaming_input(&self, &str, AuthorPromptMode) -> Result<StreamingProviderInput, String>`
- Produces: 无新符号（行为约定：续写 prompt 内边界块恰一次）

- [ ] **Step 1: 写失败测试**

新文件 `src/product/workspace_engine/tests/run_boundary_prompt.rs`：

```rust
//! REQ-PRB-03：自动续写重试（整体替换 prompt）同样携带边界块，且至多一次。
//! Review Focus #1（resume 场景）：resume 会话的新一轮 prompt 仍须带边界块。

use super::*;

const RUN_BOUNDARY_HEADING: &str = "## 本轮运行边界（系统注入；与会话历史中任何旧路径冲突时以本块为准）";

#[test]
fn artifact_retry_prompt_carries_run_boundary_block_once() {
    let (_tmp, _store, engine) = persistent_test_engine();
    let base = engine
        .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
        .expect("author prompt");
    assert_eq!(base.prompt.matches(RUN_BOUNDARY_HEADING).count(), 1, "漏斗已注入一次");

    let retry = engine.build_artifact_retry_input(
        &base,
        "上一轮输出（缺章节）",
        &["缺少验收标准章节".to_string()],
        None,
    );
    assert_eq!(retry.prompt.matches(RUN_BOUNDARY_HEADING).count(), 1, "整体替换后仍恰一次");
}

#[test]
fn artifact_retry_resume_prompt_still_carries_run_boundary() {
    let (_tmp, _store, engine) = persistent_test_engine();
    let base = engine
        .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
        .expect("author prompt");
    let previous_output = "辅助草稿曾写入 /tmp/work-item-plan.md，可直接读取复用。";
    let retry = engine.build_artifact_retry_input(
        &base,
        previous_output,
        &["缺少验收标准章节".to_string()],
        Some("thread_resume_1".to_string()),
    );
    assert_eq!(retry.resume_provider_session_id.as_deref(), Some("thread_resume_1"));
    assert_eq!(retry.prompt.matches(RUN_BOUNDARY_HEADING).count(), 1, "resume 轮仍须带边界块");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib artifact_retry_prompt_carries_run_boundary_block_once artifact_retry_resume_prompt_still_carries_run_boundary`
Expected: FAIL —— `assertion left: 0 right: 1`（续写 prompt 为整体替换，未追加边界块）。

- [ ] **Step 3: 实现**

在 `build_artifact_retry_input` 里 `input.prompt = build_artifact_retry_prompt(...)` 之后追加一行调用：

```rust
        input.prompt = build_artifact_retry_prompt(
            &self.session.workspace_type,
            previous_output,
            blocking_reasons,
        );
        self.append_author_run_boundary(&mut input.prompt);
```

- [ ] **Step 4: 运行测试确认通过**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib run_boundary_prompt`
Expected: PASS —— 2 passed。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all
git add src/product/workspace_engine/provider_drive/artifact_retry.rs src/product/workspace_engine/tests/run_boundary_prompt.rs src/product/workspace_engine/tests.rs
git commit -m "feat(run-boundary): 自动续写重试路径同样注入边界块"
```

回滚：叶提交（无后续任务依赖本提交的符号）；单独回滚后整树仍编译、`run_boundary_prompt.rs` 两个用例随之移除。

---

### Task 4: scratch 分区路径、删除护栏与 prepare / cleanup / sweep 三能力

契约：REQ-PRB-04（位置与权限）、REQ-PRB-05（创建/清空/存活准入）、REQ-PRB-06（幂等清理）、REQ-PRB-07（孤儿回收规则）、REQ-PRB-08（全部删除带护栏）。

**Files:**
- Modify: `src/product/app_paths.rs:13-24`（`root()` / `projects_root()` 之后新增常量与两个路径方法）
- Create: `src/product/lifecycle_store/run_scratch.rs`
- Modify: `src/product/lifecycle_store/mod.rs:1-21`（模块声明区追加 `pub mod run_scratch;`，与 `pub mod tool_policy_run_audit;`（`:9`）并列）
- Test: `src/product/lifecycle_store/run_scratch.rs`（内联 `#[cfg(test)] mod`；`lifecycle_store/tests.rs` 已无法再加 `include!`）

**Interfaces:**
- Consumes: `ProductAppPaths::root()`（`app_paths.rs:13`）、`validate_relative_id`（`src/product/json_store.rs:109`）、`with_workspace_session_live_write`（`src/product/lifecycle_store/tool_policy_run_audit.rs:238`）、`find_workspace_session_path`（`workspace.rs:1230`）、`read_workspace_session_record`（`utils.rs:76`）、`remove_dir_all_if_exists`（`utils.rs:170`）、`PROVIDER_IDLE_WATCHDOG_TIMEOUT`（`provider_drive/watchdog.rs:22-23`，经 `provider_drive.rs:24-27` re-export）
- Produces:
  - `pub const RUN_SCRATCH_PARTITION: &str = "run-scratch";`（`app_paths.rs`）
  - `impl ProductAppPaths { pub fn run_scratch_root(&self) -> PathBuf; pub fn run_scratch_dir(&self, workspace_session_id: &str) -> PathBuf; }`（纯路径，不校验 id，同层既有方法口径）
  - `pub const RUN_SCRATCH_ORPHAN_TTL: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);`（`run_scratch.rs`）
  - `#[derive(Debug, PartialEq, Eq)] pub enum RunScratchRemoval { Removed, Absent, Skipped }`
  - `impl LifecycleStore { pub fn run_scratch_dir(&self, workspace_session_id: &str) -> PathBuf; pub fn prepare_run_scratch(&self, workspace_session_id: &str) -> Result<(), ProductStoreError>; pub fn cleanup_run_scratch(&self, workspace_session_id: &str) -> Result<RunScratchRemoval, ProductStoreError>; pub fn sweep_orphan_run_scratch(&self, orphan_ttl: std::time::Duration) -> Result<usize, ProductStoreError>; }`
  - 内部：`impl LifecycleStore { fn remove_run_scratch_dir(&self, workspace_session_id: &str) -> Result<RunScratchRemoval, ProductStoreError>; }`（唯一删除入口）

- [ ] **Step 1: 写失败测试（路径 + 护栏 + prepare + cleanup）**

`run_scratch.rs` 内联测试模块（同一文件先建骨架，测试先行）：

```rust
#[cfg(test)]
mod run_scratch_tests {
    use super::{RunScratchRemoval, RUN_SCRATCH_ORPHAN_TTL};
    use crate::product::app_paths::{ProductAppPaths, RUN_SCRATCH_PARTITION};
    use crate::product::lifecycle_store::LifecycleStore;
    use std::os::unix::fs::PermissionsExt;

    fn store() -> (tempfile::TempDir, LifecycleStore) {
        let tmp = tempfile::tempdir().expect("tmp");
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        (tmp, LifecycleStore::new(paths))
    }

    #[test]
    fn run_scratch_dir_lives_under_state_root_not_repo() {
        let (_tmp, store) = store();
        let paths = store.app_paths();
        let scratch = paths.run_scratch_dir("workspace_session_auto_1");
        assert_eq!(RUN_SCRATCH_PARTITION, "run-scratch");
        assert_eq!(scratch, paths.root().join("run-scratch").join("workspace_session_auto_1"));
        assert!(scratch.starts_with(paths.root()));
        assert!(!scratch.starts_with(paths.projects_root()));
        assert_eq!(store.run_scratch_dir("workspace_session_auto_1"), scratch);
    }

    #[test]
    fn gitignore_covers_aria_state_root() {
        // Review Focus 之外的候选：scratch 不得进 git status（.gitignore:3 `.aria/`）。
        let gitignore = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/.gitignore"));
        assert!(gitignore.lines().any(|line| line.trim() == ".aria/"), "{gitignore}");
    }

    #[test]
    fn remove_run_scratch_dir_removes_only_partition_child() {
        let (_tmp, store) = store();
        let dir = store.run_scratch_dir("workspace_session_auto_1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("draft.md"), "x").unwrap();
        let sibling = store.app_paths().run_scratch_root().join("keep.txt");
        std::fs::write(&sibling, "keep").unwrap();

        assert_eq!(store.cleanup_run_scratch("workspace_session_auto_1").unwrap(), RunScratchRemoval::Removed);
        assert!(!dir.exists());
        assert!(sibling.exists(), "分区内 sibling 非本能力所有，不得删除");
        assert!(store.app_paths().run_scratch_root().exists(), "分区根永不删");
    }

    #[test]
    fn remove_run_scratch_dir_refuses_illegal_and_symlinked_target() {
        let (tmp, store) = store();
        assert_eq!(store.cleanup_run_scratch("../escape").unwrap(), RunScratchRemoval::Skipped);

        let external = tmp.path().join("external");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join("keep.md"), "keep").unwrap();
        let dir = store.run_scratch_dir("workspace_session_auto_2");
        std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&external, &dir).unwrap();
        assert_eq!(store.cleanup_run_scratch("workspace_session_auto_2").unwrap(), RunScratchRemoval::Skipped);
        assert!(external.join("keep.md").exists(), "符号链接目标不得被删");
        std::fs::remove_file(&dir).unwrap();

        let file_target = store.run_scratch_dir("workspace_session_auto_3");
        std::fs::write(&file_target, "not a dir").unwrap();
        assert_eq!(store.cleanup_run_scratch("workspace_session_auto_3").unwrap(), RunScratchRemoval::Skipped);
        assert!(file_target.exists(), "非目录目标不得删除");
    }

    #[test]
    fn cleanup_is_idempotent_and_reports_absent() {
        let (_tmp, store) = store();
        assert_eq!(store.cleanup_run_scratch("workspace_session_auto_4").unwrap(), RunScratchRemoval::Absent);
    }

    #[test]
    fn prepare_run_scratch_clears_previous_content() {
        let (_tmp, store) = store();
        let dir = store.run_scratch_dir("workspace_session_auto_5");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("old.md"), "前序 run 残留").unwrap();

        store.prepare_run_scratch("workspace_session_auto_5").unwrap();
        assert!(dir.is_dir());
        assert!(!dir.join("old.md").exists(), "新 run 不得看见前序内容");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn prepare_run_scratch_sets_0700_permissions() {
        let (_tmp, store) = store();
        store.prepare_run_scratch("workspace_session_auto_6").unwrap();
        let mode = std::fs::metadata(store.run_scratch_dir("workspace_session_auto_6"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn prepare_run_scratch_refused_for_purging_session() {
        // Review Focus #2：purging / deleted 的 session 不得因 prepare 复活 scratch。
        use crate::product::entity_deletion_store::{DeletionEntityKind, EntityDeletionStore, EntityScope};
        use crate::product::entity_operation_store::EntityOperationStore;
        use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
        use crate::product::models::{ProviderName, WorkspaceType};

        let (_tmp, store) = store();
        let record = store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "story_spec_0001".to_string(),
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::Codex),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("create session");
        let scope = EntityScope::new("project_0001", "issue_0001", DeletionEntityKind::Story, "story_spec_0001");
        let lease = EntityOperationStore::new(store.app_paths())
            .try_acquire_delete(&scope)
            .expect("acquire delete lease");
        EntityDeletionStore::new(store.app_paths())
            .begin(&lease)
            .expect("begin purging tombstone");

        let result = store.prepare_run_scratch(&record.id);

        assert!(result.is_err(), "purging 会话的 prepare 必须被准入拒绝");
        assert!(!store.run_scratch_dir(&record.id).exists(), "不得复活 scratch 目录");
        let _ = EntityDeletionStore::new(store.app_paths()).complete(&lease);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib run_scratch`
Expected: FAIL —— `error[E0432]: unresolved import crate::product::lifecycle_store::run_scratch`（模块与 API 尚不存在）。

- [ ] **Step 3: 实现路径方法 + 护栏 + prepare + cleanup**

`app_paths.rs`（`projects_root()` 之后）：

```rust
/// 与 `tool-policy-run-audit/`、`work-item-split-runs/` 并列的 per-session 暂存分区。
pub const RUN_SCRATCH_PARTITION: &str = "run-scratch";

impl ProductAppPaths {
    pub fn run_scratch_root(&self) -> PathBuf {
        self.root().join(RUN_SCRATCH_PARTITION)
    }

    /// 纯路径计算，不做 id 校验（同层既有方法口径）。
    pub fn run_scratch_dir(&self, workspace_session_id: &str) -> PathBuf {
        self.run_scratch_root().join(workspace_session_id)
    }
}
```

`run_scratch.rs`（新文件；`mod.rs` 加 `pub mod run_scratch;`）：

```rust
//! REQ-PRB-04/05/06/07/08：per-session scratch 分区与三段式生命周期。
//!
//! 位置：`<state>/run-scratch/<workspace_session_id>/`（`<state>` = `ProductAppPaths::root()`），
//! 0700，位于仓库工作区与 cwd 之外。删除一律经 `remove_run_scratch_dir` 的单入口护栏。

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use crate::product::json_store::{validate_relative_id, ProductStoreError};
use crate::product::models::WorkspaceSessionStatus;

use super::utils::{read_workspace_session_record, remove_dir_all_if_exists};
use super::LifecycleStore;

/// owner D6：孤儿 scratch TTL = 6 小时。
pub const RUN_SCRATCH_ORPHAN_TTL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, PartialEq, Eq)]
pub enum RunScratchRemoval {
    Removed,
    Absent,
    Skipped,
}

impl LifecycleStore {
    pub fn run_scratch_dir(&self, workspace_session_id: &str) -> PathBuf {
        self.app_paths().run_scratch_dir(workspace_session_id)
    }

    /// 唯一删除入口：只删分区下经 id 校验的直接子目录，护栏见设计 §5.4。
    fn remove_run_scratch_dir(
        &self,
        workspace_session_id: &str,
    ) -> Result<RunScratchRemoval, ProductStoreError> {
        if validate_relative_id(workspace_session_id).is_err() {
            tracing::warn!(session_id = %workspace_session_id, "run scratch skip: illegal id");
            return Ok(RunScratchRemoval::Skipped);
        }
        let root = self.app_paths().run_scratch_root();
        let target = root.join(workspace_session_id);
        let metadata = match std::fs::symlink_metadata(&target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(RunScratchRemoval::Absent)
            }
            Err(error) => return Err(ProductStoreError::Io(format!("symlink_metadata {}: {error}", target.display()))),
        };
        if !metadata.file_type().is_dir() {
            tracing::warn!(path = %target.display(), "run scratch skip: not a real directory");
            return Ok(RunScratchRemoval::Skipped);
        }
        // 路径漂移核对：目标 canonical 的父目录必须仍是 canonical 分区根。
        let canonical_root = match std::fs::canonicalize(&root) {
            Ok(root) => root,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(RunScratchRemoval::Absent)
            }
            Err(error) => return Err(ProductStoreError::Io(format!("canonicalize {}: {error}", root.display()))),
        };
        let canonical_parent = match std::fs::canonicalize(&target) {
            Ok(path) => path.parent().map(std::path::Path::to_path_buf),
            Err(error) => return Err(ProductStoreError::Io(format!("canonicalize {}: {error}", target.display()))),
        };
        if canonical_parent.as_deref() != Some(canonical_root.as_path()) {
            tracing::warn!(path = %target.display(), "run scratch skip: path drift");
            return Ok(RunScratchRemoval::Skipped);
        }
        // fs::remove_dir_all 不跟随符号链接：目录内链接只删链接本身。
        remove_dir_all_if_exists(&target)?;
        Ok(RunScratchRemoval::Removed)
    }

    /// REQ-PRB-05：清空旧内容 → 重建 → 0700；经 session 存活准入，
    /// purging/deleted 的 session 不复活目录。
    pub fn prepare_run_scratch(&self, workspace_session_id: &str) -> Result<(), ProductStoreError> {
        validate_relative_id(workspace_session_id)?;
        self.with_workspace_session_live_write(workspace_session_id, || {
            let target = self.run_scratch_dir(workspace_session_id);
            match self.remove_run_scratch_dir(workspace_session_id)? {
                RunScratchRemoval::Skipped => {
                    tracing::warn!(path = %target.display(), "run scratch prepare skip: symlink or drift");
                    return Ok(());
                }
                RunScratchRemoval::Removed | RunScratchRemoval::Absent => {}
            }
            std::fs::create_dir_all(&target)
                .map_err(|error| ProductStoreError::Io(format!("create_dir_all {}: {error}", target.display())))?;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| ProductStoreError::Io(format!("set_permissions {}: {error}", target.display())))?;
            Ok(())
        })
    }

    /// REQ-PRB-06 的幂等清理能力（条件删除由 manager 侧判定）。
    pub fn cleanup_run_scratch(
        &self,
        workspace_session_id: &str,
    ) -> Result<RunScratchRemoval, ProductStoreError> {
        self.remove_run_scratch_dir(workspace_session_id)
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib run_scratch`
Expected: PASS —— `run_scratch_tests` 的 8 个用例全绿（`RunScratchRemoval` 断言为 `Removed` / `Absent` / `Skipped`）。

- [ ] **Step 5: 写 sweep 失败测试（三类孤儿 + 活 run 保留 + 非法名 + 读 spy + TTL 关系）**

```rust
    #[test]
    fn sweep_orphan_run_scratch_removes_three_orphan_classes() {
        let (_tmp, store) = store();
        // ① session 记录不存在
        let ghost = store.run_scratch_dir("workspace_session_auto_ghost");
        std::fs::create_dir_all(&ghost).unwrap();
        // ② 终态 session
        let confirmed = create_session(&store, WorkspaceSessionStatus::Confirmed);
        let confirmed_dir = store.run_scratch_dir(&confirmed);
        std::fs::create_dir_all(&confirmed_dir).unwrap();
        // ③ 超 TTL 的活 session（用可注入 ttl = ZERO 表达年龄规则）
        let running = create_session(&store, WorkspaceSessionStatus::Running);
        let running_dir = store.run_scratch_dir(&running);
        std::fs::create_dir_all(&running_dir).unwrap();

        let removed = store.sweep_orphan_run_scratch(Duration::ZERO).unwrap();
        assert_eq!(removed, 3);
        assert!(!ghost.exists() && !confirmed_dir.exists() && !running_dir.exists());
    }

    #[test]
    fn sweep_orphan_run_scratch_keeps_live_run_scratch() {
        // Review Focus #4：活 run（session 存活、状态非终态、mtime 在 TTL 内）不得被回收。
        let (_tmp, store) = store();
        for status in [WorkspaceSessionStatus::Running, WorkspaceSessionStatus::WaitingForHuman] {
            let id = create_session(&store, status.clone());
            std::fs::create_dir_all(store.run_scratch_dir(&id)).unwrap();
        }
        let removed = store.sweep_orphan_run_scratch(RUN_SCRATCH_ORPHAN_TTL).unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn sweep_orphan_run_scratch_skips_illegal_names_and_keeps_processing() {
        let (_tmp, store) = store();
        std::fs::create_dir_all(store.app_paths().run_scratch_root()).unwrap();
        std::fs::create_dir_all(store.app_paths().run_scratch_root().join("..hidden")).unwrap();
        let ghost = store.run_scratch_dir("workspace_session_auto_ghost2");
        std::fs::create_dir_all(&ghost).unwrap();

        let removed = store.sweep_orphan_run_scratch(RUN_SCRATCH_ORPHAN_TTL).unwrap();
        assert_eq!(removed, 1);
        assert!(store.app_paths().run_scratch_root().join("..hidden").exists(), "非法名跳过不删");
    }

    #[test]
    fn sweep_orphan_run_scratch_does_not_trigger_session_read_spy() {
        // Review Focus #2：sweep 必须走 find_workspace_session_path + read_workspace_session_record。
        let (_tmp, store) = store();
        let id = create_session(&store, WorkspaceSessionStatus::Running);
        std::fs::create_dir_all(store.run_scratch_dir(&id)).unwrap();
        crate::product::lifecycle_store::workspace_session_read_spy::reset_workspace_session_read_spy();
        crate::product::lifecycle_store::workspace_session_read_spy::set_workspace_session_read_panic(true);

        let removed = store.sweep_orphan_run_scratch(RUN_SCRATCH_ORPHAN_TTL).unwrap();

        crate::product::lifecycle_store::workspace_session_read_spy::set_workspace_session_read_panic(false);
        assert_eq!(removed, 0);
        assert_eq!(
            crate::product::lifecycle_store::workspace_session_read_spy::workspace_session_read_count(),
            0,
            "sweep 不得调用 get_workspace_session"
        );
        assert!(store.run_scratch_dir(&id).exists());
    }

    #[test]
    fn run_scratch_orphan_ttl_exceeds_provider_idle_watchdog() {
        // 实现期核实项（设计 §11.3）：TTL 须大于 watchdog 零活动阈值（watchdog.rs:22-23 = 1800s）。
        assert!(RUN_SCRATCH_ORPHAN_TTL
            > crate::product::workspace_engine::provider_drive::PROVIDER_IDLE_WATCHDOG_TIMEOUT);
    }
```

配套测试夹具（同模块内）：

```rust
    /// 建一个真实 session 记录（持久 store），返回其 session_id。
    fn create_session(store: &LifecycleStore, status: WorkspaceSessionStatus) -> String {
        use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
        use crate::product::models::{ProviderName, WorkspaceType};
        let record = store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "story_spec_0001".to_string(),
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::Codex),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("create session");
        if record.status != status {
            store
                .update_workspace_session_status(&record.id, status)
                .expect("set session status");
        }
        record.id
    }
```

> 说明：状态写入用既有 `update_workspace_session_status(&self, &str, WorkspaceSessionStatus)`（`src/product/lifecycle_store/workspace.rs:720`）；不得为此修改 `workspace.rs`（行数红线 1730/1730）。

- [ ] **Step 6: 运行测试确认失败**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib sweep_orphan_run_scratch`
Expected: FAIL —— `error[E0599]: no method named sweep_orphan_run_scratch found for struct LifecycleStore`。

- [ ] **Step 7: 实现 sweep**

`run_scratch.rs` 追加：

```rust
    /// REQ-PRB-07：启动后一次孤儿回收。只处理分区直接子目录；三类条件
    /// （session 记录不存在 / 终态 / mtime 超 TTL）任一成立即删。
    /// 单项失败只 warn，不影响其余项；失败不阻塞调用方。
    pub fn sweep_orphan_run_scratch(&self, orphan_ttl: Duration) -> Result<usize, ProductStoreError> {
        let root = self.app_paths().run_scratch_root();
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(ProductStoreError::Io(format!("read_dir {}: {error}", root.display()))),
        };
        let mut removed = 0usize;
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    tracing::warn!(error = %error, "run scratch sweep: read_dir entry failed");
                    continue;
                }
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            if validate_relative_id(&name).is_err() {
                tracing::warn!(name = %name, "run scratch sweep: illegal name skipped");
                continue;
            }
            if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                tracing::warn!(name = %name, "run scratch sweep: not a real directory skipped");
                continue;
            }
            if !self.run_scratch_is_orphan(&name, &entry.path(), orphan_ttl) {
                continue;
            }
            match self.remove_run_scratch_dir(&name) {
                Ok(RunScratchRemoval::Removed) => removed += 1,
                Ok(_) => {}
                Err(error) => tracing::warn!(name = %name, error = %error, "run scratch sweep: removal failed"),
            }
        }
        Ok(removed)
    }

    fn run_scratch_is_orphan(&self, session_id: &str, dir: &std::path::Path, ttl: Duration) -> bool {
        // 会话记录是否存在：必须走 find_workspace_session_path，不得走 get_workspace_session。
        let session = match self.find_workspace_session_path(session_id) {
            Ok(path) => read_workspace_session_record(&path).ok().flatten(),
            Err(ProductStoreError::NotFound { .. }) => None,
            Err(error) => {
                tracing::warn!(session_id = %session_id, error = %error, "run scratch sweep: session lookup failed");
                return false;
            }
        };
        match session {
            None => true,
            Some(record) => match record.status {
                WorkspaceSessionStatus::Confirmed
                | WorkspaceSessionStatus::Terminated
                | WorkspaceSessionStatus::Failed => true,
                _ => {
                    let age = std::fs::metadata(dir)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| std::time::SystemTime::now().duration_since(modified).ok())
                        .unwrap_or_default();
                    age > ttl
                }
            },
        }
    }
```

- [ ] **Step 8: 运行测试确认通过**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib sweep_orphan_run_scratch`
Expected: PASS —— 5 passed。再跑 `flock /tmp/aria-cargo.lock cargo test --locked --lib run_scratch`，Expected: PASS（12 passed，含路径/护栏/prepare/cleanup/TTL 关系）。

- [ ] **Step 9: 提交**

```bash
cargo fmt --all
git add src/product/app_paths.rs src/product/lifecycle_store/mod.rs src/product/lifecycle_store/run_scratch.rs
git commit -m "feat(run-boundary): 新增 per-session scratch 分区、删除护栏与三能力"
```

回滚：本提交是第②步基座（Task 5/6/7/8 消费其 API）；单独回滚需先回滚 Task 8、Task 7、Task 6、Task 5（逆序），回滚后整树仍编译、既有测试全绿。

---

### Task 5: run 登记时 prepare（两条登记路径）

契约：REQ-PRB-05（登记后清空重建、supersede 与 claim-if-idle 行为一致、同 run 内重启不清空、失败只 warn）。

**Files:**
- Modify: `src/web/workspace_session/manager/runs.rs:140-155`（`start_run_from_attachment` 返回 `Ok(run)` 之前）与 `:344-352`（`try_start_run_if_idle` 返回 `Ok(Some(...))` 之前）
- Test: `src/web/workspace_session/manager/runs.rs:701` 之后（新增 `#[cfg(test)] mod manager_run_scratch_tests`）

**Interfaces:**
- Consumes: `LifecycleStore::prepare_run_scratch`（Task 4）、`WorkspaceSessionManager.app_paths: ProductAppPaths`、`WorkspaceSessionManager.session_id: String`、既有 `replace_command_tx_if_token(&self, u64, mpsc::Sender<ProviderCommand>)`
- Produces: 无新符号

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod manager_run_scratch_tests {
    use super::WorkspaceSessionManager;
    use crate::product::lifecycle_store::LifecycleStore;
    use crate::web::workspace_ws_handler::ProviderRunKind;

    fn scratch_of(manager: &WorkspaceSessionManager) -> std::path::PathBuf {
        LifecycleStore::new(manager.app_paths.clone()).run_scratch_dir(&manager.session_id)
    }

    #[tokio::test]
    async fn start_run_from_attachment_prepares_run_scratch() {
        let manager = WorkspaceSessionManager::test_fixture("n1-run-scratch-supersede");
        let scratch = scratch_of(&manager);
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join("old.md"), "前序 run 残留").unwrap();

        let _ = manager
            .start_run(ProviderRunKind::WorkItemPlanBatch, None)
            .await
            .expect("start run");

        assert!(scratch.is_dir());
        assert!(!scratch.join("old.md").exists(), "新 run 不得看见前序内容");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[tokio::test]
    async fn try_start_run_if_idle_prepares_run_scratch() {
        let manager = WorkspaceSessionManager::test_fixture("n1-run-scratch-idle");
        let scratch = scratch_of(&manager);
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join("old.md"), "前序 run 残留").unwrap();

        let started = manager
            .try_start_run_if_idle(ProviderRunKind::WorkItemPlanBatch, None)
            .await
            .expect("claim if idle")
            .is_some();
        assert!(started);
        assert!(!scratch.join("old.md").exists());
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[tokio::test]
    async fn restart_within_run_keeps_scratch_content() {
        let manager = WorkspaceSessionManager::test_fixture("n1-run-scratch-restart");
        let (_, token, _, _rx, _) = manager
            .start_run(ProviderRunKind::WorkItemPlanBatch, None)
            .await
            .expect("start run");
        let scratch = scratch_of(&manager);
        std::fs::write(scratch.join("keep.md"), "同 run 暂存").unwrap();

        let (tx, _rx2) = tokio::sync::mpsc::channel(8);
        manager.replace_command_tx_if_token(token, tx).await;

        assert!(scratch.join("keep.md").exists(), "同 run 内重启不得清空 scratch");
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib manager_run_scratch_tests`
Expected: FAIL —— `assertion failed: !scratch.join("old.md").exists()`（登记后未 prepare，前序内容仍在）。

- [ ] **Step 3: 实现接线**

两处登记点各加一段（`start_run_from_attachment` 在构造 `Ok(run)` 之前、`try_start_run_if_idle` 在构造 `Ok(Some(...))` 之前，均在释放状态锁之后）：

```rust
        // REQ-PRB-05：登记后尽力准备 scratch（清空重建 + 0700），失败只 warn。
        let store = crate::product::lifecycle_store::LifecycleStore::new(self.app_paths.clone());
        if let Err(error) = store.prepare_run_scratch(&self.session_id) {
            tracing::warn!(session_id = %self.session_id, error = %error, "run scratch prepare failed");
        }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib manager_run_scratch_tests`
Expected: PASS —— 3 passed。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all
git add src/web/workspace_session/manager/runs.rs
git commit -m "feat(run-boundary): run 登记后准备 per-session scratch"
```

回滚：叶提交（只新增调用，不改既有签名），可单独回滚；回滚后 Task 4 的 API 仍被其自身测试引用，整树可编译。

---

### Task 6: finish_run 条件删除与 supersede 竞态

契约：REQ-PRB-06（唯一收口点条件删除、旧 token 迟到收口不误删、NotFound 幂等、失败只 warn、等待人工状态不额外清理）。

**Files:**
- Modify: `src/web/workspace_session/manager/runs.rs:157-196`（`finish_run`：在既有状态锁块内记录 `no_active_run`，锁释放后清理）
- Test: `src/web/workspace_session/manager/runs.rs:701` 之后（并入 `manager_run_scratch_tests`）

**Interfaces:**
- Consumes: `LifecycleStore::cleanup_run_scratch`（Task 4）；`ManagerState.active_run`（`runs.rs:166-175` 的 take/restore 之后）；`finish_run` 在 `drop(engine)` 之后被调用（`provider_run.rs:1652-1653`）
- Produces: 无新符号

- [ ] **Step 1: 写失败测试**

```rust
    #[tokio::test]
    async fn finish_run_removes_scratch_when_no_active_run() {
        let manager = WorkspaceSessionManager::test_fixture("n1-run-scratch-finish");
        let (_, token, _, _rx, _) = manager.start_run(ProviderRunKind::WorkItemPlanBatch, None).await.unwrap();
        let scratch = scratch_of(&manager);
        std::fs::write(scratch.join("draft.md"), "x").unwrap();

        manager.finish_run(token).await;

        assert!(!scratch.exists(), "唯一活动 run 收口后删除 scratch");
    }

    #[tokio::test]
    async fn late_finish_run_of_superseded_token_keeps_new_scratch() {
        // Review Focus #3：旧 token 的迟到收口不得删掉新 run 的 scratch。
        let manager = WorkspaceSessionManager::test_fixture("n1-run-scratch-supersede-race");
        let (_, stale_token, _, _rx, _) = manager.start_run(ProviderRunKind::WorkItemPlanBatch, None).await.unwrap();
        let _ = manager.start_run(ProviderRunKind::WorkItemPlanBatch, None).await.unwrap();
        let scratch = scratch_of(&manager);
        std::fs::write(scratch.join("keep.md"), "新 run 暂存").unwrap();

        manager.finish_run(stale_token).await;

        assert!(scratch.join("keep.md").exists(), "旧 token 迟到收口必须跳过清理");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[tokio::test]
    async fn finish_run_cleanup_failure_does_not_block_completion() {
        let manager = WorkspaceSessionManager::test_fixture("n1-run-scratch-guard");
        let (_, token, _, _rx, _) = manager.start_run(ProviderRunKind::WorkItemPlanBatch, None).await.unwrap();
        let scratch = scratch_of(&manager);
        let external = manager.app_paths.root().join("external-target");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join("keep.md"), "keep").unwrap();
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(scratch.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&external, &scratch).unwrap();

        manager.finish_run(token).await;

        assert!(manager.active_run().await.is_none(), "收口照常完成");
        assert!(external.join("keep.md").exists(), "护栏拒绝删除符号链接目标");
        let _ = std::fs::remove_file(&scratch);
        let _ = std::fs::remove_dir_all(&external);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib manager_run_scratch_tests`
Expected: FAIL —— `assertion failed: !scratch.exists()`（`finish_run` 未清理）。

- [ ] **Step 3: 实现**

在 `finish_run` 的状态锁块前声明 `let mut no_active_run = false;`，在既有 `if let Some(run) = state.active_run.take() { … } else 分支之后`、锁块内末尾取值 `no_active_run = state.active_run.is_none();`；锁块之后、`self.maybe_recycle().await` 之前插入：

```rust
        // REQ-PRB-06：唯一收口点。仅在此时已无活动 run（旧 token 的迟到收口
        // 会看到新 run 在位 → no_active_run=false）才删除；发生在 drop(engine)
        // 之后（provider_run.rs:1652 先 drop 再 finish_run），失败只 warn。
        if no_active_run {
            let store = crate::product::lifecycle_store::LifecycleStore::new(self.app_paths.clone());
            if let Err(error) = store.cleanup_run_scratch(&self.session_id) {
                tracing::warn!(session_id = %self.session_id, error = %error, "run scratch cleanup failed");
            }
        }
        self.maybe_recycle().await;
```

- [ ] **Step 4: 运行测试确认通过**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib manager_run_scratch_tests`
Expected: PASS —— 6 passed（含 supersede 竞态与护栏用例）。再跑既有收口回归 `flock /tmp/aria-cargo.lock cargo test --locked --lib termination_tests`，Expected: PASS。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all
git add src/web/workspace_session/manager/runs.rs
git commit -m "feat(run-boundary): run 收口时按无活动 run 条件删除 scratch"
```

回滚：叶提交，可单独回滚；回滚后 `finish_run` 恢复原行为，其余任务不受影响。

---

### Task 7: 服务启动 sweep 接线（bind 之后）

契约：REQ-PRB-07（启动并完成端口绑定及其他启动副作用的前置条件后执行一次；失败只打日志、不阻塞启动）。

**Files:**
- Modify: `src/web/app.rs:768-773`（`reconcile_claimed_coding_runs_once` 之后、`refresh_provider_health_for_startup` 之前；`product_paths` 已在 `serve_web` 作用域内，`:727`）
- Test: `src/web/app.rs`（`mod tests`（`:819`）内新增顺序断言用例）

**Interfaces:**
- Consumes: `LifecycleStore::sweep_orphan_run_scratch`、`RUN_SCRATCH_ORPHAN_TTL`（Task 4）；`serve_web` 内既有 `product_paths: ProductAppPaths`
- Produces: 无新符号

- [ ] **Step 1: 写失败测试**

在 `src/web/app.rs` 的 `mod tests` 内：

```rust
    /// REQ-PRB-07：启动 sweep 必须在 bind 之后、与既有 reconcile 同侧，
    /// 且失败不得阻塞启动（只打日志）。
    #[test]
    fn orphan_run_scratch_sweep_runs_after_bind() {
        let source = include_str!("app.rs");
        let bind = source.find("TcpListener::bind(addr)").expect("bind call");
        let sweep = source
            .find("sweep_orphan_run_scratch(crate::product::lifecycle_store::run_scratch::RUN_SCRATCH_ORPHAN_TTL)")
            .expect("orphan run scratch sweep call");
        let serve = source.find("axum::serve(listener, app)").expect("serve call");
        assert!(bind < sweep, "sweep 必须在 bind 之后（B2/F3：bind 失败=零副作用）");
        assert!(sweep < serve);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib orphan_run_scratch_sweep_runs_after_bind`
Expected: FAIL —— `expect("orphan run scratch sweep call")` panic（调用尚不存在）。

- [ ] **Step 3: 实现**

`src/web/app.rs` 在既有 reconcile 块之后追加：

```rust
    // REQ-PRB-07：一次性孤儿 scratch 回收（不增常驻任务）。失败只
    // 打日志、不阻塞启动；单项失败由 sweep 内部处理。
    if let Err(error) = crate::product::lifecycle_store::LifecycleStore::new(product_paths.clone())
        .sweep_orphan_run_scratch(
            crate::product::lifecycle_store::run_scratch::RUN_SCRATCH_ORPHAN_TTL,
        )
    {
        eprintln!("[aria-startup] orphan run scratch sweep failed: {error}");
    }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib orphan_run_scratch_sweep_runs_after_bind`
Expected: PASS —— 1 passed。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all
git add src/web/app.rs
git commit -m "feat(run-boundary): 服务启动 bind 之后回收孤儿 scratch"
```

回滚：叶提交，可单独回滚（sweep 缺席不使任何功能失效，残留空目录无害）。

---

### Task 8: 边界块 scratch 条目（存在则告知，否则省略）

契约：REQ-PRB-02（暂存条目 + 体积预算）、REQ-PRB-04（绝对路径显式告知 + 运行结束会删除 + 目录不存在则省略）。

**Files:**
- Modify: `src/product/workspace_engine/prompts/baseline_teaching.rs:38-90`（`append_author_run_boundary` 内解析 scratch；新增 `fn author_scratch_dir(&self) -> Option<PathBuf>`）
- Test: `src/product/workspace_engine/prompts/baseline_teaching.rs:523` 之后（并入 `author_run_boundary_tests`）

**Interfaces:**
- Consumes: `LifecycleStore::run_scratch_dir`（Task 4，纯计算）、`prepare_run_scratch`（Task 4，供测试造目录）
- Produces: `fn author_scratch_dir(&self) -> Option<std::path::PathBuf>`（仅当 `symlink_metadata` 判定为真实目录时 `Some`）

- [ ] **Step 1: 写失败测试**

```rust
    #[test]
    fn run_boundary_block_announces_existing_scratch_dir() {
        let fixture = baseline_engine("main", None, WorkspaceType::Story);
        let session_id = fixture.engine.session.session_id.clone();
        let store = fixture
            .engine
            .lifecycle_store
            .clone()
            .expect("persistent fixture carry store");
        store.prepare_run_scratch(&session_id).expect("prepare scratch");
        let scratch = store.run_scratch_dir(&session_id);

        let input = fixture
            .engine
            .build_streaming_input("生成", AuthorPromptMode::FullConversation)
            .expect("author prompt");
        assert!(input.prompt.contains(&scratch.display().to_string()), "{}", input.prompt);
        assert!(input.prompt.contains("运行结束系统会删除它"), "{}", input.prompt);
        assert!(input.prompt.len() <= super::RUN_BOUNDARY_BUDGET_BYTES);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn run_boundary_block_omits_scratch_when_dir_absent() {
        let fixture = baseline_engine("main", None, WorkspaceType::Story);
        let session_id = fixture.engine.session.session_id.clone();
        let store = fixture.engine.lifecycle_store.clone().expect("store");
        let _ = std::fs::remove_dir_all(store.run_scratch_dir(&session_id));

        let input = fixture
            .engine
            .build_streaming_input("生成", AuthorPromptMode::FullConversation)
            .expect("author prompt");
        assert!(!input.prompt.contains("run-scratch/"), "目录不存在必须省略条目：{}", input.prompt);
    }

    #[test]
    fn run_boundary_block_omits_scratch_when_dir_is_symlink() {
        let fixture = baseline_engine("main", None, WorkspaceType::Story);
        let session_id = fixture.engine.session.session_id.clone();
        let store = fixture.engine.lifecycle_store.clone().expect("store");
        let scratch = store.run_scratch_dir(&session_id);
        let external = fixture._aria_root.path().join("external");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::create_dir_all(scratch.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&external, &scratch).unwrap();

        let input = fixture
            .engine
            .build_streaming_input("生成", AuthorPromptMode::FullConversation)
            .expect("author prompt");
        assert!(!input.prompt.contains("run-scratch/"), "符号链接不告知：{}", input.prompt);
        let _ = std::fs::remove_file(&scratch);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib run_boundary_block_announces_existing_scratch_dir`
Expected: FAIL —— `assertion failed: input.prompt.contains(...)`（当前恒传 `None`）。

- [ ] **Step 3: 实现**

```rust
    /// REQ-PRB-04：仅当 scratch 是真实目录时告知其绝对路径（不跟随符号链接）。
    fn author_scratch_dir(&self) -> Option<std::path::PathBuf> {
        let store = self.lifecycle_store.as_ref()?;
        let dir = store.run_scratch_dir(&self.session.session_id);
        match std::fs::symlink_metadata(&dir) {
            Ok(metadata) if metadata.file_type().is_dir() => Some(dir),
            _ => None,
        }
    }

    pub(crate) fn append_author_run_boundary(&self, prompt: &mut String) {
        if self.lifecycle_store.is_none() {
            return;
        }
        let scratch = self.author_scratch_dir();
        prompt.push_str(&run_boundary_block(
            &self.session.session_id,
            &self.session.workspace_type,
            scratch.as_deref(),
        ));
    }
```

可见性说明：Task 3 已采用 (A) 方案放宽为 `pub(crate)`（`provider_drive::artifact_retry` 跨模块消费同一注入函数，需要 crate 级可见），实现时照此。

- [ ] **Step 4: 运行测试确认通过**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib author_run_boundary`
Expected: PASS —— 8 passed（含预算断言与两态省略）。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all
git add src/product/workspace_engine/prompts/baseline_teaching.rs
git commit -m "feat(run-boundary): 边界块按 scratch 存在性告知暂存目录"
```

回滚：叶提交（仅改 `append_author_run_boundary` 的参数来源），可单独回滚回「恒 `None`」，整树仍编译。

---

### Task 9: 纯函数扫描器 + 四稳定码 + 表驱动测试

契约：REQ-PRB-09（四类判定、手写切词、无新依赖、历史目录可配置）。

**Files:**
- Create: `src/product/workspace_engine/run_boundary.rs`
- Modify: `src/product/workspace_engine/mod.rs:1-10`（模块声明区加 `mod run_boundary;`，与既有 `mod` 并列；不改文件其余部分）
- Test: `src/product/workspace_engine/run_boundary.rs`（内联 `#[cfg(test)] mod boundary_scan_tests`）

**Interfaces:**
- Consumes: `ProviderExecutionEvent` / `ProviderExecutionEventKind`（`src/cross_cutting/streaming_provider/mod.rs:1112-1143`）、`TimelineNodeType`（`src/web/workspace_ws_types/timeline.rs:10`）
- Produces（`run_boundary.rs` 全部 `pub(crate)`）：
  - `const MAX_SCAN_BYTES: usize = 4096;`
  - `const MAX_RUN_BOUNDARY_ALERTS_PER_NODE: usize = 8;`
  - `const TMP_DRAFT_FIRST_LEVEL_TOKENS: &[&str] = &["plan", "draft", "prior", "refined", "work-item", "spec", "design", "story"];`（`aria-*` 前缀**不**在本集合——它归 `run_boundary_aria_state_read`；契约口径 2026-10-27 统一：类别②=「Aria 状态树」= 状态根之下 **或** 临时目录下第一级名 `aria-*` 的状态/备份目录之下）
  - `const CODE_PROVIDER_HISTORY_READ: &str`、`CODE_ARIA_STATE_READ`、`CODE_TMP_DRAFT_TOUCH`、`CODE_WRITE_OUTSIDE_SCRATCH`（值逐字见 Global Constraints）
  - `struct RunBoundary { state_root: PathBuf, scratch: Option<PathBuf>, tmp_roots: Vec<PathBuf>, history_dirs: Vec<PathBuf> }`
  - `enum RunBoundaryCode { ProviderHistoryRead, AriaStateRead, TmpDraftTouch, WriteOutsideScratch }` + `fn as_str(&self) -> &'static str`
  - `enum BoundaryAccess { Read, Write }`
  - `struct BoundaryFinding { code: RunBoundaryCode, path: String, access: BoundaryAccess }`（`Clone + PartialEq + Eq`）
  - `fn scan_event(boundary: &RunBoundary, event: &ProviderExecutionEvent) -> Vec<BoundaryFinding>`
  - `fn default_history_dirs() -> Vec<PathBuf>`
  - `fn is_author_side_node(node_type: &TimelineNodeType) -> bool`
  - `fn path_hash8(path: &str) -> String`
  - 内部辅助（同一文件，`fn` 私有）：`fn truncate_to_bytes(text: &str, limit: usize) -> String`、`struct Token { text: String, access: Option<BoundaryAccess> }`、`fn tokenize(text: &str) -> Vec<Token>`、`fn absolute_path(token: &str) -> Option<String>`、`fn classify(boundary: &RunBoundary, path: &str, access: BoundaryAccess) -> Vec<RunBoundaryCode>`

- [ ] **Step 1: 写失败测试（表驱动，含事故向量、合法访问、可配置表、有界性）**

```rust
#[cfg(test)]
mod boundary_scan_tests {
    use super::*;
    use crate::cross_cutting::streaming_provider::{
        ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
    };
    use crate::web::workspace_ws_types::TimelineNodeType;
    use std::path::PathBuf;

    fn command_event(command: &str) -> ProviderExecutionEvent {
        ProviderExecutionEvent {
            event_id: "call_1".to_string(),
            kind: ProviderExecutionEventKind::Command,
            status: ProviderExecutionEventStatus::Started,
            title: "bash".to_string(),
            detail: Some(command.to_string()),
            command: Some(command.to_string()),
            cwd: None,
            output: None,
            exit_code: None,
        }
    }

    fn boundary(state_root: &str, scratch: Option<&str>, history: &[&str]) -> RunBoundary {
        RunBoundary {
            state_root: PathBuf::from(state_root),
            scratch: scratch.map(PathBuf::from),
            tmp_roots: vec![PathBuf::from("/tmp"), PathBuf::from("/var/tmp")],
            history_dirs: history.iter().map(PathBuf::from).collect(),
        }
    }

    fn codes(findings: &[BoundaryFinding]) -> Vec<&'static str> {
        findings.iter().map(|finding| finding.code.as_str()).collect()
    }

    #[test]
    fn boundary_scan_matches_incident_vectors() {
        let state = boundary("/home/u/ws/.aria", Some("/home/u/ws/.aria/run-scratch/workspace_session_auto_510c9278"), &["/home/u/.codex/sessions"]);
        let cases: &[(&str, &str)] = &[
            ("find /tmp -iname '*work*item*plan*'", CODE_TMP_DRAFT_TOUCH),
            ("jq -r . /home/u/.codex/sessions/2026/10/27/rollout-1.jsonl", CODE_PROVIDER_HISTORY_READ),
            ("ls /tmp/prior-plan-2026-10-26.md", CODE_TMP_DRAFT_TOUCH),
            ("cat /home/u/ws/.aria/projects/project_0001/issue.json", CODE_ARIA_STATE_READ),
            ("echo x > /tmp/work-item-plan.md", CODE_WRITE_OUTSIDE_SCRATCH),
        ];
        for (command, expected) in cases {
            let findings = scan_event(&state, &command_event(command));
            assert!(codes(&findings).contains(expected), "command={command} findings={:?}", codes(&findings));
        }
        // 写到 scratch 之外：access 必须是 write
        let write = scan_event(&state, &command_event("echo x > /tmp/work-item-plan.md"));
        assert!(write.iter().any(|finding| finding.code == RunBoundaryCode::WriteOutsideScratch
            && finding.access == BoundaryAccess::Write
            && finding.path == "/tmp/work-item-plan.md"));
    }

    #[test]
    fn boundary_scan_state_read_uses_configured_state_root() {
        // 契约《事故读取向量全部命中》：/tmp/aria-restore 归 aria_state_read。
        // 口径（2026-10-27 统一）：类别②=「Aria 状态树」= 状态根之下 **或**
        // 系统临时目录下第一级名 `aria-*` 的状态/备份目录之下 → 与 state_root
        // 是否位于 /tmp 之下无关，生产口径同样归 aria_state_read。
        let incident = boundary("/tmp", Some("/tmp/run-scratch/workspace_session_auto_510c9278"), &["/home/u/.codex/sessions"]);
        let findings = scan_event(&incident, &command_event("ls /tmp/aria-restore/entity-deletions/plan"));
        assert!(codes(&findings).contains(CODE_ARIA_STATE_READ), "{:?}", codes(&findings));

        // 生产口径（state_root 在家目录）下同一路径仍归 aria_state_read（备份树）。
        let production = boundary("/home/u/ws/.aria", Some("/home/u/ws/.aria/run-scratch/sid"), &[]);
        let findings = scan_event(&production, &command_event("ls /tmp/aria-restore/entity-deletions/plan"));
        assert!(codes(&findings).contains(CODE_ARIA_STATE_READ), "{:?}", codes(&findings));
        // 通用草稿名仍是 tmp_draft_touch（不含 aria-*）。
        let findings = scan_event(&production, &command_event("cat /tmp/work-item-plan-draft.md"));
        assert!(codes(&findings).contains(CODE_TMP_DRAFT_TOUCH), "{:?}", codes(&findings));
        assert!(!codes(&findings).contains(CODE_ARIA_STATE_READ), "{:?}", codes(&findings));
    }

    #[test]
    fn boundary_scan_ignores_legitimate_access() {
        let b = boundary(
            "/home/u/ws/.aria",
            Some("/home/u/ws/.aria/run-scratch/workspace_session_auto_1"),
            &["/home/u/.codex/sessions"],
        );
        for command in [
            "cat src/product/main.rs",
            "git show refs/heads/main:src/lib.rs",
            "cat /home/u/.claude/skills/review/SKILL.md",
            "echo note > /home/u/ws/.aria/run-scratch/workspace_session_auto_1/note.md",
            "cat /home/u/ws/.aria/run-scratch/workspace_session_auto_1/note.md",
        ] {
            let findings = scan_event(&b, &command_event(command));
            assert!(findings.is_empty(), "command={command} findings={:?}", codes(&findings));
        }
    }

    #[test]
    fn boundary_scan_does_not_flag_repo_under_tmp() {
        let b = boundary("/home/u/ws/.aria", Some("/home/u/ws/.aria/run-scratch/sid"), &[]);
        let findings = scan_event(&b, &command_event("cat /tmp/test-center-java/src/Main.java"));
        assert!(findings.is_empty(), "{:?}", codes(&findings));
    }

    #[test]
    fn boundary_scan_uses_configurable_history_dirs() {
        let with_table = boundary("/home/u/ws/.aria", None, &["/home/u/.acme/hist"]);
        let findings = scan_event(&with_table, &command_event("cat /home/u/.acme/hist/s.json"));
        assert!(codes(&findings).contains(CODE_PROVIDER_HISTORY_READ), "{:?}", codes(&findings));

        let empty_table = boundary("/home/u/ws/.aria", None, &[]);
        let findings = scan_event(&empty_table, &command_event("cat /home/u/.acme/hist/s.json"));
        assert!(findings.is_empty(), "表为空即不报该码：{:?}", codes(&findings));
    }

    #[test]
    fn resume_reading_provider_history_warns() {
        // Review Focus #1：resume 会话再次读取自身历史时检测层必须告警。
        let b = boundary("/home/u/ws/.aria", None, &["/home/u/.codex/sessions"]);
        let findings = scan_event(&b, &command_event("cat /home/u/.codex/sessions/2026/10/27/rollout-1.jsonl | jq -r ."));
        assert!(codes(&findings).contains(CODE_PROVIDER_HISTORY_READ), "{:?}", codes(&findings));
    }

    #[test]
    fn boundary_scan_is_bounded_and_never_panics() {
        let b = boundary("/home/u/ws/.aria", None, &[]);
        let padded = format!("{}cat /tmp/work-item-plan.md", " ".repeat(MAX_SCAN_BYTES));
        assert!(scan_event(&b, &command_event(&padded)).is_empty(), "超出前 4 KiB 的部分不扫");

        let empty = ProviderExecutionEvent {
            event_id: "call_2".to_string(),
            kind: ProviderExecutionEventKind::Command,
            status: ProviderExecutionEventStatus::Started,
            title: String::new(),
            detail: None,
            command: None,
            cwd: None,
            output: None,
            exit_code: None,
        };
        assert!(scan_event(&b, &empty).is_empty());
    }

    #[test]
    fn author_side_node_whitelist_excludes_reviewer() {
        for node_type in [
            TimelineNodeType::AuthorRun,
            TimelineNodeType::Revision,
            TimelineNodeType::WorkItemDraftRun,
            TimelineNodeType::WorkItemBatchRun,
            TimelineNodeType::WorkItemPlanOutlineRun,
            TimelineNodeType::PlanRepairAuthoringRevision,
        ] {
            assert!(is_author_side_node(&node_type), "{node_type:?}");
        }
        for node_type in [
            TimelineNodeType::ReviewerRun,
            TimelineNodeType::ReviewDecision,
            TimelineNodeType::WorkItemDraftReview,
            TimelineNodeType::WorkItemBatchReview,
            TimelineNodeType::WorkItemPlanOutlineReview,
            TimelineNodeType::PlanRepairPlanReview,
            TimelineNodeType::Completed,
        ] {
            assert!(!is_author_side_node(&node_type), "{node_type:?}");
        }
    }

    #[test]
    fn path_hash8_is_deterministic_and_eight_chars() {
        assert_eq!(path_hash8("/tmp/a"), path_hash8("/tmp/a"));
        assert_eq!(path_hash8("/tmp/a").len(), 8);
        assert_ne!(path_hash8("/tmp/a"), path_hash8("/tmp/b"));
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib boundary_scan`
Expected: FAIL —— `error[E0583]: file not found for module run_boundary`（新文件与模块声明尚不存在）。

- [ ] **Step 3: 实现扫描器**

`run_boundary.rs`：字符串切词手写实现（`split_whitespace` + 引号内合并的简化切分；不引 `regex`）；切词后把 token 归为「读路径候选」与「写目标」两类：出现 `>`、`>>`、`tee`、`cp`、`mv`、`sed -i`、`apply_patch`、`write_text`、`open(` 且后随 `'w'`、`cat <<` 时，其后第一个绝对路径 token 记为写目标；其余绝对路径 token 记为读路径候选。判定顺序与实现要点：

```rust
pub(crate) fn scan_event(boundary: &RunBoundary, event: &ProviderExecutionEvent) -> Vec<BoundaryFinding> {
    if event.kind != ProviderExecutionEventKind::Command {
        return Vec::new();
    }
    let mut text = String::new();
    if let Some(command) = event.command.as_deref() {
        text.push_str(command);
    }
    if let Some(detail) = event.detail.as_deref() {
        text.push('\n');
        text.push_str(detail);
    }
    // 按 UTF-8 字节截断到前 4 KiB（逐字符累加字节数，不切在字符中间）。
    let text: String = truncate_to_bytes(&text, MAX_SCAN_BYTES);
    let tokens = tokenize(&text);
    let mut findings: Vec<BoundaryFinding> = Vec::new();
    for token in tokens {
        let Some(access) = token.access else { continue };
        let Some(path) = absolute_path(&token.text) else { continue };
        for code in classify(boundary, &path, access) {
            let finding = BoundaryFinding { code, path: path.clone(), access };
            if !findings.contains(&finding) {
                findings.push(finding);
            }
        }
    }
    findings
}
```

`classify` 规则（按 REQ-PRB-09 逐字）：① `path` 等于/位于/构成任一 `history_dirs` 的祖先 → `ProviderHistoryRead`；② `path` 位于 `state_root` 之下且不在 `scratch` 之下，**或**位于某 `tmp_root` 之下且其**第一级名称**（小写）以 `aria-` 开头 → `AriaStateRead`（Aria 状态树口径）；③ `path` 位于某 `tmp_root` 之下且其**第一级名称**（小写）命中 `TMP_DRAFT_FIRST_LEVEL_TOKENS`（不含 `aria-` 前缀，它已归②）→ `TmpDraftTouch`；④ `access == Write` 且 `path` 是绝对路径且不在 `scratch` 之下 → `WriteOutsideScratch`。`is_author_side_node` 用显式白名单 `matches!`；`path_hash8` 用 `std::collections::hash_map::DefaultHasher` 取 `format!("{:016x}", hash)` 的前 8 字符。

- [ ] **Step 4: 运行测试确认通过**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib boundary_scan`
Expected: PASS —— 10 passed。

- [ ] **Step 5: 逐 provider 核实历史目录表（实现期核实，不得写成事实）**

`default_history_dirs()` 只放**仓库内已有证据**的条目，其余留待真机核实；核实方法逐条执行并把结论写进函数注释（`// 已核实` / `// 待核实`）：

- codex：`~/.codex/sessions` —— 证据 `src/cross_cutting/local_usage.rs:134`（已核实）。
- kimi：`~/.kimi-code/sessions` —— 仓库内用途读取路径见 `src/cross_cutting/local_usage.rs:141-146`，真机仍需确认（`ls ~/.kimi-code/sessions`）。
- pi：`~/.pi/agent/sessions` —— 仓库内仅测试夹具路径 `src/cross_cutting/pi_provider/usage_tests.rs:13`，真机仍需确认（`ls ~/.pi/agent/sessions`）。
- claude：`~/.claude/projects` —— 设计 §11.1 标注为推断，真机核实前**不得**写入表（`ls ~/.claude/projects`）。
- provider 的 skills 目录（如 `~/.claude/skills`）MUST NOT 进表，避免误报（由 `boundary_scan_ignores_legitimate_access` 钉住）。

实现：

```rust
pub(crate) fn default_history_dirs() -> Vec<PathBuf> {
    // 表为空即不报该码；条目与证据见设计 §11 实现期待核实清单。
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    vec![
        home.join(".codex/sessions"),      // 已核实：src/cross_cutting/local_usage.rs:134
        home.join(".kimi-code/sessions"),  // 待真机核实
        home.join(".pi/agent/sessions"),   // 待真机核实
    ]
}
```

补一条断言：`flock /tmp/aria-cargo.lock cargo test --locked --lib default_history_dirs`（新增 `default_history_dirs_includes_codex_sessions`：`HOME=/home/u` 下 `default_history_dirs()` 含 `/home/u/.codex/sessions`），Expected: PASS。

- [ ] **Step 6: 提交**

```bash
cargo fmt --all
git add src/product/workspace_engine/run_boundary.rs src/product/workspace_engine/mod.rs
git commit -m "feat(run-boundary): 新增作者侧 tool-call 越界扫描器与四稳定码"
```

回滚：本提交是第③步基座（Task 10 消费 `scan_event`）；单独回滚需先回滚 Task 10（逆序），回滚后整树仍编译。

---

### Task 10: `emit_execution_event` 漏斗接线（形状、去重、上界、只告警）

契约：REQ-PRB-10（事件形状/范围/上界/不递归/不 panic）、REQ-PRB-11（只告警不阻断）。

**Files:**
- Modify: `src/product/workspace_engine/provider_drive/work_item_plan.rs:464-496`（`emit_execution_event`）
- Modify: `src/product/workspace_engine/run_boundary.rs`（新增 `run_boundary_alert_event`）
- Create: `src/product/workspace_engine/tests/run_boundary_detection.rs`
- Modify: `src/product/workspace_engine/tests.rs`（末尾追加 `mod run_boundary_detection;`）

**Interfaces:**
- Consumes: `scan_event` / `is_author_side_node` / `path_hash8` / 四稳定码（Task 9）；`active_node_type()`（`lifecycle.rs:1449`）；`update_node_detail`（`session_state/timeline.rs:328`）；`execution_event_json` / `upsert_execution_event_json`（`mappings.rs:80`、`:90`）；`EngineEvent::ExecutionEvent`
- Produces:
  - `pub(crate) fn run_boundary_alert_event(finding: &BoundaryFinding, source_event_id: &str) -> ProviderExecutionEvent`
  - `impl WorkspaceEngine { fn run_boundary_context(&self) -> Option<RunBoundary>; async fn append_run_boundary_alerts(&mut self, node_id: &str, source_event_id: &str, findings: &[BoundaryFinding], agent: Option<ProviderName>); }`

- [ ] **Step 1: 写失败测试**

新文件 `src/product/workspace_engine/tests/run_boundary_detection.rs`：

```rust
//! REQ-PRB-10/11：告警事件形状、去重、上界、范围，以及「只告警不阻断」。

use super::*;

const CODE_TMP: &str = "run_boundary_tmp_draft_touch";

fn command_event(event_id: &str, command: &str) -> ProviderExecutionEvent {
    ProviderExecutionEvent {
        event_id: event_id.to_string(),
        kind: ProviderExecutionEventKind::Command,
        status: ProviderExecutionEventStatus::Started,
        title: "bash".to_string(),
        detail: Some(command.to_string()),
        command: Some(command.to_string()),
        cwd: None,
        output: None,
        exit_code: None,
    }
}

fn alert_events(store: &LifecycleStore, session_id: &str, node_id: &str) -> Vec<serde_json::Value> {
    store
        .load_node_detail(session_id, node_id)
        .expect("node detail")
        .execution_events
        .into_iter()
        .filter(|event| {
            event
                .get("event_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| id.starts_with("run_boundary_"))
        })
        .collect()
}

#[tokio::test]
async fn command_event_keeps_original_and_appends_alert() {
    let (_tmp, store, mut engine) = persistent_test_engine();
    let session_id = engine.session().session_id.clone();
    let node_id = create_author_run_node(&mut engine).await;

    engine
        .emit_execution_event(
            command_event("call_1", "find /tmp -iname '*work*item*plan*'"),
            Some(node_id.clone()),
            Some(ProviderName::ClaudeCode),
        )
        .await;

    let detail = store.load_node_detail(&session_id, &node_id).expect("detail");
    let original = detail
        .execution_events
        .iter()
        .find(|event| event.get("event_id").and_then(|value| value.as_str()) == Some("call_1"))
        .expect("原事件仍在");
    assert_eq!(
        original.get("command").and_then(|value| value.as_str()),
        Some("find /tmp -iname '*work*item*plan*'"),
        "原事件不得被修改"
    );

    let alerts = alert_events(&store, &session_id, &node_id);
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0]["kind"], "provider");
    assert_eq!(alerts[0]["status"], "completed");
    assert_eq!(alerts[0]["title"], format!("运行边界告警: {CODE_TMP}"));
    let alert_id = alerts[0]["event_id"].as_str().unwrap();
    assert!(alert_id.starts_with("run_boundary_tmp_draft_touch_"));
    assert_eq!(alert_id.len(), "run_boundary_tmp_draft_touch_".len() + 8);
    let alert_detail = alerts[0]["detail"].as_str().unwrap();
    assert!(alert_detail.contains("path="), "{alert_detail}");
    assert!(alert_detail.contains("access=read"), "{alert_detail}");
    assert!(alert_detail.contains("source_event=call_1"), "{alert_detail}");
    assert!(alerts[0]["command"].is_null() && alerts[0]["cwd"].is_null() && alerts[0]["output"].is_null());
}

#[tokio::test]
async fn alerts_are_deduped_and_capped_at_eight() {
    let (_tmp, store, mut engine) = persistent_test_engine();
    let session_id = engine.session().session_id.clone();
    let node_id = create_author_run_node(&mut engine).await;

    for _ in 0..3 {
        engine
            .emit_execution_event(
                command_event("call_same", "cat /tmp/work-item-plan.md"),
                Some(node_id.clone()),
                Some(ProviderName::ClaudeCode),
            )
            .await;
    }
    assert_eq!(alert_events(&store, &session_id, &node_id).len(), 1, "同路径同规则按 id 去重");

    for index in 0..10 {
        engine
            .emit_execution_event(
                command_event(&format!("call_{index}"), &format!("cat /tmp/work-item-plan-{index}.md")),
                Some(node_id.clone()),
                Some(ProviderName::ClaudeCode),
            )
            .await;
    }
    assert!(alert_events(&store, &session_id, &node_id).len() <= 8, "每 node 至多 8 条");
}

#[tokio::test]
async fn reviewer_node_is_not_scanned() {
    let (_tmp, store, mut engine) = persistent_test_engine();
    let session_id = engine.session().session_id.clone();
    let node_id = create_reviewer_run_node(&mut engine).await;

    engine
        .emit_execution_event(
            command_event("call_review", "cat /tmp/work-item-plan.md"),
            Some(node_id.clone()),
            Some(ProviderName::Codex),
        )
        .await;

    assert!(alert_events(&store, &session_id, &node_id).is_empty(), "reviewer 节点不扫");
}

#[tokio::test]
async fn alert_event_does_not_rescan() {
    let (_tmp, store, mut engine) = persistent_test_engine();
    let session_id = engine.session().session_id.clone();
    let node_id = create_author_run_node(&mut engine).await;

    engine
        .emit_execution_event(
            ProviderExecutionEvent {
                event_id: format!("run_boundary_tmp_draft_touch_{}", "0".repeat(8)),
                kind: ProviderExecutionEventKind::Provider,
                status: ProviderExecutionEventStatus::Completed,
                title: "运行边界告警: run_boundary_tmp_draft_touch".to_string(),
                detail: Some("path=/tmp/work-item-plan.md access=read source_event=call_x".to_string()),
                command: None,
                cwd: None,
                output: None,
                exit_code: None,
            },
            Some(node_id.clone()),
            Some(ProviderName::ClaudeCode),
        )
        .await;

    assert_eq!(alert_events(&store, &session_id, &node_id).len(), 1, "告警事件不再触发扫描");
}

#[tokio::test]
async fn detection_failure_does_not_affect_run() {
    // REQ-PRB-11：无 store 的内存态 engine，检测路径整段跳过，事件照常广播。
    let (tmp, checkpoint_store) = setup();
    let (event_tx, mut event_rx) = mpsc::channel(64);
    let mut engine = WorkspaceEngine::new(checkpoint_store, event_tx, make_session("run-boundary-headless"));
    let node_id = create_author_run_node(&mut engine).await;

    engine
        .emit_execution_event(
            command_event("call_headless", "cat /tmp/work-item-plan.md"),
            Some(node_id),
            Some(ProviderName::ClaudeCode),
        )
        .await;

    let events = drain_engine_events(&mut event_rx);
    assert!(
        events.iter().any(|event| matches!(
            event,
            EngineEvent::ExecutionEvent { event, .. } if event.event_id == "call_headless"
        )),
        "原事件必须照常广播"
    );
    drop(tmp);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib run_boundary_detection`
Expected: FAIL —— `assert_eq!(alerts.len(), 1)` 失败（`assertion left: 0, right: 1`），首个用例即红。

- [ ] **Step 3: 实现告警事件与漏斗接线**

`run_boundary.rs` 追加：

```rust
pub(crate) fn run_boundary_alert_event(
    finding: &BoundaryFinding,
    source_event_id: &str,
) -> ProviderExecutionEvent {
    let code = finding.code.as_str();
    ProviderExecutionEvent {
        event_id: format!("run_boundary_{code}_{}", path_hash8(&finding.path)),
        kind: ProviderExecutionEventKind::Provider,
        status: ProviderExecutionEventStatus::Completed,
        title: format!("运行边界告警: {code}"),
        detail: Some(format!(
            "path={} access={} source_event={source_event_id}",
            finding.path,
            match finding.access {
                BoundaryAccess::Read => "read",
                BoundaryAccess::Write => "write",
            }
        )),
        command: None,
        cwd: None,
        output: None,
        exit_code: None,
    }
}
```

`work_item_plan.rs` 的 `emit_execution_event`：在方法开头算出扫描请求（不改变既有持久化顺序），广播之后追加告警：

```rust
    pub(crate) async fn emit_execution_event(
        &mut self,
        event: ProviderExecutionEvent,
        node_id: Option<String>,
        agent: Option<ProviderName>,
    ) {
        // 原事件先持久化、再追加告警（REQ-PRB-10）；扫描只对作者侧 Command 事件。
        let scan = self.run_boundary_scan_request(&event, node_id.as_deref());
        let source_event_id = event.event_id.clone();
        // …既有持久化 + usage 诊断 + 广播（逐字不变）…
        if let Some((node_id, findings)) = scan {
            self.append_run_boundary_alerts(&node_id, &source_event_id, &findings, agent).await;
        }
    }
```

同文件（`impl WorkspaceEngine` 内）新增：

```rust
    fn run_boundary_scan_request(
        &self,
        event: &ProviderExecutionEvent,
        node_id: Option<&str>,
    ) -> Option<(String, Vec<crate::product::workspace_engine::run_boundary::BoundaryFinding>)> {
        use crate::product::workspace_engine::run_boundary;
        let node_id = node_id?;
        if event.kind != ProviderExecutionEventKind::Command {
            return None;
        }
        let node_type = self.active_node_type()?;
        if !run_boundary::is_author_side_node(&node_type) {
            return None;
        }
        let context = self.run_boundary_context()?;
        let findings = run_boundary::scan_event(&context, event);
        if findings.is_empty() {
            return None;
        }
        Some((node_id.to_string(), findings))
    }

    fn run_boundary_context(&self) -> Option<crate::product::workspace_engine::run_boundary::RunBoundary> {
        use crate::product::workspace_engine::run_boundary;
        let store = self.lifecycle_store.as_ref()?;
        let dir = store.run_scratch_dir(&self.session.session_id);
        let scratch = std::fs::symlink_metadata(&dir)
            .ok()
            .filter(|metadata| metadata.file_type().is_dir())
            .map(|_| dir);
        Some(run_boundary::RunBoundary {
            state_root: store.app_paths().root().to_path_buf(),
            scratch,
            tmp_roots: vec![
                std::path::PathBuf::from("/tmp"),
                std::path::PathBuf::from("/var/tmp"),
            ],
            history_dirs: run_boundary::default_history_dirs(),
        })
    }

    async fn append_run_boundary_alerts(
        &mut self,
        node_id: &str,
        source_event_id: &str,
        findings: &[crate::product::workspace_engine::run_boundary::BoundaryFinding],
        agent: Option<ProviderName>,
    ) {
        use crate::product::workspace_engine::run_boundary::{
            run_boundary_alert_event, MAX_RUN_BOUNDARY_ALERTS_PER_NODE,
        };
        let mut inserted: Vec<ProviderExecutionEvent> = Vec::new();
        let _ = self
            .update_node_detail(node_id, |detail| {
                let mut existing = detail
                    .execution_events
                    .iter()
                    .filter(|event| {
                        event
                            .get("event_id")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|id| id.starts_with("run_boundary_"))
                    })
                    .count();
                for finding in findings {
                    if existing >= MAX_RUN_BOUNDARY_ALERTS_PER_NODE {
                        break;
                    }
                    let alert = run_boundary_alert_event(finding, source_event_id);
                    let json = crate::product::workspace_engine::mappings::execution_event_json(&alert);
                    let duplicated = detail
                        .execution_events
                        .iter()
                        .any(|event| event.get("event_id") == json.get("event_id"));
                    crate::product::workspace_engine::mappings::upsert_execution_event_json(
                        &mut detail.execution_events,
                        json,
                    );
                    if !duplicated {
                        existing += 1;
                        inserted.push(alert);
                    }
                }
            })
            .await;
        for alert in inserted {
            let _ = self
                .event_tx
                .send(EngineEvent::ExecutionEvent {
                    event: alert,
                    node_id: Some(node_id.to_string()),
                    agent: agent.clone(),
                })
                .await;
        }
    }
```

> `execution_event_json` 若在 `mappings` 内为 `pub(crate)` 则按上例调用；若为模块私有，改为在本文件内用 `serde_json::json!` 直接构造同一形状（字段见 `mappings.rs:80-93`）。告警不改变任何 gate/审批状态——只做 `update_node_detail` + 广播。

- [ ] **Step 4: 运行测试确认通过**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib run_boundary_detection`
Expected: PASS —— 5 passed。再跑既有漏斗回归 `flock /tmp/aria-cargo.lock cargo test --locked --lib usage_persist_failure_leaves_durable_diagnostic_and_keeps_emitting`，Expected: PASS（原事件持久化与广播顺序未变）。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all
git add src/product/workspace_engine/provider_drive/work_item_plan.rs src/product/workspace_engine/run_boundary.rs src/product/workspace_engine/tests/run_boundary_detection.rs src/product/workspace_engine/tests.rs
git commit -m "feat(run-boundary): 执行事件漏斗追加四类越界告警（只告警不阻断）"
```

回滚：叶提交（第③步最后一个改动提交），可单独回滚；回滚后 Task 9 的扫描器仍被自身测试使用，整树可编译。

---

### Task 11: 贯穿验证与零改动核对

契约：REQ-PRB-11（只告警不阻断的最终证据）、REQ-PRB-12（provider 侧与既有契约零改动、既有测试全绿）。

**Files:** 无（只跑命令与核对；不产生提交）

**Interfaces:**
- Consumes: Task 1–10 全部产出
- Produces: 验收证据清单

- [ ] **Step 1: 记录基线提交（用于零改动核对）**

Run: `git rev-parse HEAD`
Expected: 输出本次第一步实施前的分支 HEAD（记为 `<baseline>`；若 Task 1 已提交，用 `git rev-parse HEAD~<本 change 提交数>` 或实施前记录的值）。

- [ ] **Step 2: 三步定向回归**

Run:
```bash
flock /tmp/aria-cargo.lock cargo test --locked --lib run_boundary_block
flock /tmp/aria-cargo.lock cargo test --locked --lib author_run_boundary
flock /tmp/aria-cargo.lock cargo test --locked --lib run_boundary_prompt
flock /tmp/aria-cargo.lock cargo test --locked --lib manager_run_scratch_tests
flock /tmp/aria-cargo.lock cargo test --locked --lib run_scratch
flock /tmp/aria-cargo.lock cargo test --locked --lib sweep_orphan_run_scratch
flock /tmp/aria-cargo.lock cargo test --locked --lib boundary_scan
flock /tmp/aria-cargo.lock cargo test --locked --lib run_boundary_detection
```
Expected: 每个过滤器 `test result: ok.`，0 failed。

- [ ] **Step 3: 完整 lib 回归（含既有 prompt 预算与内存态 engine 测试）**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --lib`
Expected: `test result: ok.`，0 failed；既有 prompt 预算断言与内存态 engine 测试未经修改即通过（REQ-PRB-12 场景 3）。

- [ ] **Step 4: 行数红线核对**

Run: `flock /tmp/aria-cargo.lock cargo test --locked --test it_core large_file_guard`
Expected: PASS；`GRANDFATHERED` 表未新增条目、红线文件行数未增长（`tests/it_core/large_file_guard.rs`）。

- [ ] **Step 5: provider 侧与既有契约零改动核对**

Run:
```bash
git diff --stat <baseline> -- \
  src/cross_cutting/codex_provider/session.rs \
  src/cross_cutting/claude_code_provider \
  src/cross_cutting/pi_provider \
  src/cross_cutting/kimi_code_provider \
  src/cross_cutting/streaming_provider/mod.rs \
  src/product/logical_codebase/provider_gateway.rs \
  src/web/workspace_ws_handler/run/provider_run.rs \
  src/product/workspace_engine/prompts.rs \
  src/product/workspace_engine/lifecycle.rs \
  src/product/lifecycle_store/plan.rs \
  src/product/lifecycle_store/workspace.rs
```
Expected: 空输出（无任何改动）。若非空，逐项确认是否属本 change 允许的落点；不允许的立即回滚该改动。

- [ ] **Step 6: 静态检查**

Run: `cargo fmt --all -- --check` 然后 `flock /tmp/aria-cargo.lock cargo clippy --locked --lib -- -D warnings`
Expected: fmt 无 diff；clippy 无 warning（`-D warnings` 下退出码 0）。

- [ ] **Step 7: 残余风险确认（写入交付说明，不做代码改动）**

逐条确认并明示：

- [ ] 提示词是软约束，模型可忽略；检测层是唯一把静默污染变成可观测事件的一层（只告警不阻断）。
- [ ] resume 场景禁令无效：已读入原生历史的内容无法撤回，只有再次读取时告警。
- [ ] codex 原生 `FileChange` 即使写 scratch 仍被 T13 判失败（`codex_provider/session.rs:199`），本 change 不开豁免。
- [ ] 事故现场既有 `/tmp` 残留不由本 change 清理。
- [ ] `RUN_SCRATCH_ORPHAN_TTL`（6h）> `PROVIDER_IDLE_WATCHDOG_TIMEOUT`（1800s，`watchdog.rs:22-23`）由 `run_scratch_orphan_ttl_exceeds_provider_idle_watchdog` 钉住；若后者上调须重新核对。
- [ ] 真机验收（codex author 在 `/tmp` 诱饵文件存在时重复生成，核对 prompt 含边界块、node detail 出现稳定码告警、run 结束后 scratch 被清理）需 owner 另行授权 provider 调用——**未完成前不得宣称跨 run 污染已被治理**。

- [ ] **Step 8: 汇总证据（不提交）**

把 Step 2–7 的命令与结果（通过/失败条目）整理进交付说明；发现缺陷时回到所属任务按 TDD 修复并补提交，不在本任务内直接改代码。

---

## 自审结论

按 `writing-plans` 自审五项逐条核对，发现并已修正如下：

1. **Spec 覆盖**：REQ-PRB-01/02/03 由 Task 1–3 + Task 8 覆盖；REQ-PRB-04/05/06/07/08 由 Task 4–8 覆盖；REQ-PRB-09/10/11 由 Task 9–10 + Task 11 覆盖；REQ-PRB-12 由 Task 11 的 Step 3–5 覆盖。发现并已修：初稿把「块内 scratch 条目」并入 Task 4，导致 REQ-PRB-04 的 `目录缺失时退化` 场景无独立可验收点，改为独立 Task 8。
2. **步骤粒度**：删除了两处「实现边界情况处理」式空步骤（prepare 的符号链接退化、sweep 的单目失败），改为在签名与测试里写死（`prepare` 遇 `Skipped` 直接 `Ok(())`；`sweep` 逐项 `continue` 并 warn）。发现并已修：Task 2 最初把 `append_author_baseline_teaching` 的签名也改写（会触碰语义面），改为只在首行插入一次调用、签名不动。
3. **类型一致性**：统一 `RunScratchRemoval { Removed, Absent, Skipped }`（Task 4 定义，Task 6 消费）、`BoundaryFinding { code, path, access }`（Task 9 定义，Task 10 消费）、`RUN_BOUNDARY_HEADING` / `RUN_BOUNDARY_BUDGET_BYTES`（Task 1 定义，Task 2/8 消费）。发现并已修：初稿在 Task 3 的测试文件里 import `RUN_BOUNDARY_HEADING` 常量，但 `prompts::baseline_teaching` 是 `prompts` 的私有子模块（`prompts.rs:9`）不可跨模块引用，且 `prompts.rs` 已到行数红线不能加 `pub(crate) use`——改为在测试文件内复写规范逐字标题常量。
4. **Review Focus**：5 条全部落到具体测试步骤——#1 → Task 3 Step 1 + Task 9 Step 1（`resume_reading_provider_history_warns`）；#2 → Task 4 Step 1（prepare 存活准入）+ Task 4 Step 5（读 spy 断言）；#3 → Task 6 Step 1；#4 → Task 4 Step 5（活 run 保留 + TTL/watchdog 关系断言）；#5 → Task 9 Step 1（可配置表 + 合法访问零告警）。第 6 个候选（scratch 不进 worktree / 不进 `git status`）按 brief 未入 Review Focus，但同样由 Task 4 Step 1 的 `run_scratch_dir_lives_under_state_root_not_repo` 与 `gitignore_covers_aria_state_root` 钉住。
5. **比例**：全文 2119 行（其中围栏代码 1450 行），契约（spec 约 250 行 + design 约 360 行）合计约 610 行。代码量偏大但结构合规：约 1100 行是**失败测试**（brief 与 skill 均要求给出测试名与断言值，且这些断言就是「实施者不能自行发明」的决策），实现体只占约 270 行且仅覆盖签名与测试无法确定的算法（护栏的 canonical 核对、sweep 判定、切词骨架）；无整段照抄实现；两处可省内容已删（见下段）。

第二轮事实核对又修正两处：① `PROVIDER_IDLE_WATCHDOG_TIMEOUT` 的可见路径是 `provider_drive` 的 re-export（`provider_drive.rs:24-27`），不是私有的 `provider_drive::watchdog` 子模块；② `WorkspaceSession` 未实现 `Clone`，内存态用例改为自建临时 store + `WorkspaceSession::from_record`，并据此删掉不再需要的 `setup_headless` 夹具与「借 engine.session」错误说法；同时补上 Review Focus #2 缺的 `prepare_run_scratch_refused_for_purging_session` 用例（原稿只在文字里声称已钉住）。

另记两处「不得写成事实」的落实：Task 9 Step 5 把 claude/pi/kimi 历史目录写成**实现期核实步骤**（表可配置、表空即不报，claude 未核实前不得入表）；Task 4 Step 5 + Task 11 Step 7 把「TTL 6h 与 watchdog 零活动阈值」写成可断言的实现期核实（`RUN_SCRATCH_ORPHAN_TTL > PROVIDER_IDLE_WATCHDOG_TIMEOUT`），不表述为已核实结论。
