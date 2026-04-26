# 设计评审：Aria 一期方案与实施计划研发可落地性 Review

**文档信息**

- **创建日期**：2026-04-26
- **版本**：v4.0
- **评审目标**：确认当前方案与实施计划是否足以让研发人员看懂并清楚落地
- **目标读者**：Aria 一期技术负责人、P1-P4 owner、daemon / REPL / provider / OpenSpec / execution 研发负责人
- **评审基准**：以 `Aria一期MVP精简设计_v1.2` 与 `Aria一期实现总契约_v1.0` 为准；派生方案和实施计划不得改变基准语义

---

## 1. 总体结论

本轮没有发现需要推翻或改写 **MVP 精简设计 v1.2**、**实现总契约 v1.0** 的方向性错误。

当前文档包已经修复上一轮大部分问题：`ExternalArtifactRef` 字段口径、`RuntimeUnit.node_id()` 协议 ID 约束、P2 validator 分层、P4 contract / prompt registry、candidate commit 生成点、`N26` OpenSpec bundle 处理都已补入当前文档。

但按“研发人员拿方案和计划即可落地实现”的标准，仍有 1 个需裁定项和 5 个需要修订的问题。主要风险不是架构方向错误，而是任务顺序、代码落位和节点边界表达仍会让研发自行脑补。

建议裁定：

| 阶段 | 当前判断 | 进入实现前必须处理 |
|------|----------|--------------------|
| P1 | 可启动 | 无阻塞项 |
| P2 | 可启动前需裁定 | 明确 OpenSpec `specs/<scope>/spec.md` 的 scope 派生规则 |
| P3 | 修正后启动 | 修正 Task 1 依赖 `context_builder` 的顺序问题 |
| P4 | 修正后启动 | 明确 `N20` 代码落位、`candidateCommitSha` snapshot 字段、`N22/N23` git 责任边界、`N26` provider 写权限与 daemon 写 OpenSpec 的边界 |

---

## 2. 评审对象

### 2.1 背景与基准

| 文档 | 结论 |
|------|------|
| `cadence/designs/2026-04-22_技术方案_Aria终端REPL与多Agent编排Runtime设计_v1.0.md` | 文档集入口，无新增阻塞问题 |
| `cadence/designs/2026-04-23_技术方案_Aria一期MVP精简设计_v1.2.md` | 一期范围、21 个实现单元、协议节点保留策略清晰 |
| `cadence/designs/2026-04-23_技术方案_Aria一期实现总契约_v1.0.md` | 对象模型、统一执行链、OpenSpec 约束与阶段收口规则可作为实现基准 |

### 2.2 派生方案与计划

| 文档 | 结论 |
|------|------|
| `Aria_IO协作协议与Provider契约_v1.1` | 字段口径已对齐 v1.3，但 `N26` 写权限表达需补 daemon / provider 边界 |
| `Aria一期研发导读与实施拆解_v1.1` | 阅读路径清晰，可保留 |
| `Aria一期评审后实施规格补齐_v1.3` | 代码级规格大体可执行，但 OpenSpec scope、`candidateCommitSha`、`N26` OpenSpec 细则需补 |
| `P1基础骨架与REPL通信_v1.1` | 可执行 |
| `P2产物投影与OpenSpec约束_v1.2` | OpenSpec scope 规则裁定后可执行 |
| `P3Provider驱动与规划节点_v1.2` | 需要修正 Task 1 测试顺序 |
| `P4执行集成与最终收口_v1.1` | 需要修正 N20/N22/N26 的落位与边界表达 |
| `实施计划总览_v1.1` | 总览清晰，需同步新增准入门槛 |

---

## 3. 需裁定项

### C1：OpenSpec `specs/<scope>/spec.md` 的 scope 派生规则未定义

**位置**

- `实现总契约_v1.0` 第 948-955 行：P2 bootstrap 要创建 `specs/<task-scope>/spec.md`。
- `评审后实施规格补齐_v1.3` 第 1189-1193 行：bundle 固定读取 `specs/<scope>/spec.md`。
- `P2计划_v1.2` 第 341-343 行：最小 skeleton 包含 `specs/<task-scope>/spec.md`。

**影响**

研发实现 P2 bootstrap 时无法确定 `<task-scope>` / `<scope>` 的来源。不同实现可能选择 `changeId`、`taskId`、用户标题 slug、固定 `main` 或 OpenSpec 既有目录，导致 fixture、bundle compiler 和后续 import/export 行为不一致。

这不是总契约方向错误，但属于总契约占位未展开的实现裁定。进入 P2 前必须定死。

**建议裁定**

在 `评审后实施规格补齐_v1.3` 第 6.1 章和 P2 Task 5 中补充：

| 项目 | 建议规则 |
|------|----------|
| 默认 scope | 若用户未显式指定，使用 `scope = sanitize(changeId)` 或固定 `scope = "main"`，二选一 |
| 显式 scope | 如允许 `new_task.requested_scope`，必须在 P1 wire payload 和 task runtime state 中定义 |
| 多 spec 目录 | 一期建议不支持多 scope；若目录中已有多个 `specs/*/spec.md`，进入 gate 或 manual intervention |
| fixture | `tests/fixtures/openspec/changes/sample-change/specs/<scope>/spec.md` 必须使用同一规则 |

推荐保守选项：一期默认 `scope = "main"`，并把多 scope 作为二期能力。

---

## 4. 阻塞问题

### P1-1：P3 Task 1 在 `ProviderContextPackage builder` 创建前要求跑 `context_builder`

**位置**

- `P3计划_v1.2` 第 91-98 行：Task 1 只创建 provider adapter / run / router，却把测试目标写成 `tests/context_builder.rs`。
- 同文档第 150-153 行：Task 1 要求 `cargo test --test context_builder` 通过，并说明 fake provider 可被 builder 调用。
- 同文档第 242-249 行：`src/cross_cutting/provider_context_builder.rs` 实际到 Task 3 才创建。

**影响**

研发按 Task 1 执行时，要么无法编译 `context_builder`，要么必须提前实现 Task 3 的 builder，破坏计划的可顺序执行性。对 agentic worker 更明显，因为每个 task 都有独立测试和 commit。

**建议修正**

二选一：

1. 将 Task 1 的测试改为 `tests/provider_adapter.rs` 或 `tests/provider_adapter_baseline.rs`，只验证 `AdapterInput/AdapterOutput`、fake provider 和 `ProviderRunRecord`。
2. 或把 `provider_context_builder.rs` 的最小骨架提前到 Task 1，并在 Task 3 只扩展 registry / prompt 渲染。

推荐第一种，保持 Task 1 只负责 adapter baseline，Task 3 再负责 context builder。

---

### P1-2：P4 没有明确 `N20 ready_for_integration` 的代码落位

**位置**

- `MVP精简设计_v1.2` 第 206-214 行：`M20 integration_prepare_impl` 覆盖 `N20 + N21 + N22`，且 A/B/C 子阶段分别负责 ready、入队、集成准备。
- `P4计划_v1.1` 第 222-229 行：Task 2 标题覆盖 `N16-N20`，但文件只有 `coding.rs`、`testing.rs`、`code_review.rs`、`rework.rs`，没有 `ready_for_integration.rs` 或明确 `integration_prepare.rs` 覆盖 `N20`。
- `P4计划_v1.1` 第 261-269 行：Step 4 要实现 `N20 ready_for_integration` 并生成 `candidateCommitSha`。
- `P4计划_v1.1` 第 283-290 行：Task 3 创建 `integration_prepare.rs`，但范围写成 `N21-N24`，没有声明它覆盖 `N20`。

**影响**

研发会不知道 `N20` 应该放在哪里：

- 放进 `rework.rs` 不符合节点语义。
- 放进 `integration_prepare.rs` 又与 Task 3 的 `N21-N24` 范围描述不一致。
- 新增 `ready_for_integration.rs` 又超出目标文件结构。

这会直接影响 `RuntimeUnit.covered_protocol_nodes()`、snapshot `nodeId`、candidate commit 生成点和 N22 输入。

**建议修正**

推荐按 MVP 的 `M20` 折叠模型修正 P4：

- 将 `src/runtime_units/integration_prepare.rs` 明确定义为 `M20 integration_prepare_impl`，`covered_protocol_nodes()` 返回 `["N20", "N21", "N22"]`。
- P4 Task 2 只实现 `N16-N19`。
- P4 Task 3 改名为“实现 `M20/N20-N22` 与 `N23-N24` 集成链路”。
- 在 Task 3 Step 1 中明确断言：`N20 ready` 生成 `candidateCommitSha`，`N21` 入队，`N22` 记录 `integrationBranch/preMergeSha`。

如果团队更想拆独立文件，则新增 `src/runtime_units/ready_for_integration.rs`，并同步总览、P4 文件清单和完成判定。

---

### P1-3：`candidateCommitSha` 要写 runtime snapshot，但 `nodeSpecificFields` 最小定义没有字段

**位置**

- `评审后实施规格补齐_v1.3` 第 1786-1788 行：`N20` 只有 `readyDecision`、`advisoryProviderRunRef?`，`N22` 只有 `integrationBranch`、`preMergeSha`。
- 同文档第 1852-1857 行：`N20 ready` 前必须创建 candidate commit 并记录 `candidateCommitSha`。
- `P4计划_v1.1` 第 266-268 行：`N20` 必须记录 `candidateCommitSha` 到 worktask runtime state、后续 `N22` 输入和 runtime snapshot。

**影响**

实现者会知道要记录 `candidateCommitSha`，但不知道它在 runtime snapshot 中属于哪个节点的 `nodeSpecificFields`。如果有人把它只放 worktask state，不放 snapshot，恢复和审计链会缺关键字段；如果有人放到 `N22`，又会弱化 “N20 生产、N22 消费” 的边界。

**建议修正**

在 `评审后实施规格补齐_v1.3` 第 11 章补充：

| 节点 | 建议字段 |
|------|----------|
| `N20` | `readyDecision`、`candidateCommitSha?`、`candidateCommitCreatedAt?`、`advisoryProviderRunRef?` |
| `N22` | `integrationBranch`、`preMergeSha`、`candidateCommitSha` |

并要求 P4 测试断言：

- `N20` ready snapshot 包含 `candidateCommitSha`。
- `N22` 只能读取并复制引用该 sha，不创建新的 candidate commit。

---

### P1-4：P4 把 cherry-pick conflict 写入 `N22 preflight`，与 `N23` Git 集成边界冲突

**位置**

- `评审后实施规格补齐_v1.3` 第 1867-1883 行：`N22` 记录 `preMergeSha`；`git cherry-pick --no-commit <candidateCommitSha>` 属于 `N23`，cherry-pick conflict 失败路由也是 `N23` 的失败处理。
- `P4计划_v1.1` 第 310-321 行：`N22 integration_prepare` 的 preflight 检查包含 “cherry-pick conflict 必须 abort 并路由 `N19`”。

**影响**

研发可能把 cherry-pick 放进 `N22` 做“预检”，这样会把 `N23 integration_execute` 的核心 Git 操作前移，破坏 `N22` 准备、`N23` 执行、`N24` 验证的审计边界。

**建议修正**

将 P4 Task 3 改成：

- `N22 integration_prepare` 只做：candidate commit 存在性校验、integration branch 创建或复用、`preMergeSha` 记录、输入引用落盘。
- `N23 integration_execute` 做：checkout integration branch、cherry-pick、integration pre-checks、commit、失败 rollback/abort。
- `cherry-pick conflict 必须 abort 并路由 N19` 移到 Step 4 `integration_execute` 和完成判定中。

---

### P1-5：`N26` 写权限表与 OpenSpec 更新职责容易误读为冲突

**位置**

- `实现总契约_v1.0` 第 959-964 行：`N05/N07/N11/N26` 写入 OpenSpec 文件后需要编译 `OpenSpecConstraintBundle`。
- `IO协作协议_v1.1` 第 252-260 行：Claude Code 节点契约里 `N26` 写权限为“只写 Aria 产物区或 stdout”。
- `P4计划_v1.1` 第 108-111 行：`N25/N26/N27` 只能写 Aria artifact 区或 stdout 候选产物。
- `P4计划_v1.1` 第 412-415 行：`N26` 需要通过 Document Operation 更新 OpenSpec `tasks.md` 或 patch task delta，并触发 bundle stale / recompile。
- `评审后实施规格补齐_v1.3` 第 1923-1929 行：`N26` 规则只写了 gate 阻断语义，没有写 OpenSpec 更新与 bundle 处理细则。

**影响**

这里的真实意图应是：provider 只能输出候选 `dispatch_package` / patch task delta，不能直接改 OpenSpec；daemon 在 gate 通过后用 Document Operation 更新 OpenSpec 并重编译 bundle。

但当前文档没有把“provider 写权限”和“daemon 文档操作权限”拆开写。研发可能有两种错误实现：

- 让 provider 直接改 `openspec/changes/<changeId>/tasks.md`。
- 因为写权限表说 `N26` 只能写 artifact/stdout，而跳过 OpenSpec `tasks.md` 更新。

**建议修正**

在 `IO协作协议_v1.1` 第 6.2、`P4计划_v1.1` Task 0 和 `评审后实施规格补齐_v1.3` 第 14.3 章补一句裁定：

> `N26` provider 只允许产出候选 `dispatch_package` 或 patch task delta；OpenSpec `tasks.md` 的实际更新必须由 daemon 通过 Document Operation 执行。更新后必须标记当前 bundle 为 `stale`、重编译，并让新 `dispatch_package` 绑定重编译后的 `taskConstraints`。

---

## 5. 非阻塞优化

### R1：面向人工研发的阅读入口可以再收敛

P1-P4 每份计划开头都有英文 `For agentic workers` 说明和 required sub-skill。它对自动化 agent 有价值，但对人工研发会干扰阅读主线。

建议保留，但移动到附录或统一放在总览“自动化 agent 执行说明”中。每份子计划开头只保留目标、架构、范围和出口。

### R2：总览应同步列出本轮新增准入门槛

`实施计划总览_v1.1` 已经有评审后准入门槛。建议补充：

- P2 启动前必须明确 OpenSpec scope 规则。
- P3 启动前必须保证 provider adapter baseline 测试不依赖尚未创建的 context builder。
- P4 启动前必须明确 `M20/N20-N22` 代码落位、`candidateCommitSha` snapshot 字段、`N22/N23` Git 操作边界和 `N26` daemon/provider 写权限边界。

---

## 6. 已确认修复项

| 上轮问题 | 当前状态 |
|----------|----------|
| IO 文档 `ExternalArtifactRef` / `CanonicalArtifactOrigin` 旧字段口径 | 已修复，IO v1.1 明确以补齐规格 v1.3 第 4.4 章为准 |
| `RuntimeUnit.node_id()` 可能返回实现侧 ID | 已修复，v1.3 明确 `node_id()` 只能返回协议节点 ID |
| P2 validator 分层冲突 | 已修复，v1.3 与 P2 v1.2 均明确 `canonical_validator` / `projection_validator` / `phase1_profile_validator` |
| P4 contract / workflow / prompt registry 缺失 | 已修复，P4 v1.1 新增 Task 0 |
| candidate commit 只有消费者没有生产者 | 基本修复，P4 v1.1 明确 `N20 ready` 前创建；还需补 snapshot 字段 |
| `N26` OpenSpec bundle stale / recompile 缺失 | P4 v1.1 已补；还需同步到 v1.3 与写权限边界 |

---

## 7. 建议修订清单

按优先级处理：

1. 在补齐规格 v1.3 与 P2 计划中裁定 OpenSpec `scope` 默认派生规则，并同步 fixture 路径。
2. 修改 P3 Task 1：不要在 `provider_context_builder.rs` 创建前要求 `cargo test --test context_builder` 通过。
3. 修改 P4：明确 `N20` 代码落位。推荐 `integration_prepare.rs` 作为 `M20` 覆盖 `N20/N21/N22`。
4. 修改补齐规格 v1.3 第 11 章：为 `N20/N22` 增加 `candidateCommitSha` 相关 `nodeSpecificFields`。
5. 修改 P4 Task 3：把 cherry-pick conflict 处理从 `N22 preflight` 移到 `N23 integration_execute`。
6. 修改 IO 文档、补齐规格 v1.3 和 P4 Task 0：明确 `N26` provider 只产候选，daemon 才能通过 Document Operation 更新 OpenSpec 并重编译 bundle。
7. 修改实施计划总览：同步以上准入门槛。

---

## 8. 一句话总结

当前方案主干已经能指导研发实现，但还不能算“无歧义开工”。修完 OpenSpec scope、P3 测试顺序、P4 `N20/N22/N26` 边界后，研发才能稳定地按文档落地，而不需要在代码实现时自行补协议裁定。
