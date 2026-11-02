# Spec Delta

## MODIFIED Requirements

### Requirement: 规则与 capability 缺失的准入预检（REQ-REG-12）

逻辑代码库首次登记、root recipe、索引或 provider 准入前，系统 SHALL 预检实际 provider 将消费的 canonical LC root 规则材料、聚合 policy 引用、Codex/Kimi provider trust 前置条件和 capability/gateway 条件。成员规则门 SHALL 按普通根消费者、有效 AggregateBootstrap 与 LC SingleCandidate（SC）的实际消费来源区分。必需材料缺失、不一致、过期、信任登记失败或 provider 能力不满足时，系统 SHALL 在依赖该材料的真实 provider 启动前返回可诊断等待项，通知原因、目标、材料、外部副作用和准备/重试/撤销操作；准备完成后 SHALL 回到原步骤继续。系统 MUST NOT 通过写入 capability 记录、复制或自动生成成员规则、绕过 gateway、改用备用 cwd 或以未登记 trust 将缺失能力视为可用。

#### Scenario: 成员规则缺失

- **WHEN** 普通请求只消费 canonical LC root 的材料，根规则、已发布政策与最终 policy receipt 经既有准入校验有效，而全部或部分成员仓的 `.claude/rules/language.md` 缺失
- **THEN** 系统 SHALL 使用已核验的根材料继续，不将成员缺失当作根缺失；`missing_member_rules` SHALL 继续作为非阻断审计事实记录，不读取不消费的成员文件正文、不复制或生成成员规则；若只有这些非必需成员材料缺失，准入 SHALL 为 ready，`allowed_actions` SHALL 为空，不提示准备无关成员

#### Scenario: 非阻断成员缺失不掩盖真实阻断项

- **WHEN** 不消费的成员规则缺失同时伴随成员无 recorded checkout、manifest/身份异常或其他既有准入门失败
- **THEN** 系统 SHALL 保留相应真实阻断材料和既有拒绝，不清空整个 `missing_materials`，不以成员规则豁免授权 provider 启动；缺规则引用的空 digest SHALL 仅保留审计语义，不作为正文已存在或已核验的证明

#### Scenario: 根规则缺失时进入自举等待或相位

- **WHEN** 普通 provider 准入在既有真实已发布政策路径发现 canonical LC root 规则缺失、越界、symlink 逃逸或既有校验所覆盖的摘要漂移，且请求没有有效 bootstrap credential
- **THEN** 系统 SHALL 保持现有 fail-closed 拒绝与准备/重试或重新核验动作，不读取成员规则作为全局 fallback、不启动 provider；预存的 Normal 加自举桩政策的根缺失放行边界 SHALL 不变且不扩大，placeholder MUST NOT 被当作本次「根材料有效」放行的证明

#### Scenario: 普通请求的发布链失效不可借成员豁免放行

- **WHEN** 普通根消费者在既有发布链校验中遇到最终 receipt 缺失、损坏、policy locator 异常、AGENTS digest 或 root 身份漂移
- **THEN** 系统 SHALL 保留既有 fail-closed 拒绝和错误投影，不因成员规则非阻断而继续，不把错误统一伪装成已具备恢复 UI 的等待项；本次成员豁免 SHALL 不扩建 receipt TTL、全源摘要或五阶段历史校验范围

#### Scenario: 有效 bootstrap credential 解除裸仓首建死锁

- **WHEN** AggregateBootstrap 请求持有由 durable Running aggregate operation 派生且重验有效的 bootstrap credential，执行合法 root recipe，而 canonical LC root 规则、最终 receipt 和全部成员规则尚未生成
- **THEN** 系统 SHALL 通过既有自举通道豁免不消费的成员规则门，并按既有 bootstrap 边界容许根规则与最终 receipt 尚不存在；authority、policy、trust、capability、gateway、canonical cwd、target 与可用性 SHALL 继续核验，合法 recipe 不先被普通会话的 Normal capability 矩阵拒绝，不伪造 Confirmed、不写成员规则

#### Scenario: bootstrap 凭据缺失或漂移拒绝

- **WHEN** bootstrap credential 缺失、durable operation 不再为 Running，或 LC、canonical root、step、input digest 等凭据绑定事实漂移
- **THEN** 系统 SHALL 在真实 provider 启动前拒绝使用该凭据及其成员规则豁免，保留可诊断的既有凭据失败路径，不借旧 recipe 证据启动、不切换备用 cwd

#### Scenario: 普通会话不得复用 root recipe 能力证据

- **WHEN** 普通会话的 Normal capability 不满足，但存在某次有效 root recipe 的能力证据或 bootstrap credential
- **THEN** 系统 SHALL 仍按普通会话的 capability/gateway 条件拒绝，不把 recipe 证据升级为普通会话 Confirmed，不借成员规则豁免绕过 capability 门

#### Scenario: LC SC 仅要求实际聚焦成员的消费材料

- **WHEN** LC SC 已解析的 `repository.path` 指向一个聚焦成员，该路径的 `.claude/rules/language.md` 可读取为 UTF-8，而其他 active 成员的同名规则缺失
- **THEN** 系统 SHALL 仅以该实际消费路径判定 SC 必需规则，成功读取的同一份正文 SHALL 用于 prompt；其他成员缺失 SHALL 不阻断、不产生成员准备动作，不枚举全部 active 成员作为 SC 文件门，不撤销根/policy/trust/capability 等其他既有准入条件

#### Scenario: LC SC 实际消费文件缺失或不可读

- **WHEN** LC SC 对实际解析的 `repository.path` 下规则文件的消费前读取返回 NotFound、权限错误、目录错误或非法 UTF-8
- **THEN** 系统 SHALL 在真实 provider 启动前以 `member_language_rules_missing` 语义可诊断停等，detail SHALL 区分实际读取错误并指明来源；Prepare/Retry、durable Prepare/Open 和 provider 零启动 SHALL 保持，不把首次告知落为终态 Failed、不回退为规则指针；材料恢复后 SHALL 在原会话重新按同一来源读取并继续

#### Scenario: 多 target 且无唯一 focus 的 SC 消费根规则

- **WHEN** LC SC 存在多个 target 且没有唯一 focus，既有 repository 解析结果为 aggregate root view
- **THEN** 系统 SHALL 以该 `repository.path` 指向的聚合根规则文件作为 SC 必需正文；根文件可读且其他既有门通过时 SHALL 继续，不因任意成员规则缺失停等；根文件读取失败 SHALL 适用 SC 实际消费文件的 waiting 合同

#### Scenario: SC 预检与消费使用同一已解析 checkout

- **WHEN** identity registry 的 primary checkout 与 active 成员列表中首个 Main 或其他 checkout 不同
- **THEN** 系统 SHALL 使用既有身份解析确定的 `repository.path` 完成 SC 消费前检查并复用其成功读取的正文，不另选 Main/任意 checkout 预检另一份材料，不从其他 worktree 兜底找文件

#### Scenario: 既有 SC 与非 LC 行为不变

- **WHEN** SC 通过规则门进入生成或 conversational-gate revision，或非 LC 会话读取自身规则
- **THEN** 系统 SHALL 保持既有 SC 缺必需正文拒绝启动、revision 注入及 durable flow 分流合同，不新增伪装 phase；非 LC 规则读取与失败行为 SHALL 不变

#### Scenario: provider trust 登记失败

- **WHEN** Codex 或 Kimi 的当前 LC root trust 登记写入失败、记录不匹配或撤销状态异常
- **THEN** 系统 SHALL 生成包含 provider、canonical root、登记动作和错误摘要的 fail-closed 等待项，不以未信任或无 MCP 的降级会话继续；登记记录 SHALL 可重试且不影响其他 LC trust 条目

#### Scenario: gateway 能力不满足

- **WHEN** capability 检查发现当前 provider/gateway 组合不被支持
- **THEN** 系统 SHALL 拒绝准入并通知合法配置/目标选择，不启动 provider、不伪造 capability 成功、不回落到另一套路径

#### Scenario: owner 四个裸成员仓启动聚合初始化

- **WHEN** `busi`、`api`、`gateway`、`frontend` 四个成员仓均为无任何规则文件的骨架仓，根材料尚未生成，owner 点击「启动聚合初始化」且现有身份、信任、自举凭据和 recipe 能力条件满足
- **THEN** 系统 SHALL 不再因 `member_language_rules_missing` 阻断，合法 root recipe SHALL 生成根材料并使五阶段后续可以继续；成员文件树与 Git 状态 SHALL 保持无规则注入，不以手工补成员规则完成验收


## ADDED Requirements

### Requirement: LC provider 信任登记生命周期（REQ-REG-14）

系统 SHALL 为需要 workspace trust 的后续 LC provider 提供按 canonical `provider_context_root` 绑定的登记与撤销。LC 根准入（REQ-REG-09）确定并冻结 root identity 后、Claude Code recipe 启动前 SHALL 完成所需 trust 的独立硬前置登记；该登记不属于五步 recipe、不插入步骤之间，普通 Codex/Kimi LC session 启动前 SHALL 再次核验；任一所需 trust 未 Ready 时五步 recipe 不创建/不启动，初始化进入可重试 fail-closed waiting。Codex SHALL 使用用户级 projects trust 配置，Kimi SHALL 使用由 basename 与 canonical root SHA-256 前缀确定的 workspace-trust 记录；登记 SHALL 幂等、只影响当前 LC 根、有归属证明时在 LC 删除或解绑撤销本 LC 管理且未被外部修改的条目，不得覆盖或撤销已有的用户自建 trust，并生成不含敏感凭据的审计事实。Codex/Pi/KimiCode 的实际 LC gateway launch 由后续 `lc-gateway-multi-provider` change 交付。

#### Scenario: Codex trust 登记

- **WHEN** LC 根准入已冻结 Codex 所用 canonical root、且尚未启动 Claude Code recipe
- **THEN** 系统 SHALL 在用户级 Codex projects trust 中登记该 canonical root 为 trusted，并配置后续 LC root 启动参数包含 `--skip-git-repo-check`；登记失败 SHALL 阻止五步 recipe 创建/启动，生成可重试 fail-closed waiting；实际 Codex LC gateway launch 由 `lc-gateway-multi-provider` 解锁

- **WHEN** LC 根准入已冻结 Kimi 所用 canonical root、且尚未启动 Claude Code recipe
- **THEN** 系统 SHALL 写入 `wd_<basename>_<sha256(canonical_root)[:12]>` 对应 workspace-trust 记录，并在后续启动前复验 root 一致；登记失败 SHALL 阻止五步 recipe 创建/启动，生成可重试 fail-closed waiting；实际 Kimi LC gateway launch 由 `lc-gateway-multi-provider` 解锁

#### Scenario: 重试与多 LC 隔离

- **WHEN** 同一 LC 重复执行登记，或另一 LC 同时执行登记
- **THEN** 同一 canonical root 的登记 SHALL 幂等，其他 root 的记录 SHALL 保持不变；每次结果均可由 operation/lc/provider/root/key 审计关联

#### Scenario: LC 删除或解绑撤销

- **WHEN** LC 被删除或 provider trust 解绑操作确认执行
- **THEN** 系统 SHALL 仅撤销该 LC 管理且其 canonical root 对应的 Codex/Kimi trust 记录，并保留撤销前后摘要与结果；不得删除其他 root 或用户原有的信任登记

#### Scenario: 既有用户 trust 与并发修改

- **WHEN** 当前 root 已有用户自行登记的 trust、同 key 已有不可信值，或 LC 管理的 trust 在撤销前被外部修改
- **THEN** 已可信的用户自建条目 SHALL 原样复用并记为非本 LC 归属，未受信值或撤销时摘要不匹配 SHALL 进入等待/核验，不得覆盖或删除用户值；其他 LC 的登记 SHALL 保持不变

#### Scenario: 跨工作区写入失败

- **WHEN** 用户 home 下的 trust 配置不可写、格式冲突或撤销无法核验
- **THEN** 系统 SHALL fail-closed 停等并展示外部副作用说明；不得启动依赖该登记的 provider，也不得覆盖无关配置
