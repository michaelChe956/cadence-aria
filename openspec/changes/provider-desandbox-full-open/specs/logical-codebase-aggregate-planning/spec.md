# 规范增量：聚合规划角色全开

<!-- 来源：openspec/specs/logical-codebase-aggregate-planning/spec.md:8-17,50-55。本 delta 改 PLN-01/06 并按 cadence/designs/2026-10-22_技术方案_评审范围对齐Issue选集与多成员对账域_v1.0.md 第十节 F 表新增 PLN-11/12/13；REQ-LCG-04 实际在 lc-gateway-multi-provider。anchor 的新 native 身份规则仍由其自身依赖工作流实施，不在这里冒领。 -->

## MODIFIED Requirements

### Requirement: 聚合规划上下文（REQ-PLN-01）

系统 SHALL 将规划 workspace 上下文组织为逻辑代码库聚合，包含 logical_codebase、成员 inventory（id/alias/path/role/profile 摘要）、focus 与索引 revision；从 canonical 聚合根启动并使用全角色宿主基线，不能由聚合 cwd 推导只读。prompt SHALL 注入紧凑成员清单与预算而非全部源码，超预算按已有确定性次序截断并标记未检索成员，具体阈值沿用已确认设计与 Plan。

#### Scenario: 聚合上下文注入
- **WHEN** 为 LC Issue 启动 Story/Design/Work Item 计划生成
- **THEN** provider SHALL 从 canonical provider_context_root 以宿主全开基线启动，注入紧凑成员清单和预算，不注入全部成员源码；author 岗位工具规则单独生效

#### Scenario: 预算超限
- **WHEN** inventory/摘要/证据超出 token 或 byte 预算
- **THEN** 系统 SHALL 按既有确定性顺序截断并标记未检索成员，不撑爆 context window，不借去沙箱重定义截断策略

### Requirement: 规划只读边界（REQ-PLN-06）

规划 provider SHALL 使用四家全角色宿主基线，不再要求 best_effort_configured/production_verified_readonly，不宣称 OS 不可写。author 保留代码编辑岗位纪律和平台文档通道；Codex danger 下按 REQ-ENV-09 检测型 fileChange 纪律验收，不承诺工具不可达。Kimi 规划 Terminal/其余工具与其它 provider 一致，不因 Orchestrator/Splitter/Handoff 整体拒绝；规划 Reviewer 另受 B 档采信约束。plan reviewer 的零写对账覆盖 Issue 选集成员全集，按 REQ-ENV-13/14 冻结范围并按集合采信；ZeroWrite 是采信政策，不是物理不可写承诺。

#### Scenario: 规划运行
- **WHEN** 任一家 provider 完成规划 run
- **THEN** 系统 SHALL 如实记录宿主非隔离执行和岗位规则，不要求“成员 checkout 与聚合根物理不可写”或 Codex read-only 证明，不把正常 terminal 能力判为越权失败

#### Scenario: Kimi 规划 Terminal 可执行
- **WHEN** Kimi 的 Orchestrator、WorkItemSplitter 或 Handoff 请求普通 argv 或 shell terminal，并满足当前权限审批
- **THEN** 系统 SHALL 正常执行，不因角色全拒、只读命令表、bwrap 缺失或解释器限制拒绝；若该运行承担 author 职责，编辑代码文件内建工具岗位规则仍适用

#### Scenario: 作者产出与规划身份保持
- **WHEN** author 生成 Story/Design/Plan 或请求编辑代码文件内建工具
- **THEN** 文档 SHALL 走平台产物通道且不由作者提交 Git；具备过滤机制的 provider 不配代码编辑内建工具，Codex 按检测型 fileChange 纪律中断并使该轮失败，不能宣称事前防写；terminal/MCP/skills 不额外禁用，聚合快照、authority 与业务成员归属继续有效

#### Scenario: plan reviewer 全集零写对账
- **WHEN** plan reviewer 对 Issue 选集成员全集完成评审并进入收口
- **THEN** 系统 SHALL 逐成员比对冻结基线并在恢复前落违规证据，任一成员差异即使整轮作废；不因恢复成功放行，也不因 action 名称宣称物理不可写

## ADDED Requirements

### Requirement: 评审范围对齐 Issue 选集全集（REQ-PLN-11）

Author 与 plan reviewer SHALL 覆盖同一有效成员全集（Issue 选集 include 且未 exclude、未删除/停用）。Story 的 involved、Design 的 involved/change_order、WorkItem 的 target 与 focus SHALL 只是产物声明与评审重点，不得收窄该集合，不得作为授权上限或对账域。primary SHALL NOT 决定评审集合，禁止首仓锚定；集合或成员身份失效时 SHALL 等待显式新轮。

#### Scenario: 阶段声明不收窄评审集合
- **WHEN** Issue 选集含 A/B/C 三成员，Story focus 仅声明 A、Design involved 仅声明 B、WorkItem target 仅指向 A
- **THEN** plan reviewer SHALL 覆盖 A/B/C 全集；阶段声明不收窄集合，被排除或未选中成员不得冒充已评

#### Scenario: primary 不决定评审集合
- **WHEN** 多仓 issue 的 primary repository 与其它成员并存
- **THEN** 评审集合 SHALL 为 Issue 有效选集全集，primary 仅作 UI 归属与兼容投影；不得以首仓锚定或「非主成员未对账但继续」路径交付

#### Scenario: 集合失效等待显式新轮
- **WHEN** 冻结后成员失效、selection 变更或成员身份漂移
- **THEN** 系统 SHALL 使本轮不可采信并等待显式新轮，不沿用旧集合、不静默缩小范围

### Requirement: plan 评审报告成员分段与总判定（REQ-PLN-12）

plan 评审 SHALL 产出一份报告并按成员分节：每个选中成员恰一节，含成员 ID、状态（reviewed/unreviewable/failed/not_reviewed）、成员 verdict、finding 归属与 raw 引用、window 与 baseline 标识，并给出唯一确定性总判定。总判定 SHALL 由服务端从集合与窗口事实计算，模型不得伪造 accepted；completeness 为 partial 时 acceptance 不得为 accepted。

#### Scenario: 成员分段与唯一总判定
- **WHEN** 选集为 A/B/C 三成员且全部成员评审完成
- **THEN** 报告 SHALL 恰含三节、每节 finding 归属可校验、跨成员问题不重复计数，并有且只有一个总判定

#### Scenario: PARTIAL 仅诊断不推进
- **WHEN** 某成员不可评审、provider 或解析失败、恢复未完成或 journal pending
- **THEN** 报告 SHALL 保留 partial 诊断且总判定不可通过，不路由 pass、不触发自动返修或交付推进；其它成员的成功不得掩盖该失败

#### Scenario: 采信先于成功路由
- **WHEN** 成员评审已完成但集合尚未完成收口与对账
- **THEN** 系统 SHALL 不发送 pass 或 Completed 成功路由，不先发成功再附警告

### Requirement: 多成员评审入口启用门（REQ-PLN-13）

多成员 plan 评审入口 SHALL 仅在范围冻结、集合屏障、ZeroWrite 对账、成员分段报告与总判定、fail-closed 与恢复闭环全部落地并通过验收后启用。不得以单仓锚定、仅分段报告、跳过失败成员或漏检的过渡模式交付；未达门禁时 SHALL 保持多成员入口关闭或只走既有单成员路径，不得声称已实现全集对账。

#### Scenario: 闭环前不启用多成员入口
- **WHEN** 集合屏障或 ZeroWrite 对账尚未完成验收
- **THEN** 多成员 plan 评审入口 SHALL 保持关闭或只走既有单成员路径，系统不得以漏检的过渡模式对外提供多成员评审

#### Scenario: 启用门禁逐项可核
- **WHEN** 申请启用多成员 plan 评审入口
- **THEN** 系统 SHALL 逐项核验范围冻结、集合屏障、零写对账、分段报告、总判定与恢复闭环的验收证据，缺任一项即拒绝启用
