# Aria 设计文档入口

本文件不再承载完整设计细节，Aria 的设计已拆分为文档集，避免把节点协议、横切机制、产物规范混写在一份大文档里。

## 入口导航

- 总览文档：
  [2026-04-22_技术方案_Aria文档集总览_v1.0.md](./aria-repl-runtime-docs/2026-04-22_技术方案_Aria文档集总览_v1.0.md)
- 全局协议：
  [2026-04-22_技术方案_Aria全局协议_v1.0.md](./aria-repl-runtime-docs/2026-04-22_技术方案_Aria全局协议_v1.0.md)
- 节点总目录：
  [2026-04-22_技术方案_Aria节点总目录_v1.0.md](./aria-repl-runtime-docs/2026-04-22_技术方案_Aria节点总目录_v1.0.md)
- 横切能力文档目录：
  `cadence/designs/aria-repl-runtime-docs/cross-cutting/`
- 节点文档目录：
  `cadence/designs/aria-repl-runtime-docs/nodes/`
- 产物规范目录：
  `cadence/designs/aria-repl-runtime-docs/artifacts/`

## 文档使用顺序

1. 先读总览文档，理解系统目标、角色分工、MVP 边界与演进方向。
2. 再读全局协议文档，理解 session、task、phase、artifact、approval、provider run、worktree、event 的统一规则。
3. 再读节点总目录，确认完整节点集合与上下游关系。
4. 需要理解具体执行时，再进入对应节点文档、横切能力文档和产物规范文档。

## 当前状态

- 本文档集处于 `v1.1`
- 已完成首轮设计评审，评审报告见 `cadence/designs-reviews/`
- v1.1 修复内容：
  - 全局协议新增：循环边界控制（§12）、回流失效规则（§13）、枚举值注册表（§14）、Risk Registry（§15）、Runtime Snapshot 规范（§16）、统一术语表（§17）
  - 新增横切能力：Provider Adapter 接口规范（CC10）
  - 横切能力增强：策略行为映射表、Event Log compaction、队列调度策略、Gate 超时机制
  - 产物规范增强：全部 16 个产物增加 L1/L2/L3 三级校验规则
  - 字段修复：spec 增加非功能需求、design 前后端改为条件必填、testing_report 增加覆盖率、code_review_report 去除冗余字段、final_review 增加 fail 状态
- 后续只有在用户确认文档集结构和节点协议完整后，才进入 implementation plan

