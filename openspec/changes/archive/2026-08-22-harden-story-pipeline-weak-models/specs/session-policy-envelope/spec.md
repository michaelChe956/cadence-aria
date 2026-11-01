## ADDED Requirements

### Requirement: kimi ACP mcpServers 走 envelope 受控注入（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

逻辑代码库流程的 kimi provider 真实会话（`session/new` 与 `session/load`）的 `mcpServers` 必须来自 Aria-owned、权限受控的 settings/MCP bundle：allowlist 校验、配置 digest 记录、凭据脱敏、argv 审计；resume 时 digest 不一致必须拒绝。此为既有"MCP bundle 受控"要求在 kimi ACP 通道上的扩展（原要求覆盖 provider settings 合并，本 delta 增加 ACP `mcpServers` 注入路径与 resume 一致性校验）。

#### Scenario: kimi 注入受控 bundle
- **WHEN** kimi 真实会话初始化且 envelope 提供 bundle
- **THEN** `mcpServers` 由 bundle 派生，argv 与 digest 记入 run 审计；无 bundle 时为空数组

#### Scenario: resume digest 漂移拒绝
- **WHEN** `session/load` 时 bundle digest 与冻结值不一致
- **THEN** 拒绝加载并报告差异，启动新会话且旧会话标记 superseded（对齐 REQ-ENV-04）
