## 背景

两套同名机制并存，职责重叠但地位悬殊。

| | `HandoffRevision` | `WorkItemHandoff` |
|---|---|---|
| 定义 | `src/product/models/work_item_revision.rs:187` | `src/product/coding_models/plan.rs:49` |
| 落盘 | issue 级 lineage，跨 attempt 存活 | attempt 目录 `units/<unit>/work-item-handoff.json` |
| 内容 | 结构化契约与能力清单 | 自然语言摘要 |
| 生成 | group completion 从契约编译产出 | provider 调用生成 |
| 校验 | `runtime_handoff_authority.rs` 比对 commit / revision / status，失败关闭 | 无 |
| 消费 | 下游 unit、运行时校验、reviewer / tester 上下文 | reviewer 读摘要、验收 `ac_handoff_published` |

真正承担交接职责的是前者。后者的六类字段中五类可由 diff 或 `HandoffRevision` 得到，唯一独有的是自然语言意图与下游提示。

## 根因

`generate_work_item_handoff_from_provider`（`handoffs.rs:401`）的 prompt 要求 provider 汇报 `files_changed`、`diff_summary`、`tests_run`，却不在 prompt 中提供这些信息；provider 只能自行查 git。而 `default_compatibility_matrix`（`src/cross_cutting/adapter_compatibility.rs`）给 claude 的 run_command 带 `--tools ""`。

实测（真实 worktree，相同命令与 prompt 形态）：provider 返回 exit 0，输出 267 字节，内容为一次工具调用尝试，不含 `<ARIA_STRUCTURED_OUTPUT>` sentinel。`cli_adapter.rs:119` 的 `parse_last_structured_output` 因此报 parse error。

该路径是结构性必然失败，而非偶发。

## 决策

### 决策一：移除而非修复

修复方案（在 prompt 中直接注入 diff，或为 handoff 开放工具权限）技术上可行，但产出仍是冗余数据。既然五类字段已有更准确的来源、且唯一独有字段不足以支撑一套生成机制与验收依据，移除比修复更彻底。

### 决策二：`HandoffRevision` 的 `tests` 与 `artifacts` 一并移除

`build_group_handoff_revision`（`group_completion.rs:576-602`）中，`tests` 来自交接摘要的 `tests_run`，`artifacts` 来自 `files_changed`。移除交接摘要后二者无数据源。

- `tests` 依赖 testing 阶段，该阶段已决定废弃。
- `artifacts` 与 git diff 冗余。

保留恒空字段会给后来者留下误导。因此移除，`HandoffRevision` 收敛为纯契约凭证。

被否决的替代：保留字段恒空（改动更小但留下失效语义）；`artifacts` 改由 completion commit 的 git diff 计算（数据更准但为冗余信息增加 git 调用）。

### 决策三：既有 lineage 记录必须保持可读

`HandoffRevision` 持久化于 issue 级 lineage，跨 attempt 存活，既有记录含 `tests` 与 `artifacts`。移除字段后反序列化必须忽略多余字段而非失败，否则历史 lineage 不可读，将破坏运行时权威校验与历史追溯。

### 决策四：legacy 前置校验一并移除

`src/web/handlers/coding.rs:117-135` 在启动 coding 前校验 `required_handoff_from` 中的上游是否都有 `handoff_summary_ref`，缺失则拒绝。而 `handoff_summary_ref` 仅在非 schema v2 路径写入（`handoffs.rs:269` 判断 `schema_v2_group_plan_lineage` 为 `None`）。

移除交接摘要后该字段永不写入，校验对所有流程恒定放行，属失效逻辑。保留它比移除更具误导性，故一并移除 `handoff_summary_ref`、`required_handoff_from`、`planned_handoff_summary` 与该校验。

### 决策五：严格区分 legacy 交接摘要与 schema v2 契约

`work_item_split_engine` 同时含两类 handoff 命名，必须区分：

| 标识 | 归属 | 处置 |
|---|---|---|
| `required_handoff_from` | legacy 交接摘要前置 | 移除 |
| `max_handoff_chars`、`max_dependency_handoffs` | legacy 摘要注入预算 | 移除 |
| `handoff_contract`（`schema.rs:282`） | schema v2 契约体系 | **保留** |
| `handoff_field`（`schema.rs:243`，`EvidenceKind` 成员） | schema v2 证据类型 | **保留** |

误删后两者会破坏 `HandoffRevision` 的编译来源。

### 决策六：前端与协议一并移除

交接摘要经 WebSocket 协议（`protocol.rs:58`、`types.rs:559`）暴露给前端并被状态层消费。后端移除而前端保留会留下永不赋值的字段与死组件，故同批移除。

## 边界

- 不改 `HandoffRevision` 的契约字段语义与运行时权威校验判定口径。
- 不改 schema v2 的 `handoff_contract` 与 `handoff_field`。
- 不改 group completion 的 unit 完成判定、commit 绑定与 `HandoffRevision` 发布路径（除不再读取交接摘要）。
- 不改 testing 阶段本身：其移除由 `remove-testing-stage` 负责。本 change 只移除交接摘要中依赖 testing 的字段。
- 不自动清理历史遗留的 `work-item-handoff.json`。
