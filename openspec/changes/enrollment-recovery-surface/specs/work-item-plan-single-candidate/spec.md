# Spec Delta

## ADDED Requirements

### Requirement: existing/create 计划意图与停等（REQ-C1-PLAN-01）

plan contract SHALL 明确每个需要写入的 Work Item 是 `existing` 还是 `create`，并携带对应 provider Work Item 身份以及可校验的 exclusive/forbidden scope 约束。`existing` SHALL 仅引用可解析且授权的既有目标；`create` SHALL 仅在计划明确声明、依赖和作用域均可验证时成立。缺失意图、错拼身份、缺少依赖或越权 create SHALL fail-closed，并向驾驶舱/系统通知说明是“未声明”还是“不能执行”，提供“修订计划／重新反馈”操作；系统 MUST NOT 自动扩大写范围或将 git 基线存在性当作 create 授权。

#### Scenario: 合法 create 通过编译

- **WHEN** plan contract 明确声明 create、provider Work Item、依赖和允许 scope，且校验通过
- **THEN** plan 可进入确定性 compile，create 意图和约束在后续 binding 中可追溯

#### Scenario: 未声明 create 停等修订

- **WHEN** provider 输出涉及不存在路径但 plan 未声明该项为 create
- **THEN** 系统明确提示“未声明”，进入 feedback/修订等待，不自动新增 scope、不批准 compile

#### Scenario: 错拼或越权 create 被拒

- **WHEN** create 的 provider Work Item 身份、依赖、exclusive/forbidden scope 或 target 不匹配
- **THEN** 系统明确提示“不能执行”及冲突事实，拒绝 compile 并提供受限修订操作，计划既有 finding 不被清空
