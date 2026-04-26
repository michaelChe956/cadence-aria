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

当前一期研发按以下顺序阅读：

1. 先读 `2026-04-23_技术方案_Aria一期MVP精简设计_v1.2.md`，确认一期边界、实现单元和不可简化项。
2. 再读 `2026-04-23_技术方案_Aria一期实现总契约_v1.0.md`，确认对象模型、wire protocol、Provider 调用链、OpenSpec 生命周期和阶段收口。
3. 再读 `2026-04-26_技术方案_Aria_IO协作协议与Provider契约_v1.1.md` 与 `2026-04-26_技术方案_Aria一期评审后实施规格补齐_v1.4.md`，确认 IO、Provider DTO、RuntimeUnit、Projection/OpenSpec 编译、prompt、fixture 和 Git 集成规则。
4. 再读 `2026-04-26_技术方案_Aria一期研发导读与实施拆解_v1.2.md`，按负责人视角拆分模块。
5. 最后按 `cadence/plans/2026-04-26_计划文档_实施计划_Aria一期实现计划总览_v1.2.md` 与 P1-P4 子计划执行。

上游文档集仍按以下顺序作为 canonical 协议参考：

1. 总览文档，理解系统目标、角色分工、MVP 边界与演进方向。
2. 全局协议文档，理解 session、task、phase、artifact、approval、provider run、worktree、event 的统一规则。
3. 节点总目录，确认完整节点集合与上下游关系。
4. 需要理解具体执行时，再进入对应节点文档、横切能力文档和产物规范文档。

## 当前状态

- 上游协议文档集仍作为 canonical 协议参考。
- 一期研发实现以 MVP v1.2、实现总契约 v1.0、IO / Provider 契约 v1.1、评审后实施规格补齐 v1.4 和 P1-P4 实施计划为准。
- 旧入口中出现的导读 v1.1、规格补齐 v1.3、计划总览 v1.1 均视为废弃引用；研发不得按旧版本补字段或写 fixture。
- implementation plan 已进入文档化阶段，当前不再以“等待用户确认后才进入 implementation plan”的旧状态判断。
