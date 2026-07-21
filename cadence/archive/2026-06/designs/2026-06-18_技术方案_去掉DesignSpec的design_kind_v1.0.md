# 技术方案：去掉 Design Spec 的 design_kind

**版本：** v1.0
**创建日期：** 2026-06-18
**目标分支：** `feat-b-0616`
**工作区：** `.worktrees/feat-b-0616`

---

## 1. 背景与目标

### 1.1 现状

Design Spec 当前携带一个 `design_kind: "frontend" | "backend"` 字段（后端枚举 `DesignKind`，PR #7 引入，已在 main）。该字段串起三条链路：

1. **Workspace 标题拼接**（`src/web/workspace_context.rs:237-241`）：Design Spec 的 Workspace 上下文标题被拼成 `{design.title} ({design_kind})`，导致 workbench 显示「对aria...Design Spec (frontend) (design_spec_0001)」。
2. **provider prompt 标注**（`src/product/work_item_split_engine.rs:459-465`，本分支 WP4 commit `041d8eee` 新增）：把每个 Design Spec 喂给 WorkItem Splitter provider 时，拼一行 `Design Spec: {title} ({id}) kind={design_kind}`，提示 provider 该 Design 是前端还是后端。
3. **API 契约与持久化**：`GenerateDesignSpecsRequest.design_kind`、`DesignSpecRecord.design_kind`、`DesignSpecDto.design_kind`、前端 `types.ts` 的 `design_kind` 字段。

### 1.2 问题

Design Spec 在产品语义上**不需要区分前端/后端**。前端创建入口（`IssueLifecycleWorkbench.tsx:283/487`）始终写死 `design_kind: "frontend"`，从未暴露 backend 选项；`design_kind` 实际上是一个永远为 frontend 的死字段，却污染了标题展示与 prompt。

### 1.3 目标

- 彻底删除 `DesignKind` 枚举与所有 `design_kind` 字段（后端 struct、API DTO、前端类型、持久化记录）。
- Design Spec 的 Workspace 标题恢复为 `{design.title}`，不再拼接 `(frontend)`。
- WorkItem Splitter 的 prompt 不再标注 Design 的 kind。
- **不写数据迁移**，清空本地 stores 重来。
- **不影响 WorkItem 的前后端区分能力**：`WorkItemKind` 枚举、`force_frontend_backend_split` 选项与校验完全保留。

### 1.4 非目标

- 不改 WorkItem 的 `kind` 字段与 `WorkItemKind` 枚举。
- 不改 `force_frontend_backend_split` 选项语义与 `work_item_split_validator.rs` 的校验逻辑。
- 不改 WorkItem Splitter 的输出 schema（`work_items[].kind` 仍由 provider 决定）。
- 不写 Playwright 浏览器 E2E。

---

## 2. 前提假设

> 经用户确认（2026-06-18）。

去掉 Design 的 `design_kind` 后，WorkItem Splitter provider 将不再从 Design 行获得「该 Design 是前端还是后端」的显式标注。该标注是 provider 区分同一 Issue 下并存的前端 Design 与后端 Design 的**唯一显式信号**。

本方案成立的前提：

- **短期内不会出现同一 Issue 下并存前端 + 后端两个 Design Spec 的场景。** 当前前端 UI 只能创建 frontend Design（`design_kind` 始终 frontend），不存在歧义；provider 靠 `force_frontend_backend_split` 选项 + Design 正文内容即可判断拆分。
- 一旦未来需要支持「同一 Issue 并存前端 + 后端 Design」，必须改用其他标注方式（例如让用户给 Design 打 tag，或在 Design 正文里写明前端/后端），而不是恢复 `design_kind` 字段。

在此前提下，去掉 `kind={design_kind}` prompt 片段不影响 WorkItem 的前后端区分能力与 `force_frontend_backend_split` 校验。

---

## 3. 影响分析

### 3.1 WorkItem 拆分链路（关键调研结论）

WorkItem 的前后端/大小拆分**主语是 provider**，不是 Design 的 `design_kind`：

- 入口 `WorkItemSplitEngine::generate`（`work_item_split_engine.rs:187`）→ `build_split_prompt`（:509）把 `[user_options] force_frontend_backend_split` + Story/Design 上下文 + 仓库结构喂给 provider。
- provider 按 `WORK_ITEM_SPLIT_OUTPUT_SCHEMA`（:22）输出，每个 work_item **必须**自带 `kind` 字段（:64、:84），值由 provider 自主决定。
- 后端 `WorkItemSplitValidator`（`work_item_split_validator.rs`）只校验、不决策：`force_frontend_backend_split: true` 时要求计划同时含 `WorkItemKind::Frontend` 和 `WorkItemKind::Backend`（:380-386），读的是 WorkItem 自己的 `kind`，**不依赖 DesignKind**。
- Design 的 `design_kind` 仅在 `collect_design_context`（:440-468）拼 prompt 行时被读取一次，是辅助标注，非决策来源。

因此删除 `design_kind` 不触及 WorkItem 拆分的决策与校验核心。

### 3.2 两个独立枚举

| 枚举 | 定义位置 | 值 | 挂载点 | 本方案处理 |
|---|---|---|---|---|
| `DesignKind` | `src/product/models.rs` | Frontend / Backend | `DesignSpecRecord.design_kind` | **删除** |
| `WorkItemKind` | `src/product/models.rs:280` | Backend/Frontend/Integration/E2e/Docs/Infra/Other | `LifecycleWorkItemRecord.kind` | **不动** |

`WorkItemKind` 与 `DesignKind` 互相独立，删除 `DesignKind` 不影响 `WorkItemKind`。

### 3.3 存量数据

不写迁移。本地开发环境的 stores（DesignSpecRecord JSON 文件）清空重来。删除字段后，旧 JSON 文件反序列化会因缺少 `design_kind` 而失败 —— 由用户清空本地 stores 规避。

---

## 4. 改动清单

### 4.1 后端 Rust

| 文件 | 改动 |
|---|---|
| `src/product/models.rs` | 删除 `DesignKind` 枚举定义；删除 `DesignSpecRecord.design_kind` 字段。 |
| `src/product/lifecycle_store.rs` | 删除 `CreateDesignSpecInput.design_kind` 字段；`create_design_spec`（:320 附近）不再写入该字段。 |
| `src/web/handlers.rs` | 删除 `parse_design_kind`（:2864）、`design_kind_text`（:2697）；`generate_design_specs` handler（:477-486）不再解析/传 `design_kind`；DTO 响应（:2214）不再序列化 `design_kind`。 |
| `src/web/types.rs` | 删除请求/响应 DTO 的 `design_kind: String` 字段（:417、:549 等处）。 |
| `src/web/workspace_context.rs` | Design 分支标题拼接（:237-241）从 `format!("{} ({})", design.title, design_kind_label(...))` 改为 `design.title.clone()`；删除 `design_kind_label`（:574-578）。 |
| `src/product/work_item_split_engine.rs` | 删除 `design_kind_text`（:406）；`collect_design_context`（:459-465）prompt 行从 `Design Spec: {} ({}) kind={}\n{}` 改为 `Design Spec: {} ({})\n{}`，删除 `design_kind_text(&spec.design_kind)` 参数。 |
| 构造点同步 | `workspace_context.rs:796/863/980/1087`、`workspace_ws_handler.rs:1442`、`workspace_engine.rs:9328/9610/9918`、`coding_evaluation_context.rs:613` 等所有构造 `DesignSpecRecord` / `CreateDesignSpecInput` 的位置，删除 `design_kind: DesignKind::Backend` 赋值。**注：经核实这些位置全部是测试夹具/test helper（`#[cfg(test)]` 或 test 模块内），非生产代码默认值；生产代码中 `CreateDesignSpecInput.design_kind` 一律由调用方显式传入，无默认值兜底逻辑。** |

### 4.2 前端 TS/TSX

| 文件 | 改动 |
|---|---|
| `web/src/api/types.ts` | 删除 `design_kind: "frontend" \| "backend"` 字段（:100、:828 等处）。 |
| `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx` | 两处 `generateDesignSpecs` 调用（:283、:487）删除 `design_kind: "frontend"`。 |

### 4.3 测试

| 范围 | 改动 |
|---|---|
| 后端 | 所有构造 `DesignSpecRecord` / `CreateDesignSpecInput` / `GenerateDesignSpecsRequest` 的测试夹具删除 `design_kind` 字段；所有断言 `design_kind` / `design_kind_text` / `(frontend)` 标题的测试改写或删除。覆盖 `it_web`、`it_product`、lib 单测。 |
| 前端 | `IssueLifecycleWorkbench.test.tsx`、`client.test.ts`、`lifecycle-workbench-store.test.ts`、`LifecycleCard.test.tsx` 中 `design_kind` 夹具字段删除。 |

### 4.4 不改的部分（明确边界）

- ❌ `WorkItemKind` 枚举（`models.rs:280`）
- ❌ `force_frontend_backend_split` 选项（`IssueWorkItemPlanOptions.force_frontend_backend_split`，`models.rs:455`）及其 validator 校验（`work_item_split_validator.rs:380`）
- ❌ `LifecycleWorkItemRecord.kind`（`models.rs:386`）
- ❌ `WORK_ITEM_SPLIT_OUTPUT_SCHEMA` 的 `work_items[].kind`（provider 输出契约）
- ❌ `work_item_kind_text`（`work_item_split_engine.rs:568`，仍被 WorkItem 自身使用）
- ❌ `build_split_prompt` / `build_revision_prompt` 中 `force_frontend_backend_split` 的 prompt 片段（:546、:563）

---

## 5. 风险与缓解

| 风险 | 缓解 |
|---|---|
| provider 在并存前后端 Design 时失去显式标注 | 第 2 节前提假设已确认短期内不出现该场景；未来需要时换 tag/正文方式标注。 |
| 测试夹具面广，机械改动易漏 | 实施计划按文件枚举改动点；`cargo check --locked` + `cargo clippy -D warnings` 会捕获所有遗漏的构造点。 |
| 本地旧 JSON 数据反序列化失败 | 用户清空本地 stores；非生产路径。 |
| `work_item_split_engine.rs:463`（WP4 本分支独有）改动影响 revision | revision prompt（`build_revision_prompt`）同样不读 design_kind，改法一致；WP8 贯通测试 `work_item_plan_full_flow` 验证 revision 链路不回归。 |

---

## 6. 验证链

全绿标准（遵循 `cadence/project-rules/build-test-commands.md`，🔴 禁止 `-j 1`）：

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
pnpm -C web test
pnpm -C web build
```

定向回归（快反馈）：

```bash
cargo test --locked --lib work_item_split
cargo test --locked --test it_web work_item_plan
cargo test --locked --test it_product work_item_split
pnpm -C web test -- --run IssueLifecycleWorkbench
```

---

## 7. 验收标准

- [ ] workbench 中 Design Spec 的 Workspace 标题不再带 `(frontend)`，显示为 `{design.title}`。
- [ ] `grep -rn "design_kind\|DesignKind" src/ web/src/` 仅剩 `WorkItemKind` / `work_item_kind` 相关（无 `DesignKind` 残留）。
- [ ] WorkItemPlan 全流程（prepare → author → revert → review → revision → confirm）在 Fake provider 下贯通测试全绿。
- [ ] `force_frontend_backend_split: true` 校验行为不变（计划缺 frontend 或 backend work item 仍报 `frontend_backend_split_required`）。
- [ ] 第 6 节验证链全绿。

---

## 8. 后续

本方案仅删除 Design 的 `design_kind`。若未来要支持「同一 Issue 并存前后端 Design」，需另起设计，采用 tag 或正文标注方案，不在本方案范围。
