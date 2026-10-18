# Spec Delta: fix-planning-session-anchor-recovery × session-policy-envelope

## MODIFIED Requirements

### Requirement: resume 一致性（REQ-ENV-04）

系统 SHALL 仅当 policy digest、target/git identity、canonical working directory、provider exact version/dialect、capability snapshot与适用的完整LC projection/trust/tool/MCP冻结事实一致时允许LC resume；任一失配或冻结材料不可验证 SHALL 拒绝旧native、标记superseded并等待明确新会话，只有用户显式fresh后重新完整admission。Planning/Coding/Review/initialization继续原action门，direct单仓语义不变。

#### Scenario: 续接旧 provider session
- **WHEN** 尝试续接LC旧provider session且任一冻结指纹不一致
- **THEN** 系统 SHALL 拒绝旧native并保存superseded waiting，不能静默fresh；fingerprint判定本身无provider会话进程，显式fresh后才重新validate/revalidate

#### Scenario: cwd 或 target 改变时拒绝 resume
- **WHEN** LC canonical root cwd、native target/git identity、policy或capability变化
- **THEN** 系统 SHALL 拒绝旧会话且零adapter session-start/协议child，不以cwd/member fallback放行；PlanningReadOnly的native target按REQ-PLN-08固定aggregate-root，业务材料target独立保持

#### Scenario: 版本观测与会话 spawn 口径
- **WHEN** LC candidate准备或spawn前复验需要当前exact version
- **THEN** 系统 SHALL 允许有界只读CLI --version及60秒非滑动TTL、同键single-flight；版本probe与adapter start/session child SHALL 分别计数，有效缓存age小于60秒、到期再观测，失败/超时/取消不缓存且不使用过期成功，不缓存其它admission事实

#### Scenario: 原生握手后失败如实收口
- **WHEN** 已创建会话child后发现原生恢复ID缺失/错ID或runtime协议失败
- **THEN** 系统 SHALL kill/reap并记录未恢复，不冒称零child或伪回填请求ID；不得自动清旧ID转fresh

### Requirement: kimi ACP mcpServers 走 envelope 受控注入（REQ-ENV-08）

系统 SHALL 使LC kimi的session/new与session/load受控注入mcpServers来自Aria-owned、权限受控settings/MCP bundle，执行allowlist、digest、脱敏和审计；无注入bundle时为空数组。Aria注入bundle的resume digest失配 SHALL 拒旧并等待显式fresh。Kimi从已信任canonical LC root原生自发现的项目MCP配置属REQ-ENV-06受信任例外，不归Aria bundle管控；trust登记与注入bundle为独立事实/审计。

#### Scenario: kimi 注入受控 bundle
- **WHEN** kimi真实初始化且envelope提供Aria bundle
- **THEN** mcpServers SHALL 从该bundle派生，argv/digest按既有run审计；无注入bundle为空数组，不把native发现配置复制成Aria bundle

#### Scenario: kimi 自发现不受控
- **WHEN** kimi原生机制加载已信任canonical LC root项目级MCP配置
- **THEN** 该通道 SHALL NOT 受本requirement的bundle管控或拒绝；root-specific trust登记按未归档REQ-REG-14独立审计

#### Scenario: resume digest 漂移拒绝
- **WHEN** session/load的Aria注入bundle digest与冻结值不一致
- **THEN** 系统 SHALL 拒绝加载、报告差异并保存旧native superseded waiting，不能静默session/new；用户明确新会话后才完整fresh admission

#### Scenario: Kimi 原生 root MCP 自发现
- **WHEN** Kimi CLI已登记canonical LC root trust并从根项目配置原生发现MCP
- **THEN** 系统 SHALL 保留该native通道，不要求复制为Aria bundle、不误判受控注入；native-source标记与Aria digest保持区分

#### Scenario: Kimi Aria bundle 保持受控
- **WHEN** Kimi初始化或resume且envelope提供Aria bundle
- **THEN** allowlist/digest/脱敏/审计 SHALL 原样；注入digest漂移拒旧、superseded且显式fresh等待，不能借原生自发现例外免检Aria注入
