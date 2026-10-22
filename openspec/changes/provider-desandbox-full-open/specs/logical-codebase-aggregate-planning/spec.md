# 规范增量：聚合规划角色全开

<!-- 来源：openspec/specs/logical-codebase-aggregate-planning/spec.md:8-17,50-55。本 delta 只改 PLN-01/06；REQ-LCG-04 实际在 lc-gateway-multi-provider。anchor 的新 native 身份规则仍由其自身依赖工作流实施，不在这里冒领。 -->

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

规划 provider SHALL 使用四家全角色宿主基线，不再要求 best_effort_configured/production_verified_readonly，不宣称 OS 不可写。author 保留代码编辑岗位纪律和平台文档通道；Codex danger 下按 REQ-ENV-09 检测型 fileChange 纪律验收，不承诺工具不可达。Kimi 规划 Terminal/其余工具与其它 provider 一致，不因 Orchestrator/Splitter/Handoff 整体拒绝；规划 Reviewer 另受 B 档采信约束。

#### Scenario: 规划运行
- **WHEN** 任一家 provider 完成规划 run
- **THEN** 系统 SHALL 如实记录宿主非隔离执行和岗位规则，不要求“成员 checkout 与聚合根物理不可写”或 Codex read-only 证明，不把正常 terminal 能力判为越权失败

#### Scenario: Kimi 规划 Terminal 可执行
- **WHEN** Kimi 的 Orchestrator、WorkItemSplitter 或 Handoff 请求普通 argv 或 shell terminal，并满足当前权限审批
- **THEN** 系统 SHALL 正常执行，不因角色全拒、只读命令表、bwrap 缺失或解释器限制拒绝；若该运行承担 author 职责，编辑代码文件内建工具岗位规则仍适用

#### Scenario: 作者产出与规划身份保持
- **WHEN** author 生成 Story/Design/Plan 或请求编辑代码文件内建工具
- **THEN** 文档 SHALL 走平台产物通道且不由作者提交 Git；具备过滤机制的 provider 不配代码编辑内建工具，Codex 按检测型 fileChange 纪律中断并使该轮失败，不能宣称事前防写；terminal/MCP/skills 不额外禁用，聚合快照、authority 与业务成员归属继续有效
