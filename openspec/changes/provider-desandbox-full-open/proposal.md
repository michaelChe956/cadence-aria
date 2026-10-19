# 变更提案：四 Provider 全角色应用层全开与评审对账作废

## Why

Bug4 的 Reviewer 污染与自提交、Bug5 的 Coder CHECK 无法执行，需要按 owner 已确认的同 UID 宿主全开基线一次收敛；现行只读、永久拒绝 danger-full-access 与 write_boundary 物理隔离门会把新行为继续判错。本提案仅持久化 2026-10-17 总览及去沙箱设计最终修订，不重新裁决产品决策。

## What Changes

- **BREAKING**：Claude Code、Codex、Pi、Kimi Code 的全部角色统一宿主直跑，退役应用层 OS 沙箱；Codex LC 全相位 `danger-full-access` 为明示终裁，Direct 同步全开是“诉求 1：四家全角色统一”的推论，并非锚点对 Direct 的逐字裁定；审批维度仍独立。
- **BREAKING**：作者仅保留“不配编辑代码文件内建工具”的岗位规则；Codex danger 下无预防保证，以 `fileChange` 事件检测后中断 turn、失败关闭和 durable 审计实现检测型岗位纪律，必须真机探针验收。Reviewer 不继承作者 deny，采用快照恢复、平台通道锁、违规整轮作废的 B 档，无独立 UID。
- **BREAKING**：LC gateway 保留真实能力证据、身份/配置/投影冻结及 spawn 复验，但撤销“宿主写成功即能力失败”的只读/target-only 证明门；旧隔离能力记录不能自动当作新基线证据。
- Kimi 的终端 OS 隔离、封闭 grammar、角色与 fs 三条合同整体改为全角色宿主执行；fs API 的 no-follow/授权根校验与资源护栏保留，长 CHECK 不受独立 120 秒硬截断。
- Coder 在宿主环境自行执行 CHECK，环境能力声明不等于授权；A′ verification-triage 仅补充，不新增平台通用命令 runner。
- 平台 API 鉴权（REST、WS、敏感 GET）及 Origin/Host 校验为无条件上线前置，不等风险触发、外网暴露或二期部署。

## Capabilities

### New Capabilities

- 无其它全新 provider capability；LC 网关以 controller 先归档原始 change 后产生的真实主规范为 MODIFIED 基线，不创建第二份 ADDED。
- `platform-api-authentication`：承载平台控制面的统一入站鉴权、敏感读取和浏览器跨站防护。

### Modified Capabilities

- `lc-gateway-multi-provider`：修改真实主规范的 Codex 基线、角色守卫、write_boundary 证据语义及双阶段门；显式废止 Unknown 迁移放行桥，仅当前语义 Confirmed 允许启动。
- `session-policy-envelope`：修订 REQ-ENV-01/02/03/05/09 的角色策略与受限档语义，补充受信宿主执行及 Reviewer B 档要求。
- `logical-codebase-aggregate-planning`：修订 REQ-PLN-01/06 的规划只读承诺，保留聚合上下文与作者岗位纪律。
- `kimi-acp-client-services`：三条真实沙箱合同及终端生命周期配套修订，保留 fs API 校验、并发与输出预算。
- `kimi-code-provider-integration`：ACP 会话、Auto/Supervised 与逐工具审批三条集成合同明确采用全角色宿主执行，不再把审批当隔离或额外角色禁用。
- `coding-provider-transport-retry`：Reviewer 技术重试改为新的完整 B 档审查窗口，不重建已退役的只读会话。

## Impact

- 后续实现涉及受信 engine builder、Direct/LC gateway、四家 adapter、Kimi ACP client services、平台 Git 启动与窗口恢复、API 路由与鉴权；本阶段不修改任何 `src/`、现有设计或笔记。
- 保留 authority、canonical cwd/native target、policy/config/MCP/trust、原生 session 身份、精确版本观测与失败关闭，不借去沙箱绕过材料缺失或角色配置。
- owner 接受同 UID 读取凭据、恶意依赖执行、审计非物理防写及外部副作用不可回滚；不新增 UID 隔离、cgroup、容器调度器、CHECK schema 路由或升级自动触发。二期 dev-sandbox 由 owner 自行安排。

### 同 capability 冲突面与归档顺序

原 `lc-gateway-multi-provider` 须先完成 strict 与十四项实施证据核验，再由 controller 归档建立真实主规范，本 change 网关 delta 只使用 MODIFIED。`fix-planning-session-anchor-recovery` 不作为已完成合同，不以 supersedes 标注代替修订；它排在去沙箱③之后，须先按依赖门最小修订其 Codex/角色/写边界冲突条款。本 change 的 tasks 设置归档硬门，未修订不准归档；重叠 header 与失效措辞清单见 design.md。
