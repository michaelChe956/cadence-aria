# platform-api-authentication Specification

## Purpose
为同 UID 宿主全开运行前的平台控制面建立入站来源校验（Host/Origin），覆盖状态变更、WebSocket 和敏感读取；该边界用于阻止跨站偷连与 DNS rebinding，不要求入站令牌或会话，不冒充同 UID 恶意进程或凭据读取的物理隔离。

## Requirements

### Requirement: 平台以 Host/Origin 来源校验作为控制面边界（REQ-API-AUTH-01）

平台 SHALL 以服务端 Host 校验与浏览器 Origin 校验作为控制面入站边界，SHALL NOT 要求入站令牌或会话。请求 SHALL 携带允许的 Host；状态变更与 WebSocket upgrade SHALL 携带受信精确 Origin，非法/`null` Origin 与伪造/缺失 Host SHALL 拒绝；Host 受信时只读 GET/HEAD/OPTIONS 无 Origin SHALL 放行到下一层。

#### Scenario: 伪造 Host 的请求
- **WHEN** 请求携带非允许 Host（含缺失、畸形或与 URI authority 冲突）
- **THEN** 系统 SHALL 返回 403，不转交任何 handler、不升级 WS

#### Scenario: 非法 Origin 的状态变更与 WS
- **WHEN** 状态变更路由（含 alias）或 WS upgrade 携带非法/`null` Origin 或缺失 Origin
- **THEN** 系统 SHALL 返回 403，不创建/修改/删除对象、不触发 provider 或补偿

#### Scenario: 无会话的合法来源调用
- **WHEN** 受信 Host+Origin 的客户端不带任何令牌或会话 cookie 调用 REST/WS
- **THEN** 系统 SHALL 放行到 handler，不因缺少登录会话拒绝

#### Scenario: 来源/Host 校验是全开运行的前置
- **WHEN** 准备启用四家宿主执行但 Host/Origin 来源校验未生效
- **THEN** 发布验收 SHALL 失败，不因 localhost、单人自用或“二期再补”而放行

### Requirement: WebSocket 与浏览器来源校验（REQ-API-AUTH-02）

WebSocket upgrade 与 REST SHALL 校验服务端允许的 Host 与浏览器 Origin；受信来源下不要求已认证身份。非浏览器调用缺 Origin 时 SHALL 按方法区分处置：只读 GET/HEAD/OPTIONS 放行，状态变更与 WS upgrade 拒绝。系统 SHALL 不在日志、URL 或 provider 环境中泄漏入站令牌材料（`/api/evidence-query` 的专用 evidence token 语义不变，由 handler 专用链鉴权）。

#### Scenario: 浏览器 drive-by 偷连
- **WHEN** 不受信网页向本地平台发起 WS upgrade 或控制请求
- **THEN** 系统 SHALL 在 upgrade/操作前拒绝非法 Origin，即使目标为回环地址，不能仅以 CORS 代替

#### Scenario: DNS rebinding 与伪造 Host
- **WHEN** 请求携带非允许 Host
- **THEN** 系统 SHALL 返回拒绝，不升级 WS、不转交真实控制操作

#### Scenario: 合法客户端与同 UID 边界
- **WHEN** 合法前端以受信 Origin/Host 访问
- **THEN** 系统 SHALL 允许其原有控制能力（无需登录或令牌粘贴）；本机同 UID 程序可伪造 Host/Origin 的暴露面为已明确接受的残余风险，不声明该边界抵御同 UID 取证调用
