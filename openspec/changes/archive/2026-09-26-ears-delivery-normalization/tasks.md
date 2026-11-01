# Tasks: ears-delivery-normalization

**工作包性质**：只登记高层工作包与验收口径；精确步骤由 `superpowers:writing-plans` 展开到 `cadence/plans/`。

**全局边界**：不放宽 EARS 校验器字面量契约；不动 contract_autorepair 既有收敛范围；不动 Final Compile recovery 门；「每 candidate 至多一次教学重驱」上限不变。

## 1. EARS 关键字空白归一化

- [x] 1.1 在 `normalize.rs`（prepare_author_delivery_for_compile 入口）实现三类确定性空白修复（WHEN 后补空格/THE SYSTEM SHALL 前补空格/关键字邻位 U+3000/NBSP→半角），护栏=仅关键字邻位、正文零触碰、救回落审计事件；以 F-48 真实原文 L158/L193 为红绿 fixture（REQ-WSC-02 新场景）。
- [x] 1.2 非空白语法错误不救（缺 WHEN 前缀等照旧 fail-closed）+ 未知标题仍拒（既有场景零回归）的回归用例。

## 2. 教学重驱白名单扩展

- [x] 2.1 编译 loop first_round_failure 匹配扩为 parse 语法类集合（以 parse.rs 错误枚举全量核对），复用 code:line:message 回灌；补「invalid_ears 触发一次重驱」「上限不被突破」「非 parse 类不触发」回归用例（诊断 open item ⑥：现状无网）（REQ-WSC-09）。

## 3. prompt 判例与门禁

- [x] 3.1 CJK 空格规则后补 `」` 收尾正/反判例（22,000 字节红线内净增），prompt contract 断言同步（随批）。
- [x] 3.2 change 级 strict 校验 + lib/it_web 全量 + fmt/clippy；记录 F-48 原文端到端救回证据。
