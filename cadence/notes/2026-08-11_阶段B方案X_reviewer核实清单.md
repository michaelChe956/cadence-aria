# 阶段 B 方案 X 核实清单（reviewer 用，精简版）

## 方案 X 核心（两阶段创建，解决 finding1 时序）

**问题**：`create_story_spec` 创建前同步调 `validate_aggregate_story_scope`（spec.rs:390），空 involved 立即 blocker。但 involved 由 AI 在 WebSocket run 后产出。所以「先 create 空 scope 再回填」不可能。

**方案 X**：
1. **阶段1 handler**：create 时 aggregate_codebase=Some{involved:vec![]}，**放宽 validate 让 Draft 态允许空 involved**（仅校验给出的 involved ∈ effective）→ 注入 prompt → 返回 session
2. **阶段2 WebSocket AI run**：AI 产出 `<ARIA_STRUCTURED_OUTPUT>{involved_repository_ids}` → `extract_structured_json` 解析 → **在 provider_drive.rs:779 的 append_version 处回写** → 新增 update_*_spec_aggregate 更新 record
3. **confirm gate**：confirm_workspace_entity（lifecycle.rs:800）对多仓校验 involved 非空 + Design involved>1 须 change_order

## reviewer 需核实的 4 个代码落点（只读这些，勿发散）

| # | 落点 | 核实问题 |
|---|---|---|
| 1 | `src/product/lifecycle_store/spec.rs:390` `validate_aggregate_story_scope` | 能否加 Draft 态判定放宽（Draft 允许空 involved）？create 时 status 是 Draft（spec.rs:68）吗？|
| 2 | `src/product/workspace_engine/parsers.rs:14` `extract_structured_json` | 这机制成熟可用吗？（schema: involved_repository_ids JSON）|
| 3 | `src/product/workspace_engine/provider_drive.rs:779`（Story\|Design 分支 append_version）| 这是 AI run 后回写 involved 的精确落点吗？在此解析+update 可行？|
| 4 | `src/web/handlers/lifecycle.rs:800` `confirm_workspace_entity` | 能在此加 involved 校验（confirm gate）吗？|

## 输出（简短即可）
1. 4 个落点各：可行/不可行 + 一句证据
2. 方案 X 时序：自洽/有漏洞
3. 总判定：Approved（可写 plan）/ 需修改

**只核实这 4 点，不要读设计文档全文，不要发散新问题。**
