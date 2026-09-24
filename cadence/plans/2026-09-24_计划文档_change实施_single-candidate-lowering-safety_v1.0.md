# change 实施 single-candidate-lowering-safety 实施计划 v1.0

> **For agentic workers:** 按 superpowers:executing-plans / subagent-driven-development 执行。

**Goal:** SC 编译器 lowering 重复字段（capabilities/required_capabilities）从 last-write-wins 改为按序累积+稳定去重，消灭 F-52 假阳性工厂。

**Architecture:** lower.rs 两处（outputs/inputs）共享 append+first-seen-dedup helper + contract 边界 flush；零外部耦合。

**Spec:** `openspec/changes/single-candidate-lowering-safety/`（strict valid）；实证 `.superpowers/sdd/2026-09-24_计划文档_change实施_ears-delivery-normalization_v1.0/f52-diagnosis.md`（CT-001 案例）。

## Global Constraints

- 不排序/不改写 capability 值形态（大小写/标点/内部空白零触碰），仅消除完全相同重复。
- contract 边界 flush：无当前 entry 的字段按既有 fail-closed 规则处理，不归给下一 contract。
- 不改 validator coverage 口径/freshness 语义；历史 IR/publication 零迁移。
- 测试命令照仓规。

## Review Focus

1. 跨 contract 串项（边界 flush 缺失）→ 1.1 用例
2. 值形态被隐性改写（trim/排序混入）→ 1.1 字节级断言
3. fixture 保真（CT-001 真实形态从 durable 提取，python json 读勿 jq）→ 1.2
4. 真实缺能力被误救（累积只消除重复，不造能力）→ 1.2 负例

---

### Task 1: 累积语义实现

- [ ] 1.1 失败测试：①多行能力并集（场景 6）②完全重复去重 ③required_capabilities 同构（场景 7）④相邻 contract 隔离（场景 8）⑤字段错位/空值/未知字段负向 ⑥值形态字节级断言（场景 9）。
- [ ] 1.2 fixture：从 durable 提取 F-52 CT-001 真实 6 行形态（`.aria/.../workspace_session_0009/artifact_versions.json` 早期版本 markdown；python json 读）→ 端到端：不再 missing_capabilities 假阳性；负例=删一行真实能力仍 Error（场景 5 边界）。
- [ ] 1.3 重编译 IR 差异仅源于重复字段断言（场景 10）+freshness 零回归。
- [ ] 1.4 strict 复跑+compiler/validator 目标测试+lib 全量+fmt/clippy → commit `feat(compiler): lowering 重复字段累积与稳定去重（REQ-WSC-02，F-52 P0）`。

---

## Self-Review

REQ-WSC-02 新场景 6-10→Task 1.1/1.2/1.3 全覆盖；Review Focus 四条挂测试；无占位符。
