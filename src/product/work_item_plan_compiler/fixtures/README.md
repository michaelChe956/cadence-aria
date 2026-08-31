# fixtures 目录说明

本目录是测试消费 fixture 的**唯一活来源**（阶段 1 先例：`src/product/work_item_plan_policy/fixtures/golden_findings.json`）。

`openspec/changes/archive/2026-08-31-rearch-workitem-plan-pipeline/` 内的同名文件是**归档冻结快照**，仅为 change 历史的组成部分，不再同步、不再修改。

历史背景：这些 fixture 原放在 change 目录下并由测试 `include_str!` 引用；2026-08-31 归档 change 时目录迁移导致编译断裂，遂按阶段 1 模式将活副本落位本目录（reviewer Approved，双副本无防漂移断言——归档快照不是活数据）。

教训（入流程认知）：长期存活的 golden fixture 应从第一天就放 `src/` 下，避免归档 change 目录导致编译断裂。
