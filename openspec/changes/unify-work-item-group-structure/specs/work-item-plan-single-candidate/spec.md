# 规范增量：删除「必须把根因指向 design」的强制归因

<!-- 来源：owner 2026-10-31 第二轮「review 之后直接出错，说让人工处理的」。
     现场 2adc5e41：`owner_hint` 按本条强制断言「根因指向 design、需要 owner 修改 design」，
     但该案 design 的 change_order（3 仓、已确认、无矛盾）正常，错在 plan 把集成验收项
     托管在仓序更靠前的仓。强制归因把 owner 引向无效操作，且掩盖真实可修事实。
     替代条款：新能力 `work-item-group-structure` 的 REQ-WG-05 / REQ-WG-06
     （owner 面由 witness 事实 + 2~3 个可执行选项构成；教学面带算出的位次与保护项）。
     「候选编译失败必须在人工门完整可读呈现」仍由既有 REQ-WSC-18 承担。 -->

## REMOVED Requirements

### Requirement: 候选编译失败必须给 owner 出路与 author 重写要点（REQ-WSC-20）

**Reason**: 本条把「必须讲清是 design 的什么原因导致成环」「必须明确需要 owner 修改 design」写成无条件义务，在现场造成**事实错误**的归因（设计仓序正常时仍断言 design 有错），并以四条通用原则代替可执行事实。它的两条实质内容分别由 `work-item-group-structure` 的 `REQ-WG-05`（owner 面：witness 事实 + 可执行选项，禁止无条件归因 design）与 `REQ-WG-06`（author 面：算出的位次 + 保护项）取代，且判据由 typed witness 决定而非固定文案。

**Migration**: 已实现的 `owner_hint` / `rewrite_guidance` / `author_teaching_block`（`src/product/workspace_engine/compile_failure_surface.rs:363-445`）在 `REQ-WG-05` / `REQ-WG-06` 落地时一并改写；过渡期允许保留结构、但 MUST NOT 继续无条件断言「根因在 design」。
