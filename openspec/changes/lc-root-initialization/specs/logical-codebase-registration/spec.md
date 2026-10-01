# Spec Delta

## MODIFIED Requirements

### Requirement: 规则与 capability 缺失的准入预检（REQ-REG-12）

逻辑代码库首次登记、root recipe、索引或 provider 准入前，系统 SHALL 预检实际 provider 将消费的 canonical LC root 规则材料、聚合 policy 引用、面向后续 Codex/Kimi session 的 provider trust preparation 和 capability/gateway 条件。材料缺失、不一致、过期、信任登记失败或 provider 能力不满足时，系统 SHALL 在依赖该材料的 provider 启动前返回可诊断等待项，通知原因、目标、缺失材料、外部副作用和准备/重试/撤销操作；准备完成后 SHALL 回到原步骤继续。系统 MUST NOT 通过写入一条 capability 记录、复制成员规则、绕过 gateway、改用备用 cwd 或以未登记 trust 将缺失能力视为可用；固定 Claude Code root recipe 不因 Codex/Kimi trust preparation 失败而改用降级 provider。

#### Scenario: 成员规则缺失

- **WHEN** 规则消费前检查发现成员仓原有规则材料缺失，但 canonical LC root 的规则与 policy receipt 有效
- **THEN** 系统 SHALL 使用已核验的根规则材料继续，不将成员规则缺失当作根规则缺失，不复制成员规则；若根规则材料也缺失，则在真实 provider 启动前停等并提供产品化准备/重试操作

#### Scenario: 根规则缺失时进入自举等待或相位

- **WHEN** 普通 provider 准入发现 canonical LC root 缺失、越界、symlink 逃逸或摘要漂移，且当前请求不是有效 bootstrap phase
- **THEN** 系统 SHALL 在真实 provider 启动前停等并提供根 recipe 的准备/重试操作；不读取成员规则作为全局 fallback、不启动 provider

#### Scenario: bootstrap phase 只豁免根规则

- **WHEN** 有效 Running aggregate operation 派生的 bootstrap credential 请求执行 root recipe
- **THEN** 系统 MAY 豁免根规则尚未存在这一项，但 SHALL 继续校验 authority、policy、trust、capability、gateway、canonical cwd、target 与可用性

#### Scenario: provider trust 登记失败

- **WHEN** Codex 或 Kimi 的当前 LC root trust 登记写入失败、记录不匹配或撤销状态异常
- **THEN** 系统 SHALL 生成包含 provider、canonical root、登记动作和错误摘要的 fail-closed 等待项，不以未信任或无 MCP 的降级会话继续；登记记录 SHALL 可重试且不影响其他 LC trust 条目

#### Scenario: gateway 能力不满足

- **WHEN** capability 检查发现当前 provider/gateway 组合不被支持
- **THEN** 系统 SHALL 拒绝准入并通知合法配置/目标选择，不启动 provider、不伪造 capability 成功、不回落到另一套路径

## ADDED Requirements

### Requirement: LC provider 信任登记生命周期（REQ-REG-14）

系统 SHALL 为需要 workspace trust 的后续 LC provider 提供按 canonical `provider_context_root` 绑定的登记与撤销。LC 根准入（REQ-REG-09）确定并冻结 root identity 后、Claude Code recipe 启动前 SHALL 完成/记录所需 trust preparation；该事实不属于五步 recipe、不插入步骤之间，普通 Codex/Kimi LC session 启动前 SHALL 再次核验；不得等根配置生成后才准备。Codex SHALL 使用用户级 projects trust 配置，Kimi SHALL 使用由 basename 与 canonical root SHA-256 前缀确定的 workspace-trust 记录；登记 SHALL 幂等、只影响当前 LC 根、有归属证明时在 LC 删除或解绑撤销本 LC 管理且未被外部修改的条目，不得覆盖或撤销已有的用户自建 trust，并生成不含敏感凭据的审计事实。Codex/Pi/KimiCode 的实际 LC gateway launch 由后续 `lc-gateway-multi-provider` change 交付。

#### Scenario: Codex trust 登记

- **WHEN** LC 根准入已冻结 Codex 所用 canonical root、且尚未启动 Claude Code recipe
- **THEN** 系统 SHALL 在后续 Codex provider turn 前于用户级 Codex projects trust 中登记该 canonical root 为 trusted，并在后续 LC root 启动参数中包含 `--skip-git-repo-check`；登记失败 SHALL 阻止依赖该 trust 的 Codex session，且不得阻断固定 Claude Code recipe；实际 Codex LC gateway launch 由 `lc-gateway-multi-provider` 解锁

- **WHEN** LC 根准入已冻结 Kimi 所用 canonical root、且尚未启动 Claude Code recipe
- **THEN** 系统 SHALL 在后续 Kimi provider turn 前写入 `wd_<basename>_<sha256(canonical_root)[:12]>` 对应的 workspace-trust 记录，记录 root 与登记时间，并在后续启动前复验记录与 canonical root 一致；登记失败 SHALL 阻止依赖该 trust 的 Kimi session，且不得阻断固定 Claude Code recipe；实际 Kimi LC gateway launch 由 `lc-gateway-multi-provider` 解锁

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
