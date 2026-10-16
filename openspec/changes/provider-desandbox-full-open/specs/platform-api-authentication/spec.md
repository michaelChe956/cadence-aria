# 规范增量：平台 API 入站鉴权

## Purpose

为同 UID 宿主全开运行前的平台控制面建立统一入站身份与浏览器来源校验，覆盖状态变更、WebSocket 和敏感读取；该边界用于阻止未授权客户端与跨站偷连，不冒充同 UID 恶意进程或凭据读取的物理隔离。

## ADDED Requirements

### Requirement: 平台鉴权是全开运行的无条件前置（REQ-API-AUTH-01）

平台 SHALL 在发布四家全角色宿主全开能力前完成入站认证与 Origin/Host 校验；本地回环、单用户、未暴露外网及未触发风险清单均不得豁免。未授权调用 SHALL 在产生操作或敏感数据前拒绝，不依赖 CORS、隐藏按钮、prompt 或二期容器提供鉴权。

#### Scenario: 无令牌 REST 状态变更
- **WHEN** 未认证客户端调用任一状态变更路由及其 alias
- **THEN** 系统 SHALL 返回 401/403，不创建/修改/删除对象、不触发 provider 或补偿

#### Scenario: 敏感 GET 一样认证
- **WHEN** 未认证客户端读取凭据/配置、政策正文、运行日志或其它敏感 GET
- **THEN** 系统 SHALL 在返回敏感数据前拒绝；路由别名不构成绕过

#### Scenario: 全开部署前置不等触发
- **WHEN** 准备启用四家宿主执行但认证或来源/Host 保护未生效
- **THEN** 发布验收 SHALL 失败，不因 localhost、单人自用或“二期再补”而放行

### Requirement: WebSocket 与浏览器来源校验（REQ-API-AUTH-02）

WebSocket upgrade 与后续控制面 SHALL 绑定已认证身份，REST/WS SHALL 校验服务端允许的 Host 与浏览器 Origin。身份有效但来源/Host 非法仍须拒绝；非浏览器调用缺 Origin 时不等于免认证。系统 SHALL 不在日志、URL 或 provider 环境中泄漏平台入站令牌。

#### Scenario: 浏览器 drive-by 偷连
- **WHEN** 不受信网页向本地平台发起 WS upgrade 或控制请求
- **THEN** 系统 SHALL 在 upgrade/操作前拒绝非法 Origin，即使目标为回环地址，不能仅以 CORS 代替

#### Scenario: DNS rebinding 与伪造 Host
- **WHEN** 请求携带非允许 Host，或认证通过但 Origin 不受信
- **THEN** 系统 SHALL 返回拒绝，不升级 WS、不转交真实控制操作

#### Scenario: 合法客户端与同 UID 边界
- **WHEN** 合法前端完成认证且 Origin/Host 符合配置
- **THEN** 系统 SHALL 允许其原有控制能力，敏感认证材料不进入 provider 运行环境或明文日志；同 UID 仍可读取 owner 可读凭据的残余风险明确记录，不声明该鉴权抵御同 UID 取证调用
