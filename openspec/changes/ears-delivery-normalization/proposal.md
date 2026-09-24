# Proposal

## Why

F-48 实证缺陷：SC author（pi）交付的 plan markdown 中 36 条 statement 恰 2 条以全角引号 `」` 收尾后无半角空格直接接 `THE SYSTEM SHALL`（对上游 Story Spec「重新开始」按钮表述的逐字承接），编译器 `is_ears_statement` 以字面量 `" THE SYSTEM SHALL "` 切分判非法（invalid_ears:158/193）→ 编译 loop 的「恰一次教学重驱」硬编码只认 missing_section → 会话直接终态失败。用户重跑即复发：同族 prompt 明令使用中文引号「」，该形态注定由上游规格引号习惯定向播下，prompt 判例消除不了；且预算入口不可达（repairs_used=0，非耗尽）。

## What Changes

1. **交付侧确定性 EARS 关键字空白归一化**：在既有 `prepare_author_delivery_for_compile` 归一化层（与结构标题归一化同入口）新增三类纯空白修复——`WHEN` 后缺半角空格补齐、`THE SYSTEM SHALL` 前缺半角空格补齐、关键字邻位 U+3000/NBSP 归一为半角空格；零语义改写、落审计事件；author 与人工修订两路共用。
2. **教学重驱白名单扩展**：编译 loop 的「恰一次教学重驱」从仅认 missing_section 扩展到 parse 语法类失败（invalid_ears、invalid_work_item_id 等未收敛者），复用既有 code:line:message 回灌 prompt；守住「每 candidate 至多一次额外驱动」上限不变。
3. **prompt 判例净增**：CJK 空格规则后补 1 条 `」` 收尾正/反判例（随批，非安全边界）。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `work-item-plan-single-candidate`: REQ-WSC-02 归一化范围从「结构标题行」扩展到 EARS 关键字空白（扩已声明边界，正文语义仍零触碰）；新增「generate 段编译语法失败的教学重驱」requirement

## Non-Goals

- 不放宽 EARS 校验器（`is_ears_statement` 明文契约不动——诊断已证严格空格是契约约定，放宽会与教学/fixture 期望分叉）
- 不改 Final Compile recovery 门（plan-compile-gate-visibility 契约，阶段不同）
- 不改 contract_autorepair 既有收敛范围（invalid_ears 走新归一化层而非收敛器）
- 不新增终态显式重开按钮（「开始生成」复用入口已存在且可用，诊断已验证）
