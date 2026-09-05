# 2026-09-05 计划文档：F3 角色黑名单实施 `restrict-role-write-tools` v1.0

> v1.0.1（2026-09-05 controller 亲验修订）：Task 0 证据不入 git（.superpowers 为 git-ignored）；测试片段改为真实 API 签名（pi `build_args` 扩第三参/claude `build_args` 扩第二参、`ensure_request_id`、新增 `codex_launch_params` 抽取）；`json_rpc_peer.rs` 路径纠正；行号锚点校正；语义层 provider 参数定为 `&str`。

> v1.0.2（2026-09-05 oracle 过审驱动修订，8 findings 全修）：P1 全量构造点迁移（70 处/29 文件补 `tool_policy`/`audit_sink`/`native_session_id` 的 `None`）；P2 request id 改 codex 作用域化 namespace（pi/kimi 数字 id 零变化）；P3 claude 扩第二参勘误；P4 守卫签名统一 `&AdapterRole`；P5 审计 trait/事件 DTO 移中立模块 `cross_cutting/tool_policy_audit.rs`；P6 version probe 按 adapter 映射（pi 复用既有 probe，策略会话才 fail-closed）；P7 `ProviderSession` 增 `native_session_id`+start 返回前握手时序；P8 单一 role-fixture 改逐格矩阵 fixture（Handoff 无 builder，由守卫测试合成 input 覆盖）。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to execute this plan. Steps use checkbox (`- [ ]`) syntax.

## Goal

在不扩大已获批契约范围的前提下，为作者/评审角色的 built-in 文件写工具建立统一的角色黑名单、provider 物理翻译、Codex 审批分类、双向 fail-closed 守卫、冻结 digest/resume 比对与 durable 启动/审批审计；Coder、kimi 及既有非策略路径保持契约规定的行为。

## Architecture

`ProviderToolPolicy` 是宿主层唯一语义策略源，由 engine builder 写入 `StreamingProviderInput`，经 pi/claude/codex adapter 翻译为 canonical 物理启动参数；adapter 在 spawn 前执行角色双向守卫。Codex adapter 在本地按 wire 形态完成审批分类与即时协议应答，策略会话写 `durable_tool_policy_audit`，Coder/非策略/kimi 走既有 `execution_event_audit`。LifecycleStore 以 `tool-policy-run-audit/` JSONL 保存 provider_start、approval_decision、protocol_warning、session_terminated，并在 resume 前比较策略 digest、CLI version、adapter dialect。

## Tech Stack

Rust 2024、Cargo stable（`rust-toolchain.toml`）、现有 streaming provider adapter、workspace/coding engine builder、Codex app-server JSON-RPC、LifecycleStore JSONL、sha256 canonical digest、现有 ApprovalBridge 与 provider event/command 链路。

## Spec

本计划只展开以下已批准契约，不修改其范围、边界或验收：

- `openspec/changes/restrict-role-write-tools/proposal.md`
- `openspec/changes/restrict-role-write-tools/design.md`（D1-D8）
- `openspec/changes/restrict-role-write-tools/tasks.md`（0.x-4.x）
- `openspec/changes/restrict-role-write-tools/specs/session-policy-envelope/spec.md`（REQ-ENV-01/02/06/08 MODIFIED、REQ-ENV-09 ADDED）
- 实施锚点：`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/f3-contract-review-r8-final.md`

## Global Constraints

以下约束对全部工作包生效，关键值按契约逐字冻结：

1. **黑名单而非 allowlist。** `ProviderToolPolicy` 只表达语义意图；本期唯一合法意图为 `DenyFileWriteBuiltins`。保护范围是 built-in 文件写工具，不是全工具 allowlist。黑名单之外的读取、bash/terminal 只读命令、自发现 MCP/extension、`ask_user` 不因本 change 被禁用；bash/terminal 与自发现 MCP 是显式信任逃逸面，真只读不在本 change。
2. **角色矩阵。** Orchestrator、WorkItemSplitter、Reviewer 必须带 `DenyFileWriteBuiltins`；Executor/Coder 与聚合初始化 provider turns 必须不带策略；Handoff 无真实入口且携带策略时拒绝。SC author/revision、WorkItemPlan author（普通/fresh/resume/with-session/serial/batch）、SC/workspace reviewer、review repair、coding CodeReviewer/InternalReviewer/group review 均按矩阵注入。
3. **pi 物理片段冻结。** 策略会话 argv 必须注入 `--exclude-tools edit,write`；与 `--session-id` 组合及空 session id 边界必须测试。不得把其它 provider 片段写入 pi。
4. **claude 物理片段冻结。** 策略会话 argv 必须注入 `--disallowedTools Edit,Write,NotebookEdit`，名单大小写与成员冻结；与 `--resume` 组合必须测试。不得改写为其它名单。
5. **codex 三联动冻结。** 策略直连 adapter 的 `thread/start` 与 `thread/resume` 同时使用 `sandbox=read-only`、`approvalPolicy=on-request` 与审批分类规则；Coder 维持既有 `danger-full-access` 和 permission mode 映射。gateway-mediated codex 的 REQ-ENV-05 路由阻断不解除、不改写。
6. **codex 审批分类冻结。** `item/commandExecution/requestApproval` 是 commandExecution；`item/fileChange/requestApproval` 是 fileChange；`mcpServer/elicitation/request` 只有 `_meta.codex_approval_kind="mcp_tool_call"` 才是 MCP。策略会话 commandExecution/fileChange=拒绝并审计；所有会话 MCP=accept 并审计；Coder commandExecution/fileChange 继续 ApprovalBridge 既有上抛链；未知 elicitation 返回 JSON-RPC error `-32601` 并带 `data`，未知 item 返回 `{"decision":"decline"}`，同一会话连续 `>=3` 次未知形态终止并记录 `reason_code=unknown_approval_storm`。未知形态不得静默不应答，`reason` 不得作为分类依据。
7. **request id 命名空间冻结。** server→client 入站 id 保持原生数字；Aria 出站 request id 使用 typed namespace 字符串 `aria-<seq>`。必须有 server `0` 与 client `aria-0` 共存且不冲突的 fixture。
8. **digest 规范冻结。** 使用 sha256；规范输入是 `"tp-v1"`、provider 名、canonical token 序列、`"ap-v1"`，各段以 `\x1f` 连接：`"tp-v1" + \x1f + provider + \x1f + canonical token sequence + \x1f + "ap-v1"`。argv flag/value 按出现顺序原样、大小写保留并以 `\x1f` 连接；Codex 审批规则变化必须升级 `ap-v1`。物理片段或审批规则变化必须改变 digest。
9. **version/dialect 与 resume。** version 是每次策略会话启动时 adapter 执行 CLI `--version` 的结果（进程内缓存、有界超时；不可得则策略会话启动 fail-closed）；dialect 是 adapter 常量，如 `codex-app-server-rpc`、`claude-stream-json`、`pi-rpc`。resume 在 spawn 前于当前 workspace 会话审计分区检索最近 `provider_start`，精确比较 provider_session_id 对应的 digest/version/dialect 三元组；记录缺失或任一不一致时拒绝 resume、标记 superseded、新建会话。
10. **审计通道严格分离。** 策略角色 pi/claude/codex 使用 LifecycleStore 的 `durable_tool_policy_audit`（分区 `tool-policy-run-audit/`）；Coder、非策略路径与 kimi 使用既有 `execution_event_audit`。`GatewayRunAudit`、driver choice 兜底、permission 映射与 ApprovalBridge 既有语义不因本 change 取代或混用。
11. **durable 事件与时序冻结。** 仅允许四类 canonical 事件：`provider_start`、`approval_decision`、`protocol_warning`、`session_terminated`。每个文件 key 为 `(workspace_session_id, role_run_seq)`；文件内 JSONL 行 seq 单调递增；`provider_start` 恰为首行且每文件仅一条，后三类可跟随。provider 启动握手成功后、`start` 返回前追加 `provider_start`；追加失败由 adapter 终止子进程并返回错误，engine 沿既有 provider task kill 链终止并将 run 判失败。后续事件追加失败同样沿既有 kill 链。读取坏行跳过并产生读取结果告警，该告警不写回 durable 分区。
12. **kimi 零改动。** 不向 kimi 注入 tool-policy argv，不接入 `tool-policy-run-audit/`；只为既有 Orchestrator/WorkItemSplitter/Reviewer/Executor client services 行为增加回归断言，不收紧也不放宽。
13. **REQ-ENV-01/02 例外边界。** 既有流式 legacy 直连 author/revision/review 与 Coder 直连可以保留，但必须由 builder 构造 input、接受 adapter 双向守卫及适用审计；同步 `AdapterInput` 直连例外不启用，逻辑代码库仍 fail-closed；不引入裸 input fallback，不解除 gateway Codex 阻断。
14. **REQ-ENV-06/08 自发现边界。** provider 原生发现的项目/用户配置（`.mcp.json`、`.kimi-code/mcp.json`、`.codex/config.toml` 等）是用户裁决的受信任通道，不受 Aria bundle 管控；Aria 主动注入的 settings/MCP bundle 仍按既有审计、脱敏和 digest 规则。MCP 可用性不作正向保证。
15. **命令纪律。** 所有本地命令在当前 worktree 根目录、宿主机 Rust 环境执行；`cargo test` **禁止 `-j` 参数**。定向单测必须用 `cargo test --locked --lib <filter>`；approval_bridge 使用 `cargo test-approval-bridge`。全量门禁严格为：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo check --locked`、`cargo test --locked`。每个 Task 收尾必须运行对应 targeted 测试与 `openspec validate restrict-role-write-tools --strict`。
16. **实施停止条件。** Task 0 的三项 CLI 实测任一与契约不符，立即停止本 change 实施并提交证据，不得擅自改名单、沙箱、审批分类或扩大范围。
17. **证据与 fixture 约定。** `.superpowers/` 为 git-ignored（SDD workspace 磁盘持久），其下证据文件只落盘不 commit，完成后在 `progress.md` 台账登记路径。测试片段中未在生产代码中存在的 helper（`entry_input`、`test_tool_policy_audit_sink`、`*_event`、`probe_fixture`、`provider_start_record` 等）均为随实现新建的测试 fixture，签名以各 Task Interfaces 为准；生产函数（`codex_launch_params`、`ensure_request_id` 扩展、`OutboundIdNamespace`、`validate_tool_policy_for_role` 等）必须先在 Interfaces 声明再使用。


## Task 0 — CLI 实测门禁（OpenSpec task 0.x / REQ-ENV-09）

**验收尺寸：** 三项彼此独立；每项都记录 CLI version、完整命令、原始输出与断言到指定证据文件。任一冻结能力、参数名、大小写、组合行为或空值边界不符，立即停止 change，不进入 Task 1。

### Task 0.1 — Claude denylist 与 resume 复核

- **Files**
  - Create（实测证据）：`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/f3-task0-claude.md`
  - Modify：无源码修改；此门禁仅产生实测证据
  - Test：CLI 帮助、策略启动与 `--resume` 组合
- **Interfaces**
  - Consumes：Claude CLI 的 `--disallowedTools`、`--resume`、`--version`。
  - Produces：证据文档，必须证明精确名单 `Edit,Write,NotebookEdit`、大小写、built-in 写形态覆盖及 resume 组合可用。
- **Steps**
  - [ ] 运行 `claude --version` 与 `claude --help`，把原始输出和 exit status 记录到证据文件；预期帮助中存在 `--disallowedTools` 与 `--resume`，不存在时立即停止。
  - [ ] 以最小非交互 fixture 启动 `claude --disallowedTools Edit,Write,NotebookEdit`，发送一个 Edit/Write/NotebookEdit 请求和一个读取请求；预期三个写工具均被拒绝或不可用，读取仍可用。
  - [ ] 以同一 denylist 组合有效 `--resume <provider-session-id>` 启动 fixture；预期参数解析成功且 denylist 未丢失。
  - [ ] 在证据文件写入版本、命令、原始输出、通过断言与失败停止条件；不得用摘要替代原始输出。证据文件落盘即完成（`.superpowers/` 为 git-ignored，不入 git），并在 `.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/progress.md` 登记证据路径。

### Task 0.2 — Codex 三联动、MCP 与 Coder 对照复核

- **Files**
  - Create（实测证据）：`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/f3-task0-codex.md`
  - Modify：无源码修改；此门禁仅产生实测证据
  - Test：Codex app-server JSON-RPC fixture，覆盖 read-only/on-request、MCP、fileChange、commandExecution、Coder danger-full-access/never
- **Interfaces**
  - Consumes：`thread/start`/`thread/resume` 参数 `sandbox=read-only`、`approvalPolicy=on-request`；Codex 三种 wire 形态及 `_meta.codex_approval_kind`。
  - Produces：原始双向 wire 与 Coder 对照证据；必须证明 fileChange/commandExecution 的批准会越出 read-only，MCP 形态可识别且能完成调用，Coder 对照结果与 D5 一致。
- **Steps**
  - [ ] 运行隔离 CODEX_HOME 下的 app-server fixture，以 `thread/start(cwd, sandbox=read-only, approvalPolicy=on-request)` 和 `thread/resume` 各启动一次；预期两个请求均原样携带两项冻结参数。
  - [ ] 在 read-only 线程发送只读命令，记录完整双向 JSONL；预期命令零审批直接完成。
  - [ ] 在 read-only 线程发送文件写请求，记录 `item/fileChange/requestApproval` 与关联 `item/started`；预期审批形态可判别，批准后确认写入确实落盘，证明不可无脑批准。
  - [ ] 在 read-only 线程发送 shell 写请求，记录 `item/commandExecution/requestApproval`；预期审批形态可判别，批准后确认写入确实落盘。
  - [ ] 在 read-only 线程发送 MCP 请求，确认方法为 `mcpServer/elicitation/request` 且 `_meta.codex_approval_kind="mcp_tool_call"`，记录批准后的 `serverRequest/resolved` 与结果；预期 MCP 调用完成。
  - [ ] 以 Coder 对照参数 `sandbox=danger-full-access`、`approvalPolicy=never` 触发 fileChange/MCP 可能到达的路径并记录实际 wire；预期结果支持“Coder fileChange/commandExecution 维持 bridge，MCP accept”的契约断言，不得推断未观察到的请求。
  - [ ] 在证据文件写入 version、启动参数、所有原始 wire、通过断言与失败停止条件；任一三联动或分类事实不符立即停止。证据文件落盘即完成（git-ignored，不入 git），并在 progress.md 登记证据路径。

### Task 0.3 — Pi exclude-tools、session-id 与空 id 复核

- **Files**
  - Create（实测证据）：`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/f3-task0-pi.md`
  - Modify：无源码修改；此门禁仅产生实测证据
  - Test：Pi help、argv fixture、非空与空 session id
- **Interfaces**
  - Consumes：Pi CLI 的 `--exclude-tools`、`--session-id`、`--version`。
  - Produces：证据文档，必须证明精确片段 `--exclude-tools edit,write` 与 `--session-id` 组合可用，并定义空 id 的 fail-closed/合法边界。
- **Steps**
  - [ ] 运行 `pi --version` 与 `pi --help`，记录完整输出；预期存在 `--exclude-tools` 与 `--session-id`。
  - [ ] 以 `pi --exclude-tools edit,write --session-id aria-task0-pi` 启动最小 fixture；预期 edit/write 不可用，非写工具仍可用。
  - [ ] 以空 session id 运行等价 fixture；预期行为与 adapter 设计的边界断言一致且无静默放宽，异常必须被记录为 fail-closed。
  - [ ] 在证据文件写入版本、命令、原始输出、通过断言与失败停止条件。证据文件落盘即完成（git-ignored，不入 git），并在 progress.md 登记证据路径。

### Task 0 收尾

- [ ] 运行 `openspec validate restrict-role-write-tools --strict`；预期输出通过。
- [ ] 运行 `git status --short`；预期干净工作树（Task 0 证据不入 git，无实现文件变更）。

## Task 1 — 语义策略与宿主注入（OpenSpec task 1.x / REQ-ENV-09）

**验收尺寸：** 完成后可独立证明策略类型、三 provider translator、canonical digest、所有作者/评审 builder 注入和 pi argv enforcement；Task 1 不实现 Codex 审批分类、不实现 durable sink。每个失败测试先锁定一个契约断言，再以最小实现通过。

### Task 1.1 — 定义策略、translator 与 digest

- **Files**
  - Create/Modify：`src/cross_cutting/streaming_provider/mod.rs`（`StreamingProviderInput` 附近，新增 `ProviderToolPolicy`、`ToolPolicyIntent`、canonical 片段与 digest 类型）
  - Modify：全仓 `StreamingProviderInput` 字面量构造点（存量 70 处/29 文件，含 `src/product/repository_store/initializer.rs:111`、`src/product/image_create/prompt_iteration.rs:124`、`src/product/logical_codebase/coordinator_provider_turn.inc.rs:66`、`src/product/logical_codebase/planning_context_resolver.rs:112`、`src/cross_cutting/provider_availability_gate.rs:436` 等；以 `grep -rn 'StreamingProviderInput\s*{' src --include='*.rs'` 生成全量清单，rustc 错误驱动逐一补 `tool_policy: None`）
  - Create/Modify：`src/cross_cutting/pi_provider/mod.rs`、`src/cross_cutting/claude_code_provider/mod.rs`、`src/cross_cutting/codex_provider/session.rs`（translator 与 canonical 参数）
  - Modify：`Cargo.toml`、`Cargo.lock`（仅在现有 sha256 依赖不存在时，以仓库现有依赖方式加入）
  - Test：`src/cross_cutting/streaming_provider/tests.rs` 或对应 provider 单测模块
- **Interfaces**
  - Consumes：`AdapterRole`、provider 名（`&str`，各 adapter 提供自身 CLI 名常量：`"pi"`/`"claude-code"`/`"codex"`）、`Option<ProviderToolPolicy>`。语义层不依赖 gateway 层枚举（`ProviderRefType` 属 `logical_codebase/provider_gateway.rs:133`，不引入 cross-cutting→product 反向依赖）。
  - Produces：
    - `pub enum ToolPolicyIntent { DenyFileWriteBuiltins }`。
    - `pub struct ProviderToolPolicy { pub intent: ToolPolicyIntent }` + `impl ProviderToolPolicy { pub fn deny_file_write_builtins() -> Self }`（后续 Task 统一用此构造器）。
    - `pub struct CanonicalToolPolicy { pub provider: String, pub tokens: Vec<String>, pub approval_policy_version: String, pub digest: String }`。
    - `pub fn canonical_tool_policy(provider: &str, policy: &ProviderToolPolicy) -> Result<CanonicalToolPolicy, ToolPolicyError>`。
    - `pub fn translate_tool_policy(provider: &str, policy: &ProviderToolPolicy) -> Result<Vec<String>, ToolPolicyError>`。
  - `StreamingProviderInput` 增加 `pub tool_policy: Option<ProviderToolPolicy>`；非策略路径必须传 `None`。
- **Steps**
  - [ ] 先写失败单测，定义唯一意图和精确向量：
    ```rust
    #[test]
    fn canonical_tool_policy_uses_tp_v1_provider_tokens_and_ap_v1() {
        let policy = ProviderToolPolicy { intent: ToolPolicyIntent::DenyFileWriteBuiltins };
        let actual = canonical_tool_policy("pi", &policy).unwrap();
        assert_eq!(actual.tokens, vec!["--exclude-tools", "edit,write"]);
        assert_eq!(actual.approval_policy_version, "ap-v1");
        assert_eq!(actual.digest.len(), 64);
        assert!(actual.digest.chars().all(|ch| ch.is_ascii_hexdigit()));
    }
    ```
  - [ ] 运行 `cargo test --locked --lib canonical_tool_policy_uses_tp_v1_provider_tokens_and_ap_v1`；预期失败信息为 `cannot find ... canonical_tool_policy` 或 digest 断言失败。
  - [ ] 以最小实现加入 `ToolPolicyIntent`、`ProviderToolPolicy`、`CanonicalToolPolicy` 和 translator；pi tokens 必须是 `vec!["--exclude-tools", "edit,write"]`，claude tokens 必须是 `vec!["--disallowedTools", "Edit,Write,NotebookEdit"]`，codex tokens 必须包含 `sandbox=read-only` 与 `approvalPolicy=on-request`，并按 `"tp-v1" + \x1f + provider + \x1f + tokens.join(\x1f) + \x1f + "ap-v1"` 计算 sha256。
  - [ ] 运行 `cargo check --locked`：rustc 将列出全部未补字段的 `StreamingProviderInput` 构造点（存量 70 处/29 文件）；非策略构造点逐一补 `tool_policy: None`（策略 builder 的 `Some` 注入在 Task 1.2 完成），直至 `cargo check --locked` 零错；不得用 `#[derive(Default)]` 或 `..Default::default()` 掩盖遗漏，也不得只在 D2 builder 文件内修补。
  - [ ] 以 shell 实算向量替换长度断言（禁止手写假 digest）：`printf 'tp-v1\x1fpi\x1f--exclude-tools\x1fedit,write\x1fap-v1' | sha256sum`，把输出 hex 写入测试的精确断言 `assert_eq!(actual.digest, "<实算值>")` 并在测试注释记录实算命令；再运行 `cargo test --locked --lib canonical_tool_policy_uses_tp_v1_provider_tokens_and_ap_v1`；预期通过且 digest 在同一输入下稳定。
  - [ ] 追加物理片段、大小写、token 顺序和 `ap-v1` 漂移测试：改变任一 token 或审批规则版本时 `assert_ne!(old.digest, new.digest)`。
  - [ ] 运行 `cargo fmt --check`；预期通过。
  - [ ] 提交实现：`git add src/cross_cutting/streaming_provider src/cross_cutting/pi_provider src/cross_cutting/claude_code_provider src/cross_cutting/codex_provider Cargo.toml Cargo.lock && git commit -m "feat(restrict-role-write-tools): add canonical tool policy"`。

### Task 1.2 — 按 D2 全表向 builder 注入

- **Files**
  - Modify：`src/product/workspace_engine/prompts.rs:219-268,288-348`、`src/product/workspace_engine/prompts/revision.rs:18-72`、`src/product/workspace_engine/prompts/review.rs:126-238,241-265,777-948,951-1157`、`src/product/workspace_engine/prompts/review_repair.rs:6-60`
  - Modify：`src/product/coding_workspace_engine/provider_retry.rs:216-233,373-395`、`src/product/coding_workspace_engine/internal_pr_review.rs:301-325`、`src/product/coding_workspace_engine/group_review_orchestrator.rs:929-964`、`src/product/coding_workspace_engine/coordinator_provider_turn.inc.rs:57-77`
  - Test：`src/product/workspace_engine/tests/part_31.rs`、`src/product/workspace_engine/tests/part_32.rs`、对应 coding workspace engine builder 测试
- **Interfaces**
  - Consumes：`ProviderToolPolicy`、`AdapterRole`、各已有 builder 的 provider/session 参数。
  - Produces：每个作者/评审 builder 返回 `StreamingProviderInput { tool_policy: Some(ProviderToolPolicy { intent: DenyFileWriteBuiltins }), .. }`；Executor/Coder 与聚合初始化返回 `tool_policy: None`。
- **Steps**
  - [ ] 先写失败表驱动测试，逐格调用真实构造路径（D2 全表），不虚构单一 role-fixture：在 workspace_engine 测试模块新建分派 fixture `fn entry_input(entry: &str) -> StreamingProviderInput`——每个分支调用一个真实 builder（workspace author/revision/review 用 part_31 式 session fixture；coding 用 provider_retry/internal_pr_review/group_review 构造；聚合初始化用 coordinator_provider_turn.inc.rs:57-77 真实构造）；Handoff 无真实 builder，不参与本矩阵，仅由 Task 3.1 守卫测试以合成 input 覆盖：
    ```rust
    #[test]
    fn builder_factory_applies_role_policy_matrix_per_entry() {
        for (entry, role, denied) in [
            ("sc_author", AdapterRole::Orchestrator, true),
            ("sc_revision", AdapterRole::Orchestrator, true),
            ("wip_author", AdapterRole::WorkItemSplitter, true),
            ("workspace_reviewer", AdapterRole::Reviewer, true),
            ("coding_coder", AdapterRole::Executor, false),
            ("coding_reviewer", AdapterRole::Reviewer, true),
            ("aggregate_turn", AdapterRole::Executor, false),
        ] {
            let input = entry_input(entry);
            assert_eq!(input.role, role, "{entry}");
            assert_eq!(input.tool_policy.is_some(), denied, "{entry}");
        }
    }
    ```
  - [ ] 运行 `cargo test --locked --lib builder_factory_applies_role_policy_matrix_per_entry`；预期失败为作者/评审 builder 返回 `tool_policy: None`（字段已在 Task 1.1 补齐）。
  - [ ] 最小修改真实锚点 builder：`prompts.rs:219-268,288-348`、`prompts/revision.rs:18-72`、`prompts/review.rs:126-238,241-265,777-948,951-1157`、`prompts/review_repair.rs:6-60`、coding `provider_retry.rs:216-233,373-395`、`internal_pr_review.rs:301-325`、`group_review_orchestrator.rs:929-964`、`coordinator_provider_turn.inc.rs:57-77`；作者/评审设置 `Some(ProviderToolPolicy::deny_file_write_builtins())`，Executor/Coder/聚合初始化设置 `None`。
  - [ ] 增加 builder 全集断言，逐一调用上述真实 builder（含 review.rs 各分派分支与 serial/batch），并断言 `role` 与 `tool_policy` 成对一致；对同步 `AdapterInput` 仅保留既有 logical fail-closed，不扩展本 change 的同步例外。
  - [ ] 运行 `cargo test --locked --lib builder_factory_applies_role_policy_matrix_per_entry`、`cargo test --locked --lib workspace_engine`；预期角色矩阵与既有 workspace 单测通过。
  - [ ] 运行 `cargo fmt --check`；预期通过。
  - [ ] 提交实现：`git add src/product/workspace_engine src/product/coding_workspace_engine && git commit -m "feat(restrict-role-write-tools): inject role policy in builders"`。

### Task 1.3 — Pi argv enforcement

- **Files**
  - Modify：`src/cross_cutting/pi_provider/mod.rs:230-245`
  - Test：`src/cross_cutting/pi_provider/tests.rs:275-296` 或同模块 args 测试
- **Interfaces**
  - Consumes：`StreamingProviderInput.tool_policy`、已有 `session_id` 与 `build_args`。
  - Produces：`PiProvider::build_args(&self, resume_session_id: Option<&str>, extension_path: &Path, tool_policy: Option<&ProviderToolPolicy>) -> Vec<String>`（现签名 `mod.rs:230-248` 为两参，扩第三参；既有调用点与测试同步补 `None`）；策略输入 argv 精确包含 `--exclude-tools`, `edit,write`；session id 原有位置与 resume 行为不变。
- **Steps**
  - [ ] 先写失败单测，直接调用现有 `src/cross_cutting/pi_provider/tests.rs:299-312` 的 `streaming_input_for_test` fixture 与生产 `build_args`：
    ```rust
    #[test]
    fn build_args_policy_keeps_session_id_and_excludes_only_file_writes() {
        let cache = tempfile::tempdir().expect("temporary cache");
        let provider = PiProvider::new("pi".into());
        let extension = ensure_ask_extension_in(cache.path()).expect("ask extension");
        let args = provider.build_args(
            Some("aria-17"),
            &extension,
            Some(&ProviderToolPolicy::deny_file_write_builtins()),
        );
        assert!(args.windows(2).any(|w| w == ["--exclude-tools", "edit,write"]));
        assert!(args.windows(2).any(|w| w == ["--session-id", "aria-17"]));
    }
    ```
  - [ ] 运行 `cargo test --locked --lib build_args_policy_keeps_session_id_and_excludes_only_file_writes`；预期先因 `build_args` 无第三参编译失败——加第三参并在既有调用点传 `None` 后，转为 argv 缺 `--exclude-tools` 的断言失败。
  - [ ] 在 `build_args` 仅对 `Some(DenyFileWriteBuiltins)` 追加 `--exclude-tools edit,write`（置于 `--session-id` 逻辑之后，不改变其顺序）；空 session id 复用既有 trim/filter 语义，不降级成无限制 argv。
  - [ ] 运行 `cargo test --locked --lib build_args_rpc_mode_auto_only` 与 `cargo test --locked --lib build_args_resume_includes_session_id`（既有两测试同步补第三参 `None`）；预期既有 args 测试通过，再以新增策略断言验证 `--exclude-tools edit,write`。
  - [ ] 提交实现：`git add src/cross_cutting/pi_provider && git commit -m "feat(restrict-role-write-tools): enforce pi write denylist"`。

### Task 1 收尾

- [ ] 运行 `cargo test --locked --lib canonical_tool_policy`、`cargo test --locked --lib builder_factory_applies_role_policy_matrix_per_entry`、`cargo test --locked --lib pi_policy_args`；预期全部通过。
- [ ] 运行 `openspec validate restrict-role-write-tools --strict`；预期通过。

## Task 2 — Claude/Codex 执行点与审批分类（OpenSpec task 2.x / REQ-ENV-09）

**验收尺寸：** 完成后可独立证明 Claude denylist、Codex start/resume 三联动、typed request id、三类审批与未知形态确定性应答；Task 2 不实现 LifecycleStore sink 或 resume 审计检索。

### Task 2.1 — Claude denylist 与 resume 组合

- **Files**
  - Modify：`src/cross_cutting/claude_code_provider/mod.rs:82-103`
  - Test：`src/cross_cutting/claude_code_provider/tests/args.rs:9-25`
- **Interfaces**
  - Consumes：`StreamingProviderInput.tool_policy`、`provider_session_id`、现有 `--permission-prompt-tool=stdio`。
  - Produces：`ClaudeCodeProvider::build_args(&self, resume_provider_session_id: Option<&str>, tool_policy: Option<&ProviderToolPolicy>) -> Vec<String>`（现签名 `mod.rs:82` 为单参 `build_args(&self, resume_provider_session_id: Option<&str>)`，扩第二参 `tool_policy`；既有调用点与测试同步补 `None`）；策略 input 的 argv 精确增加 `--disallowedTools Edit,Write,NotebookEdit`，fresh/resume 均保留；非策略 input 不增加该片段。
- **Steps**
  - [ ] 先写失败测试：
    ```rust
    #[test]
    fn claude_policy_args_include_frozen_denylist_with_resume() {
        let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));
        let deny = ProviderToolPolicy::deny_file_write_builtins();
        let args = provider.build_args(Some("claude-session-7"), Some(&deny));
        assert!(args.windows(2).any(|w| w == ["--disallowedTools", "Edit,Write,NotebookEdit"]));
        assert!(args.windows(2).any(|w| w == ["--resume", "claude-session-7"]));
        assert_eq!(args.iter().filter(|arg| arg.as_str() == "--disallowedTools").count(), 1);
        let fresh = provider.build_args(None, Some(&deny));
        assert!(fresh.windows(2).any(|w| w == ["--disallowedTools", "Edit,Write,NotebookEdit"]));
        assert!(!fresh.contains(&"--resume".to_string()));
    }
    ```
  - [ ] 运行 `cargo test --locked --lib claude_policy_args_include_frozen_denylist_with_resume`；预期先因 `build_args` 无第二参编译失败——扩参并在既有调用点传 `None` 后，转为 argv 缺 denylist 的断言失败。
  - [ ] 在 Claude `build_args` 对 `Some(DenyFileWriteBuiltins)` 追加冻结片段，保持 stdio permission prompt 与现有 resume 参数；非策略 input（`None`）保持原 argv；既有 `claude_args_*` 测试同步补第二参 `None`。
  - [ ] 运行 `cargo test --locked --lib claude_policy_args_include_frozen_denylist_with_resume`、现有 `claude_args_include_resume_when_provider_session_is_available` 与 `claude_args_always_include_stdio_permission_prompt`（补参后）；预期全部通过。
  - [ ] 提交实现：`git add src/cross_cutting/claude_code_provider && git commit -m "feat(restrict-role-write-tools): enforce claude denylist"`。

### Task 2.2 — Codex thread/start、thread/resume 与 request id

- **Files**
  - Modify：`src/cross_cutting/codex_provider/session.rs:102-150`、`src/cross_cutting/json_rpc_peer.rs:263-280`（注意：在 codex_provider 之外，为共享 JSON-RPC peer）
  - Test：`src/cross_cutting/codex_provider/tests.rs:149-209`、request id 单测
- **Interfaces**
  - Consumes：`StreamingProviderInput.tool_policy`、`ProviderPermissionMode`、native resume id。
  - Produces：
    - `pub(crate) fn codex_launch_params(input: &StreamingProviderInput) -> serde_json::Value`：从 `session.rs:102-150` 既有内联构造抽出的单一来源，`thread/start` 与 `thread/resume` 两路复用；策略 input（`tool_policy.is_some()`）同时给出 `sandbox:"read-only"` 与 `approvalPolicy:"on-request"`，Coder（`None`）保持 `danger-full-access` 与既有 permission mode 映射。
    - `ensure_request_id`（`src/cross_cutting/json_rpc_peer.rs:263`，🔴 共享 `JsonRpcPeer`，pi/kimi/codex 三方在用）增第三参 `namespace: OutboundIdNamespace`（新枚举 `pub(crate) enum OutboundIdNamespace { Numeric, Aria }`）：仅 codex peer 配 `Aria`（出站 `aria-<seq>` 字符串），pi/kimi 默认 `Numeric` 保持数字 id 零变化；已带 id 的 payload 原样透传。
- **Steps**
  - [ ] 先写失败 Codex launch 测试，在 `src/cross_cutting/codex_provider/tests.rs:149-209` 既有 fixture 基础上新增两个变体（策略/Coder）：
    ```rust
    #[test]
    fn codex_policy_start_and_resume_use_read_only_on_request() {
        let policy_input = codex_streaming_input_with_policy(); // 扩展既有 fixture:tool_policy=Some(deny)
        let params = codex_launch_params(&policy_input);
        assert_eq!(params["sandbox"], "read-only");
        assert_eq!(params["approvalPolicy"], "on-request");
        let coder_input = codex_streaming_input_without_policy(); // 既有形态:tool_policy=None
        let coder = codex_launch_params(&coder_input);
        assert_eq!(coder["sandbox"], "danger-full-access");
    }
    ```
  - [ ] 运行 `cargo test --locked --lib codex_policy_start_and_resume_use_read_only_on_request`；预期先因 `codex_launch_params` 不存在编译失败——抽出该函数（start/resume 两路改为复用）后，转为策略参数断言失败。
  - [ ] 在 `codex_launch_params` 内仅由 `tool_policy.is_some()` 分派 `read-only`/`on-request`；`None` 保持 Coder 的 `danger-full-access` 与现有 permission mode 映射；`thread/start`（session.rs:132）与 `thread/resume`（session.rs:107）两路均改用该函数。
  - [ ] 🔴 作用域警示：`ensure_request_id` 属共享 `JsonRpcPeer`（pi/kimi/codex 三方在用），不得全局改 `aria-<seq>`——pi/kimi 出站 id 必须保持数字零变化（Global Constraints 12）。先写失败测试，基于真实分配点 `ensure_request_id`（`src/cross_cutting/json_rpc_peer.rs:263`）新增 namespace 参数：
    ```rust
    #[test]
    fn codex_outbound_ids_use_aria_namespace_default_peers_keep_numeric() {
        let mut codex_out = serde_json::json!({"method":"item/commandExecution/requestApproval"});
        let next = std::sync::atomic::AtomicU64::new(0);
        let assigned = ensure_request_id(&mut codex_out, &next, OutboundIdNamespace::Aria).unwrap();
        assert_eq!(assigned, "aria-0");
        let mut default_out = serde_json::json!({"method":"session/new"});
        let next2 = std::sync::atomic::AtomicU64::new(0);
        assert_eq!(ensure_request_id(&mut default_out, &next2, OutboundIdNamespace::Numeric).unwrap(), "0"); // pi/kimi 路径零变化
        let mut with_id = serde_json::json!({"id":0,"method":"mcpServer/elicitation/request"});
        let next3 = std::sync::atomic::AtomicU64::new(0);
        assert_eq!(ensure_request_id(&mut with_id, &next3, OutboundIdNamespace::Aria).unwrap(), "0"); // 已带 id 原样保留（入站消息不经本函数，由读取分发保持原 id）
    }
    ```
  - [ ] 运行 `cargo test --locked --lib codex_outbound_ids_use_aria_namespace_default_peers_keep_numeric`；预期先因无 `OutboundIdNamespace` 编译失败。
  - [ ] 实现：`ensure_request_id` 增 `namespace` 第三参；`Numeric` 分配数字 id（现状），`Aria` 分配 `format!("aria-{seq}")`；已带 id 的 payload 两分支均原样返回；全部既有调用点补 `Numeric`（行为零变化），仅 codex peer 调用处传 `Aria`；运行定向测试+两个 launch 定向测试+既有 sandbox start/resume 测试，预期通过；pi/kimi 数字 id 回归断言在 Task 4.1 补锁。
  - [ ] 提交实现：`git add src/cross_cutting/codex_provider && git commit -m "feat(restrict-role-write-tools): enforce codex policy launch"`。

### Task 2.3 — Codex 三类审批、未知应答与审计事件载荷

- **Files**
  - Modify：`src/cross_cutting/codex_provider/parse.rs:196-227`、`src/cross_cutting/codex_provider/session.rs:233-245`、`src/cross_cutting/codex_provider/response.rs:8-25`
  - Modify：`src/cross_cutting/streaming_provider/mod.rs`（新增结构化 `CodexApprovalCategory` 与协议结果，不改变既有 Coder bridge API）
  - Test：`src/cross_cutting/codex_provider/tests.rs:271-324` 及新增每类未知 method fixture
- **Interfaces**
  - Consumes：原始 JSON-RPC method、params、`_meta.codex_approval_kind`、策略 presence、session unknown counter。
  - Produces：`pub enum CodexApprovalCategory { McpToolCall, CommandExecution, FileChange, Unknown { method: String } }`；`pub struct CodexApprovalRequest { pub rpc_id: serde_json::Value, pub category: CodexApprovalCategory, pub server_name: Option<String>, pub tool_name: Option<String>, pub request_id: String, pub description: String }`；`pub enum CodexApprovalResponse { Accept, Decline, ElicitationError { code: i32, data: serde_json::Value } }`。
- **Steps**
  - [ ] 先写失败解析测试，调用现有生产入口 `parse_approval_request(&serde_json::Value)`（`src/cross_cutting/codex_provider/parse.rs:196-227`）：
    ```rust
    #[test]
    fn codex_parser_distinguishes_mcp_from_generic_elicitation() {
        let mcp = parse_approval_request(&json!({
            "method":"mcpServer/elicitation/request", "id":0,
            "params":{"serverName":"proj_spike","_meta":{"codex_approval_kind":"mcp_tool_call","tool_params":{"text":"hi"}}}
        })).unwrap();
        assert!(matches!(mcp.category, CodexApprovalCategory::McpToolCall));
        let unknown = parse_approval_request(&json!({
            "method":"mcpServer/elicitation/request", "id":1,
            "params":{"serverName":"proj_spike"}
        }));
        assert!(unknown.is_none() || matches!(unknown.unwrap().category, CodexApprovalCategory::Unknown { .. }));
    }
    ```
  - [ ] 运行 `cargo test --locked --lib codex_parser_distinguishes_mcp_from_generic_elicitation`；预期失败为 `CodexApprovalRequest` 无 category 或 MCP 仅按方法名识别。
  - [ ] 在 parse 层扩展现有 `CodexApprovalRequest { rpc_id, tool_name, description }` 为结构化 category/metadata；按方法名与 `_meta.codex_approval_kind` 精确分类；fileChange 的 diff 通过 item id 关联缓存的 fileChange item，不使用自然语言 `reason`；未知 item/elicitation 保留原 method。
  - [ ] 先写策略会话决策失败测试，使用生产 `CodexApprovalCategory`：
    ```rust
    #[test]
    fn codex_policy_session_declines_exec_and_file_change_but_accepts_mcp() {
        assert_eq!(decide_for_policy(CodexApprovalCategory::CommandExecution), CodexApprovalResponse::Decline);
        assert_eq!(decide_for_policy(CodexApprovalCategory::FileChange), CodexApprovalResponse::Decline);
        assert_eq!(decide_for_policy(CodexApprovalCategory::McpToolCall), CodexApprovalResponse::Accept);
    }
    ```
  - [ ] 运行 `cargo test --locked --lib codex_policy_session_declines_exec_and_file_change_but_accepts_mcp`；预期失败为三类均进入 bridge 或均 accept。
  - [ ] 实现策略会话即时决策：MCP accept 并写 `approval_decision`；commandExecution/fileChange decline 并写 `approval_decision`；Coder 的 commandExecution/fileChange 继续既有 bridge，Coder MCP accept 并走 `execution_event_audit`。
  - [ ] 先写失败未知应答测试，复用 `src/cross_cutting/codex_provider/response.rs:8-25` 的 JSON-RPC response writer，锁定真实错误结构：
    ```rust
    #[test]
    fn codex_unknown_approval_has_protocol_reply_and_terminates_on_third() {
        let first = decide_unknown("mcpServer/elicitation/request", 1);
        assert_eq!(first, CodexApprovalResponse::ElicitationError {
            code: -32601,
            data: serde_json::json!({"codex_approval_kind":"unknown","reason":"unsupported_approval_kind"}),
        });
        assert_eq!(unknown_storm_reason_after(3), Some("unknown_approval_storm"));
    }
    ```
  - [ ] 运行 `cargo test --locked --lib codex_unknown_approval_has_protocol_reply_and_terminates_on_third`；预期失败为静默忽略或未在第三次终止。
  - [ ] 实现未知 elicitation `-32601`+data、未知 item `{"decision":"decline"}`、连续第三次 `session_terminated`，并让每次未知产生 `protocol_warning`；所有响应复用入站 rpc id。
  - [ ] 运行 `cargo test --locked --lib codex_parser_distinguishes_mcp_from_generic_elicitation`、`cargo test --locked --lib codex_policy_session_declines_exec_and_file_change_but_accepts_mcp`、`cargo test --locked --lib codex_unknown_approval_has_protocol_reply_and_terminates_on_third`；预期通过。
  - [ ] 提交实现：`git add src/cross_cutting/codex_provider src/cross_cutting/streaming_provider && git commit -m "feat(restrict-role-write-tools): classify codex approvals"`。

### Task 2 收尾

- [ ] 运行 `cargo test --locked --lib codex_provider`、`cargo test --locked --lib claude_code_provider`、`cargo test --locked --lib pi_provider`；预期既有与新增定向测试通过。
- [ ] 运行 `openspec validate restrict-role-write-tools --strict`；预期通过。

## Task 3 — 双向守卫、durable 审计与 resume（OpenSpec task 3.x / REQ-ENV-09）

**验收尺寸：** 完成后可独立证明三 adapter 的 spawn 前双向守卫、LifecycleStore durable JSONL、启动握手/写失败 kill 链、四类事件 schema、坏行读取语义及 version/dialect/digest resume 比对。Task 3 不改变 gateway 路由、permission 映射、ApprovalBridge 既有语义或 kimi。

### Task 3.1 — 三 adapter 双向 spawn 前守卫

- **Files**
  - Modify：`src/cross_cutting/pi_provider/mod.rs`、`src/cross_cutting/claude_code_provider/mod.rs`、`src/cross_cutting/codex_provider/mod.rs`
  - Test：各 provider adapter tests；`src/protocol/contracts.rs:35-40` 的 `AdapterRole` 全枚举守卫测试
- **Interfaces**
  - Consumes：`StreamingProviderInput.role`、`tool_policy`、`ProviderToolPolicy`。
  - Produces：`pub fn validate_tool_policy_for_role(role: &AdapterRole, policy: Option<&ProviderToolPolicy>) -> Result<(), ToolPolicyGuardError>`（按引用接 `AdapterRole`，无 `Copy` 派生不隐式复制）；Orchestrator/WorkItemSplitter/Reviewer 缺失、空或非法意图在 spawn 前拒绝；Executor/Handoff 携带策略拒绝。
- **Steps**
  - [ ] 先写失败表驱动测试，复用现有 `AdapterRole`（`src/protocol/contracts.rs:33-41`）和各 provider 的真实 `StreamingProviderInput` fixture：
    ```rust
    #[test]
    fn adapter_tool_policy_guard_is_bidirectional_for_every_role() {
        let deny = ProviderToolPolicy::deny_file_write_builtins();
        for role in [AdapterRole::Orchestrator, AdapterRole::WorkItemSplitter, AdapterRole::Reviewer] {
            assert!(validate_tool_policy_for_role(&role, None).is_err());
            assert!(validate_tool_policy_for_role(&role, Some(&deny)).is_ok());
        }
        for role in [AdapterRole::Executor, AdapterRole::Handoff] {
            assert!(validate_tool_policy_for_role(&role, None).is_ok());
            assert!(validate_tool_policy_for_role(&role, Some(&deny)).is_err());
        }
    }
    ```
  - [ ] 运行 `cargo test --locked --lib adapter_tool_policy_guard_is_bidirectional_for_every_role`；预期失败为守卫函数或双向错误分支不存在。
  - [ ] 将同一守卫接入 pi `PiProvider::start`（`src/cross_cutting/pi_provider/mod.rs:251`）、Claude `ClaudeCodeProvider::start`（`src/cross_cutting/claude_code_provider/mod.rs:345`）和 Codex `run_codex_session`（`src/cross_cutting/codex_provider/session.rs:63`）中，且均位于创建子进程之前；非法 `ToolPolicyIntent` 只能走拒绝路径，不得 fallback 到无策略 argv。
  - [ ] 运行该定向测试及三 provider 缺失/非法/误带策略 fixture；预期所有拒绝发生在 spawn 前。
  - [ ] 提交实现：`git add src/cross_cutting/pi_provider src/cross_cutting/claude_code_provider src/cross_cutting/codex_provider src/protocol/contracts.rs && git commit -m "feat(restrict-role-write-tools): add bidirectional launch guards"`。

### Task 3.2 — LifecycleStore tool-policy-run-audit 与四类事件

- **Files**
  - Create：`src/cross_cutting/tool_policy_audit.rs`（中立模块：`ToolPolicyAuditSink` trait+`DurableToolPolicyEvent`+四类事件 DTO 与 schema v1 serde——🔴 不得放 product 层，避免 cross-cutting→product 反向依赖）
  - Create/Modify：`src/product/lifecycle_store/`（新增 `tool_policy_run_audit.rs` 实现 `ToolPolicyAuditSink` trait，分区 `tool-policy-run-audit/` 落盘）
  - Modify：`src/cross_cutting/streaming_provider/mod.rs`（`audit_sink: Option<Arc<dyn ToolPolicyAuditSink>>` 字段，引用中立模块 trait）
  - Modify：`src/product/workspace_engine/`、`src/product/coding_workspace_engine/`（engine 构造 sink、分配并持久化 `role_run_seq`、kill 链）
  - Test：`src/product/lifecycle_store/` tests、`src/product/coding_workspace_engine/tests/provider_start_persistence.rs:516-562`、JSONL sequence fixture
- **Interfaces**
  - Consumes：`workspace_session_id`、engine 分配的 `role_run_seq`、provider/role、canonical digest、最终 argv、sandbox/approval 原文、provider version/dialect、native `provider_session_id`。
  - Produces：
    - `pub trait ToolPolicyAuditSink: Send + Sync { fn append(&self, workspace_session_id: &str, role_run_seq: u64, event: DurableToolPolicyEvent) -> Result<(), ToolPolicyAuditError>; }`（定义于 `src/cross_cutting/tool_policy_audit.rs`；`LifecycleStore` 在 product 层实现该 trait，无跨层反向依赖）。
    - `ProviderSession` 增 `pub native_session_id: Option<String>`（第三字段，现仅 `events`/`commands`，`mod.rs:383-386`；全仓构造点按 Task 1.1 同法 rustc 驱动补 `None`，非策略路径零变化）；策略会话在 `start` 返回前完成有界握手：codex=await `thread/start` 应答取 thread id（策略路径将握手从后台 `run_codex_session` 提前到 start 内，超时→杀子进程+fail-closed）、pi=id 预生成/传入（既有 `--session-id` 语义）、claude=resume 已知/fresh 等首个 init 事件有界超时；握手成功→写 `provider_start`→返回 `ProviderSession { native_session_id: Some(..), .. }`；握手/写失败按 kill 链处理。
    - `pub enum DurableToolPolicyEvent { ProviderStart(ProviderStartAudit), ApprovalDecision(ApprovalDecisionAudit), ProtocolWarning(ProtocolWarningAudit), SessionTerminated(SessionTerminatedAudit) }`。
    - schema_version=1、append-only JSONL，文件 key=`(workspace_session_id, role_run_seq)`，行 seq 单调递增，`provider_start` 首行且唯一。
- **Steps**
  - [ ] 先写失败 schema 单测，使用 `ToolPolicyAuditSink::append` 的真实 trait 形状：
    ```rust
    #[tokio::test]
    async fn tool_policy_audit_writes_provider_start_once_then_canonical_events() {
        let sink = test_tool_policy_audit_sink();
        sink.append("ws-1", 7, provider_start_event("codex")).unwrap();
        sink.append("ws-1", 7, approval_decision_event("fileChange", "aria-0")).unwrap();
        sink.append("ws-1", 7, protocol_warning_event("unknown")).unwrap();
        sink.append("ws-1", 7, session_terminated_event("unknown_approval_storm")).unwrap();
        let lines = sink.read_lines("ws-1", 7).unwrap();
        assert_eq!(lines[0].event_type(), "provider_start");
        assert_eq!(lines.iter().filter(|line| line.event_type() == "provider_start").count(), 1);
        assert!(lines.windows(2).all(|pair| pair[0].seq < pair[1].seq));
    }
    ```
  - [ ] 运行 `cargo test --locked --lib tool_policy_audit_writes_provider_start_once_then_canonical_events`；预期失败为分区不存在、事件类型不完整或 seq 不单调。
  - [ ] 最小实现 LifecycleStore 分区、schema_version=1、四类事件 serde schema 与互斥串行 sink；重复 provider_start、未知事件类型、跨文件复用同一 role_run_seq 均返回错误。
  - [ ] 运行 schema test；预期生成 JSONL 含 `provider_start`、`approval_decision`、`protocol_warning`、`session_terminated` 四类 canonical 事件且字段含 policy digest、request id、reason code 等契约字段。
  - [ ] 先写坏行读取失败测试：
    ```rust
    #[test]
    fn tool_policy_audit_reader_skips_bad_line_and_returns_protocol_warning() {
        let store = temp_lifecycle_store_with_lines(&["{\"seq\":0,\"event_type\":\"provider_start\"}", "not-json"]);
        let result = store.read_lines_with_warnings("ws-1", 7).unwrap();
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.warnings[0].reason_code, "invalid_json_line");
        assert!(!store.contains_event("protocol_warning"));
    }
    ```
  - [ ] 运行 `cargo test --locked --lib tool_policy_audit_reader_skips_bad_line_and_returns_protocol_warning`；预期失败为坏行中止读取或把读取告警写回 durable 分区。
  - [ ] 实现读取端坏行跳过+结果告警；写入失败必须传播错误，不能只记录日志。
  - [ ] 扩展 `ProviderSession`（`src/cross_cutting/streaming_provider/mod.rs:383-386`）增 `native_session_id: Option<String>`：rustc 错误驱动补全全仓构造点 `None`；三 adapter 策略会话按 Interfaces 定义把有界握手提前到 `start` 返回前（codex 等待 thread/start 应答；claude fresh 等首个 init 事件；pi 预生成传入），非策略/Coder 路径保持现有立即返回行为（`None`）；补握手超时→杀子进程 fail-closed fixture。
  - [ ] 在 engine 构造 `ToolPolicyAuditSink` 并将其以 `StreamingProviderInput.audit_sink` 传给三 adapter；`role_run_seq` 按 provider run 分配并随 run 记录持久化；native session id 握手确认后、start 返回前写 provider_start。
  - [ ] 注入 append 失败 fixture；预期 adapter 在返回前终止子进程，engine 沿既有 provider task kill 链终止会话并将 run 判失败；后续审批事件写失败同样触发既有 kill 链。
  - [ ] 运行 `cargo test --locked --lib tool_policy_audit` 与 `cargo test --locked --lib provider_start_persistence`；预期通过。
  - [ ] 提交实现：`git add src/product/lifecycle_store src/cross_cutting/streaming_provider src/product/workspace_engine src/product/coding_workspace_engine && git commit -m "feat(restrict-role-write-tools): persist durable tool policy audit"`。

### Task 3.3 — version probe、resume 冻结三元组比对

- **Files**
  - Modify：`src/cross_cutting/pi_provider/mod.rs`、`src/cross_cutting/claude_code_provider/mod.rs`、`src/cross_cutting/codex_provider/mod.rs`（`--version` 探测与 dialect 常量）
  - Modify：`src/product/workspace_engine/`、`src/product/coding_workspace_engine/`、`src/product/lifecycle_store/`（resume 检索、superseded/new session）
  - Test：各 provider resume tests、version probe tests、digest drift fixtures
- **Interfaces**
  - Consumes：`resume_provider_session_id`、当前 workspace 的 durable provider_start、canonical digest、CLI version、adapter dialect。
  - Produces：统一错误枚举 `pub enum VersionProbeError { Unavailable, Timeout }` 与按 adapter 的探测函数：pi 复用既有 `probe_pi_version_with_timeout`（`pi_provider/mod.rs:172-190`）映射 `PiVersion::Unknown(ProbeFailure::TimedOut)→Timeout`、其余 `Unknown(_)→Unavailable`（🔴 仅策略会话走该 fail-closed 映射；既有 `ensure_pi_version_compatible` 的 unknown→Ok 旧路径与 Coder/非策略会话零变化）；claude/codex 新增 `probe_claude_version`/`probe_codex_version`（有界超时执行 `<cli> --version`，空输出/命令失败→Unavailable，超时→Timeout，fixture 注入三态输出）；adapter dialect 常量；resume 决策 `pub enum ResumeDecision { Resume, RejectSupersedeAndStartNew }` + `pub fn resume_with_audit_record(stored: Option<ProviderStartAudit>, current: &ProviderStartAudit) -> ResumeDecision`。
- **Steps**
  - [ ] 先写失败 version probe 测试；调用各 adapter 现有 probe（Pi 已有 `probe_pi_version_with_timeout`，Claude/Codex 新增同名语义函数），用 fixture 明确三态：
    ```rust
    #[test]
    fn version_probe_is_fail_closed_for_success_empty_and_timeout() {
        assert_eq!(probe_fixture("ok").unwrap(), "provider 1.2.3");
        assert!(matches!(probe_fixture("empty"), Err(VersionProbeError::Unavailable)));
        assert!(matches!(probe_fixture("timeout"), Err(VersionProbeError::Timeout)));
    }
    ```
  - [ ] 运行 `cargo test --locked --lib version_probe_is_fail_closed_for_success_empty_and_timeout`；预期失败为 empty/timeout 被当成可用 version。
  - [ ] 实现 CLI `--version` 有界探测并进程内缓存；成功非空返回精确字符串，空输出与 timeout 均返回错误；策略会话启动遇错误直接 fail-closed，Coder 非策略路径不改变既有启动。
  - [ ] 运行 version probe 定向测试；预期三态分别是成功、Unavailable、Timeout，且不会执行 provider spawn。
  - [ ] 先写失败 resume fixture；以 D7 `provider_start` 审计字段构造记录，调用新增的 `resume_with_audit_record` 决策函数：
    ```rust
    #[test]
    fn resume_rejects_digest_version_or_dialect_drift_and_missing_record() {
        let stored = provider_start_record("sha256:a", "provider 1.2.3", "codex-app-server-rpc");
        assert!(matches!(resume_with_audit_record(Some(stored.clone()), &stored), ResumeDecision::Resume));
        assert!(matches!(resume_with_audit_record(Some(stored.clone()), &stored.with_digest("sha256:b")), ResumeDecision::RejectSupersedeAndStartNew));
        assert!(matches!(resume_with_audit_record(Some(stored.clone()), &stored.with_version("provider 1.2.4")), ResumeDecision::RejectSupersedeAndStartNew));
        assert!(matches!(resume_with_audit_record(Some(stored.clone()), &stored.with_dialect("codex-app-server-rpc-v2")), ResumeDecision::RejectSupersedeAndStartNew));
        assert!(matches!(resume_with_audit_record(None, &stored), ResumeDecision::RejectSupersedeAndStartNew));
    }
    ```
  - [ ] 运行 `cargo test --locked --lib resume_rejects_digest_version_or_dialect_drift_and_missing_record`；预期失败为 drift 或记录缺失仍 resume。
  - [ ] 在 spawn 前按 workspace 分区和 provider_session_id 检索最近 provider_start，精确比较 tool-policy digest、version、dialect；失败时标记 superseded 并新建会话，成功时保留 native resume id。
  - [ ] 运行 digest/ap-v1、version、dialect、missing-record 全部 fixture；预期通过且 `tool_policy_canonical_digest` 不与 gateway aggregate policy digest 混用。
  - [ ] 提交实现：`git add src/cross_cutting/pi_provider src/cross_cutting/claude_code_provider src/cross_cutting/codex_provider src/product/workspace_engine src/product/coding_workspace_engine src/product/lifecycle_store && git commit -m "feat(restrict-role-write-tools): enforce resume policy fingerprint"`。

### Task 3 收尾

- [ ] 运行 `cargo test --locked --lib tool_policy_audit`、`cargo test --locked --lib version_probe`、`cargo test --locked --lib resume_rejects_digest_version_or_dialect_drift_and_missing_record`；预期通过。
- [ ] 运行 `openspec validate restrict-role-write-tools --strict`；预期通过。

## Task 4 — 回归与运行验收（OpenSpec task 4.x / REQ-ENV-09）

**验收尺寸：** 完成后可独立证明零变化边界、正向可用性、MCP 逃逸面正负向、审计通道隔离、真实 CLI denylist+resume 组合和运营门禁；失败停止并记录，不以替代命令宣称通过。

### Task 4.1 — 零变化、正向可用性与 MCP 逃逸面回归

- **Files**
  - Modify：无生产范围扩展；仅在现有测试文件新增回归
  - Test：`src/cross_cutting/approval_bridge/tests.rs:51-373`、`src/cross_cutting/kimi_code_provider/client_services/policy.rs:40-67`、`src/cross_cutting/pi_provider/tests.rs`、`src/cross_cutting/claude_code_provider/tests/permissions.rs`、`src/cross_cutting/codex_provider/tests.rs`、workspace/coding integration tests
- **Interfaces**
  - Consumes：三 provider policy matrix、既有 ApprovalBridge、kimi `ClientServicePolicy`、execution/durable audit sinks。
  - Produces：回归断言：Coder/聚合初始化无黑名单；pi→Auto permission mapping 不变；ApprovalBridge commandExecution 既有链不变；kimi 四角色策略不变；MCP/extension/ask_user 不在 denylist；策略会话 fileChange/unknown 不静默。
- **Steps**
  - [ ] 先写失败零变化测试，复用真实 coding builder 输出的 `AdapterInput`/`StreamingProviderInput`，并通过现有 Codex sandbox 参数解析器断言：
    ```rust
    #[test]
    fn coder_and_aggregate_executor_keep_existing_full_tool_launch() {
        let coder = entry_input("coding_coder");
        assert_eq!(coder.role, AdapterRole::Executor);
        assert_eq!(coder.tool_policy, None);
        // 聚合初始化同 Executor 档：经 coordinator_provider_turn.inc.rs:57-77 真实构造路径断言（同 Task 1.2 builder 全集断言）
        assert_eq!(codex_launch_params(&coder)["sandbox"], "danger-full-access");
    }
    ```
  - [ ] 运行 `cargo test --locked --lib coder_and_aggregate_executor_keep_existing_full_tool_launch`；预期失败为 Coder 被误注入 denylist 或沙箱被改写。
  - [ ] 增加 file-write negative 与非写 positive 测试：assert denylist 含且仅含 `edit`,`write`（Claude 对应冻结三项），assert MCP、extension、`ask_user` 不在排除集合；Codex 策略 command/fileChange decline，MCP accept；未知方法有协议应答。
  - [ ] 增加 pi/kimi 出站 request id 保持数字的回归断言（`OutboundIdNamespace::Numeric` 路径，锁定 Task 2.2 作用域约束）；运行 `cargo test --locked --lib json_rpc_peer` 与 `cargo test --locked --lib pi_provider`，预期通过。
  - [ ] 运行 `cargo test --locked --lib approval_bridge`（或 `cargo test-approval-bridge`）；预期既有 commandExecution/Auto/Supervised/timeout/unmatched/abort 通过且 ApprovalBridge API 语义不变。
  - [ ] 增加 kimi Orchestrator/WorkItemSplitter/Reviewer/Executor 角色表断言；运行 `cargo test --locked --lib kimi`；预期既有 client services 决策完全不变，未创建 tool-policy audit 文件。
  - [ ] 增加策略审计隔离断言：策略角色事件只出现在 `tool-policy-run-audit/`，Coder/非策略/kimi 事件只出现在 `execution_event_audit`；运行对应 workspace/coding tests，预期无跨通道记录。
  - [ ] 提交回归测试：`git add src/cross_cutting src/product/workspace_engine src/product/coding_workspace_engine tests && git commit -m "test(restrict-role-write-tools): lock zero-change and escape-surface regressions"`。

### Task 4.2 — operational checklist 与持续 validate

- **Files**
  - Create（运行证据）：`cadence/reports/workitem-conversational-gate-advance/evidence/convergence-f3/` 下由 controller 生成的 pi/codex/claude 证据
  - Modify：无契约、源码或测试文件；仅生成指定运行证据
  - Test：全量门禁、真实 CLI、部署 4317、3.6 矩阵首跑
- **Interfaces**
  - Consumes：Task 0 证据、构建产物、controller 部署环境、kimi Trust 状态、Codex `.codex/config.toml` MCP 就绪状态。
  - Produces：四条全量门禁结果、PID/exe/md5 三对账、pi×轻三连跑、Codex 审批分类真实 wire、Claude/Codex 3.6 矩阵首跑及停止/回滚记录。
- **Steps**
  - [ ] 运行 `cargo fmt --check`；预期结果 `passed`，否则停止并记录。
  - [ ] 运行 `cargo clippy --all-targets --all-features --locked -- -D warnings`；预期结果 `passed`，不得加入 `-j`。
  - [ ] 运行 `cargo check --locked`；预期结果 `passed`。
  - [ ] 运行 `cargo test --locked`；预期结果 `passed`，不得加入 `-j`，不得替换为不带 `--lib` 的过滤命令。
  - [ ] 部署 4317 后执行 PID/exe/md5 三对账；三项全部相等才记录通过，任一不符立即失败回滚。
  - [ ] 在证据目录执行 pi×轻 3 连跑并保存原始结果；任一写面回归或运行失败立即停止并记录待裁决，不继续矩阵。
  - [ ] 在证据目录执行 Codex 审批分类真实验证（MCP、fileChange、commandExecution、未知形态）并保存双向 wire；预期与 D5 完全一致。
  - [ ] 随 3.6 矩阵执行 Claude/Codex 对应格首跑；前置记录 kimi 目标仓 Trust 状态与 Codex `.codex/config.toml` MCP 就绪状态；缺少用户侧前置不宣称通过。
  - [ ] 每个实现 Task 收尾均运行 `openspec validate restrict-role-write-tools --strict`；Task 4 最终再次运行该命令，预期通过。
  - [ ] 提交运行证据（仅在 controller 授权且证据生成后）：`git add cadence/reports/workitem-conversational-gate-advance/evidence/convergence-f3 && git commit -m "test(restrict-role-write-tools): record F3 operational acceptance"`。

## Self-Review

### Requirement 对账

| Requirement | 覆盖 Task | 对账结论 |
|---|---:|---|
| REQ-ENV-01 两层政策结构及 legacy 直连显式过渡例外 | Task 1.2、Task 3.1、Task 3.2、Task 4.1 | builder 工厂设置策略、adapter 双向守卫、策略角色 durable 审计、Executor/Coder execution_event_audit、同步 AdapterInput 不扩例外均有步骤；gateway 拓扑与 REQ-ENV-05 不改。 |
| REQ-ENV-02 validated launch policy 与裸 input/fallback fail-closed | Task 1.2、Task 3.1 | 所有流式例外入口仅由 builder 构造并由 adapter 守卫保护；同步逻辑仓仍拒绝；缺失策略在 spawn 前拒绝；未新增 fallback。 |
| REQ-ENV-06 配置来源隔离及自发现通道例外 | Task 0.2、Task 2.3、Task 4.1、Task 4.2 | Codex MCP 自发现 wire 只按 `_meta.codex_approval_kind` 分类，MCP 作为用户裁决信任逃逸面；Aria bundle 管控边界与正向 MCP 可用性不被虚构。 |
| REQ-ENV-08 kimi ACP bundle 与自发现例外 | Task 4.1、Task 4.2 | kimi 不改 argv、不接 durable tool-policy 分区；既有角色 client services 与无 bundle/Trust 前置仅做回归与运行前检查；Aria 注入 bundle 的既有 execution_event_audit 范围不扩大。 |
| REQ-ENV-09 非编码角色 built-in 文件写工具拒绝 | Task 0.x、Task 1.x、Task 2.x、Task 3.x、Task 4.x | 覆盖角色矩阵、pi/claude/codex translator、Codex 审批分类、双向守卫、digest/version/dialect resume、四类审计事件、坏行读取、Coder/kimi 零变化和真实 CLI 验收。 |

### R4 七项实施期验证条件对账

| 条件 | 落点 |
|---|---|
| ① version probe fail-closed 三态 | Task 3.3：success、empty、timeout fixture，错误即策略启动拒绝。 |
| ② request id 碰撞 fixture | Task 2.2：server `0` 与 client `aria-0`。 |
| ③ 四类事件 JSON schema + 坏行读取 | Task 3.2：schema v1、四类事件、坏行跳过+读取告警且不回写。 |
| ④ MCP 逃逸面正负向验证 | Task 0.2、Task 2.3、Task 4.1：MCP accept，command/fileChange 按角色分流，未知拒绝。 |
| ⑤ 策略/非策略审计通道隔离 | Global Constraints 10、Task 3.2、Task 4.1。 |
| ⑥ 真实 CLI denylist+resume 组合 | Task 0.1/0.3、Task 2.1、Task 4.2；Claude/Pi 组合实测与矩阵首跑。 |
| ⑦ openspec validate 持续通过 | Task 0、1、2、3、4 每个收尾，最终 Task 4.2 再次执行。 |

### 占位符与类型一致性结论

- 已执行保留字占位符扫描：未命中禁用占位符；所有停止条件、失败条件、证据路径和命令均已写明。
- 类型命名与跨 Task 接口一致：`ProviderToolPolicy`、`ToolPolicyIntent::DenyFileWriteBuiltins`、`StreamingProviderInput.tool_policy`、`ToolPolicyAuditSink`、`DurableToolPolicyEvent`、`CodexApprovalCategory`、`aria-<seq>`、`ResumeDecision` 在首次定义后保持同名；三 provider 共用语义策略而保留 provider-specific wire response。
- 证据锚点只采用契约与终审锚点清单已核实的 file:line；新增代码位置以现有模块/测试文件及已核实行号为实施定位，不声称尚未产生的命令输出。

### 计划完成门禁

- [ ] worker 执行前先读取本计划头部、Global Constraints 与对应 Task 全文。
- [ ] 每个 Task 的失败测试、精确命令、最小实现、通过命令、commit 命令均按 checkbox 顺序执行；任一门禁失败即停止，不扩大范围。
- [ ] 最终只允许提交本计划文档本身；本计划中的源码/证据 commit 命令是未来实施 worker 的执行步骤，不是本 writing-plans 工作树的当前操作。
