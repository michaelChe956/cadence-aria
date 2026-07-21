## 1. 规则引用与生命周期测试基础

- [x] 1.1 建立最小的 Cadence 原始规则引用插入工具，并为新建/恢复任务、纯格式修复和禁止 `cadence-workflow` 的行为添加回归测试。
- [x] 1.2 锁定现有 OpenSpec/Superpowers 合同、角色权限和结构化输出契约，确保规则接入不改变既有解析语义。

## 2. Workspace 与 WorkItemPlan 提示词接入

- [x] 2.1 为 Story Spec、Design Spec 的 author、reviewer 与真实 resume 入口接入当前阶段的 Cadence 原始规则引用，并保持 Aria gate 与候选 artifact 边界。
- [x] 2.2 为 WorkItemPlan outline、draft、review、revision 与 Work Item 相关入口补强规则引用，同时保留既有 traceability、`writing-plans`、最少拆分和验证计划合同。

## 3. Coding、审查与 Runtime Unit 提示词接入

- [x] 3.1 为 Coder、bounded rework、Tester、CodeReviewer 和 GroupFinalReview 的新建或恢复入口接入对应阶段的规则引用，保留材料驱动执行与审查协议。
- [x] 3.2 为独立 Runtime Unit Provider 节点接入实际阶段的规则引用，保留 N04–N27 的候选产物、daemon writeback 和结构化输出边界。

## 4. 回归验证与提示词审查

- [x] 4.1 运行覆盖 Story、Design、Work Item、Coding、Code Review 和组级 PR Review 的定向 prompt 测试，验证格式修复未被重新路由。
- [x] 4.2 运行项目规定的 Rust 格式、检查和测试，并审查所有最终 prompt 是否只接收当前阶段所需规则而没有重复或冲突指令。
