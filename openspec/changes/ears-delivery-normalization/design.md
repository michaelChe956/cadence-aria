# Design: ears-delivery-normalization

> 完整诊断依据：`.superpowers/sdd/2026-09-23_计划文档_change实施_plan-compile-gate-visibility_v1.0/f48-ears-fail-diagnosis.md`（F48Diag，codepoint 级实证）。

## Context

F-48：pi 交付 36 条 statement 恰 2 条 `」THE SYSTEM SHALL`（无半角空格）→ `parse.rs:488-495` 以 `" THE SYSTEM SHALL "`（grammar.rs:24 字面量）split_once 判非法 → 编译 loop（`src/web/workspace_ws_handler/run/single_candidate.rs:484-556`）教学重驱只认 missing_section（:494-497）→ 终态。形态由上游 Story Spec 的 `「」` 引号习惯定向播下，prompt 消除不了。

## Decisions

### D1 归一化落点=既有交付归一化层（normalize.rs + prepare_author_delivery_for_compile）

与结构标题归一化同层同入口，author 与人工修订两路自动共用；不做第二套入口。修复范围严格限定三类：`WHEN` 后补空格、`THE SYSTEM SHALL` 前补空格、关键字邻位 U+3000/NBSP→半角。护栏：只允许在已知关键字与邻位空白上操作，正文与 CJK 标点零触碰；每次救回落审计诊断/事件（与结构标题归一化同形态）。

### D2 教学重驱=白名单扩展而非通用 retry

`first_round_failure` 匹配从 `missing_section` 扩为 parse 语法类集合（invalid_ears / invalid_work_item_id 等未收敛者）；复用既有 code:line:message 回灌；「每 candidate 至多一次」上限与失败消息 `(after one teaching re-drive)` 后缀语义不变。诊断 open item ⑥：实施必须补「非 missing_section parse 类失败触发重驱」回归用例（现状无网）。

### D3 prompt 判例净增（预防层，非安全边界）

CJK 空格规则（prompts.rs:180）后补 `」` 收尾正/反判例各一；预算红线 22,000 字节内净增（实施时以 contract 断言核对）。

### D4 明确否决面

不放宽 `is_ears_statement`（EARS 前缀字面量是教学/fixture 共同契约）；invalid_ears 不进 contract_autorepair（那是 required_capability_missing 等候选语义类收敛的地盘，语法空白归一在编译前一层完成更早更确定）。

## Risks / Trade-offs

- 归一化层改动 REQ-WSC-02 已声明边界（「正文内容零触碰」→「关键字邻位空白除外」）——spec delta 已同步修订，防两套口径
- 重驱扩面增加一次额外 author 驱动的 token 成本（失败场景才发生，可接受）
- 归一化误伤风险：白名单关键字+空白位置确定性强，回归用例覆盖 F-48 真实两行原文（L158/L193 fixture）

## Open Questions（实施首步验证）

- 重驱白名单的精确类别集以 parse.rs 错误枚举实查为准（诊断已列 invalid_ears/invalid_work_item_id，实施时全量核对未收敛 parse 类）
