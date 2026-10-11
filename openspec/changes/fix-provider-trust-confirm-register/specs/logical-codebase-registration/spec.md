# Spec Delta

> 基线：本 capability 已存在；本 delta 仅 **ADDED REQ-REG-15**。REQ-REG-14「LC provider 信任登记生命周期」位于尚未归档的 `openspec/changes/lc-root-initialization/specs/logical-codebase-registration/spec.md:36`；本 requirement 复用该生命周期，不修改主规格不存在的 REQ-REG-14。
>
> 冻结来源：`lc-gateway-multi-provider` delta 的「LC admission 与 spawn 前复验」「可验证的 provider policy projection」及主规格 REQ-CFC-06。REQ-LCG-02/03 是源 change tasks §1.1/§1.3 的映射编号，不是此 delta 的 Requirement 编号。

## ADDED Requirements

### Requirement: 人工确认链按需登记（REQ-REG-15）

系统 SHALL 在合法 LC 人工确认的真正关门/定稿之前，按权威会话的有效 provider 与冻结 canonical root 复用 REQ-REG-14 生命周期完成按需 trust 登记；覆盖 WS 批量、live driver、同形单会话确认及 REST human action approve。未 Ready SHALL 保持确认门、返回可诊断等待与场景化人工指导；原仲裁、CAS、唯一成功广播与 gateway 只读复验 MUST 保持。方案 A MUST NOT 在首次生成自动登记，但首次生成因缺 trust 停等时 SHALL 展示同源手工登记指引。

#### Scenario: 开态门确认缺失条目自动补齐

- **WHEN** 已抵达合法开态人工门的 LC 会话由有权用户确认，其有效 author 或 reviewer 为 Codex/Kimi，冻结 canonical root 的 trust 条目缺失
- **THEN** 系统 SHALL 在该门关闭、定稿或创建子会话前登记本会话所需 trust，全部 Ready 后才沿原确认裁决继续；成功 SHALL 产生 `ensure/registered` 审计并关联 `workspace_confirm:<session_id>`、project、LC、provider、root、key，不生成不存在的 `gate_registered`

#### Scenario: 公共 WS 与 REST approve 使用相同等待合同

- **WHEN** 用户经收件箱短连接 runner、当前 live driver、同形单会话 WS confirm 或 REST human action approve 确认同一类合法人工门
- **THEN** 系统 SHALL 共用产品层准备与关门裁决；trust 未 Ready 时 WS SHALL 返回既有 protocol error，REST SHALL 返回非成功 ApiError，稳定 code 均为 `workspace_confirmation_trust_waiting`，不得落入 compile failure、generic engine error 或 accepted 成功；不增加 bulk 来源字段、第二次 confirm 或独立登记 endpoint

#### Scenario: 不合法确认在登记前拒收

- **WHEN** 确认违反 driver/observer、stale lease、stage/flow 白名单、durable WaitingForHuman、Busy、候选完整性或批准资格；其中 SC 的 WaitingForHuman 记录为 phase=None/Prepare/Generate/Failed，或 snapshot=None
- **THEN** 系统 SHALL 在任何 ensure/home/ownership/trust audit 调用之前沿既有拒绝、Busy、AlreadyClosed 或 Conflict 翻译返回；SC 合法批准仍仅接受 phase=Approval/Evaluate 且 snapshot 非空；Abandon 的既有脱困能力、反馈与 Running 普通 confirm SHALL 不变且不触发登记

#### Scenario: 有效 provider 集与免登记会话

- **WHEN** 合法确认读取本会话已锁定的 author/effective reviewer，包含同 provider、禁用 reviewer、provisional reviewer、Claude/Pi/Fake，或无 LC gateway 的单仓会话
- **THEN** 系统 SHALL 仅为有效且需要 trust 的 Codex/Kimi 去重登记，不采信建联陈旧快照、前端 root/provider、默认配置或安装列表；禁用/provisional reviewer 与无需 trust/非 LC 会话 SHALL 零 home/ownership/trust audit 调用，不改变 recipe 的 Claude-only 选择

#### Scenario: 冻结身份缺失或漂移等待

- **WHEN** LC 会话有 gateway 却缺确认登记上下文/precondition，issue 原始 LC 归属已改变，冻结 root 不能 canonicalize，或同 scope manifest root 与 gateway 冻结 root 不一致
- **THEN** 系统 SHALL 以相应 `confirmation_trust_context_unavailable`、`confirmation_trust_scope_changed`、`confirmation_trust_root_unavailable`、`confirmation_trust_root_changed` reason 停等，零 home 调用且门不变；只返回已知身份，不编造 provider/root/key、不向错误根提供手工 trust 指引，不降级到另一 LC、成员 checkout 或原路径；历史 LC SHALL 沿既有 legacy scope 约定解析

#### Scenario: 已可信条目复用与归属隔离

- **WHEN** 同 provider/root 已可信，包括本 LC 已管理条目、用户自建条目或其它 LC 创建的条目
- **THEN** 每次合法确认 SHALL 仍进入既有 ensure 归属/审计流程，但 adapter home 写次数 SHALL 为零；本 LC replay SHALL 保持 owned=true 并记 `ensure/replay`，首次复用用户或其它 LC 条目 SHALL 保持字节不变、owned=false 并记 `ensure/reuse_user_entry`，后续撤销 SHALL 为 NotOwned；不得跨 LC 合并 durable 事实

#### Scenario: Aria 协作写方并发保持条目与归属

- **WHEN** 遵守同一 home 工件锁约定的 Aria 写方并发操作同 root 同 LC、同 root 不同 LC、不同 root 的共享 Codex config，或交错 ensure/revoke
- **THEN** 系统 SHALL 串行化从读取到 home 与 ownership/clear-waiting/audit 提交的完整生命周期，保留 digest CAS 与 atomic rename；不得丢失其他 root、把本 LC owned=true 降为 false、认领其它 LC 归属或误撤销用户条目；平台不支持锁或锁获取失败 SHALL fail-closed 等待，不宣称任意外部非协作编辑受此锁保护

#### Scenario: 失败 waiting 不覆盖较新成功当前态

- **WHEN** 一个同 scope/provider/key 的失败尝试与另一成功尝试受控交错，失败的 waiting/audit 提交可能晚于成功清理
- **THEN** 当前 waiting 的写入 SHALL 与 ensure 全体排序，旧失败 MUST NOT 在较新 Ready 之后重新留下陈旧当前 waiting；失败尝试审计可保留，审计历史 MUST NOT 被当作当前等待状态；等待事实/审计 store 自身失败时仍 SHALL 返回 fail-closed 等待，不伪称所有失败均落盘

#### Scenario: 取消锁等待与确认结果未知

- **WHEN** 用户确认仍在等待 home 锁时取消，已开始 trust 提交时取消/断连，或客户端既有 8 秒超时先发生
- **THEN** 待锁取消 SHALL 不留下无界阻塞 waiter、也不在锁释放后复活业务；已开始提交 SHALL 安全收尾且不据取消误删条目，不释放确认互斥后去关闭另一扇门；超时 SHALL 保持结果未知、无自动重发，用户先读取权威门态再显式操作；home 锁 SHALL 不跨越 compile、网络或 provider run

#### Scenario: 确认登记失败给出真实指导且门不变

- **WHEN** home 读取/写入/格式/冲突/CAS 失败，或 ownership/审计等 durable fact IO 失败导致任何所需 provider 未 Ready
- **THEN** 系统 SHALL 保持原 workspace status/phase/snapshot、门节点、预算、turn、compile reservation 与 provider-start ledger，不 compile、不创建子会话、不启动 provider、不发 close、不 abort；响应 SHALL 保留原始 reason_code，包含已知 session/project/LC/provider/canonical_root/trust_key、`retryable:true`、场景正确的 `retry_action` 与 `manual_guidance`，中文 message SHALL 展示原因、门未关闭、处置及「完成后重新选择该门并确认；失败不自动重试」，不得提示确认用户「重试初始化」；retryable 表示修复后可显式确认，不表示冲突可立即安全重试

#### Scenario: provider 特定人工指导安全展示

- **WHEN** 确认或 generation 等待面需要展示 Codex/Kimi 的人工处置
- **THEN** 指导 SHALL 来自实际服务端 adapter 的同源路径/root/key：Codex 为 `.codex/config.toml` 中正确转义的目标 projects 表与 `trust_level = "trusted"`，提示备份及仅改目标表；Kimi 为实际 `.kimi-code/workspace-trust/<trust_key>` 及正确 JSON root/trustedAt；冲突须人工核验，权限/格式错误先修权限/格式，fact IO 须修 durable store，缺 adapter 只提示装配修复；不得返回完整 home 配置/凭据、伪造路径、直接执行 shell 或覆盖冲突值

#### Scenario: 部分提交不构成跨介质事务

- **WHEN** 一个 provider 已登记而另一 provider 失败，或 home 写成功后 ownership/audit 写失败或进程中断
- **THEN** 本次确认 SHALL 保持门开，已成功的 home/trust 事实 SHALL 保留、不全批回滚、不自动撤销；下次显式确认 SHALL replay/reuse，缺可读归属时保守 user-owned，不猜测认领；系统 SHALL 如实告知可能部分副作用，不承诺 home 零变化、审计必落盘或 home/facts 原子事务

#### Scenario: 准备成功后仍使用原关门竞争判据

- **WHEN** trust Ready 期间另一 worker 已改变加载的 expected workspace record 或先关闭门
- **THEN** 原整记录 CAS 与迟到翻译 SHALL 继续裁决，不重读替换 expected 规避竞争；败者 SHALL 不 compile、不创建子会话、不发第二次 close；至多一个胜者在原 compile/durable Confirmed 核验后关闭该门；可能完成的幂等 trust 登记不等于确认成功

#### Scenario: 批量入口与可见性保持 REQ-CFC-06

- **WHEN** 收件箱选择两个真实可选 human_gate 执行跨会话批量确认，包含当前 live driver 会话及远端短连接会话，某项 trust 等待
- **THEN** 每会话 SHALL 去重、沿既有 driver 仲裁恰一次发送并逐项报告，成功 close SHALL 通过原 journal/session 广播对其它连接可见；waiting 项 SHALL rejected 并展示 message/retry_action/manual_guidance，操作审计 SHALL 只存稳定 code，其它 protocol error detail 行为不变；并发上限 3、8 秒超时、无自动重试 SHALL 不变；AuthorConfirm、batch_confirm、compile_recovery、危险动作、关闭/blocked/无合法 session 身份的门 SHALL 不被新增为可批量选择项

#### Scenario: GET 与 gateway 复验仍零 trust 副作用

- **WHEN** readonly verify/build、GET/eligibility、early admission 或 spawn 前复验发现 missing、conflict，或发现此前登记后被用户删除的条目
- **THEN** 系统 SHALL 只读拒收，前后 home 与 durable trust ownership/waiting/audit 快照 SHALL 相等，不自动补条目、不伪造 trust/capability 证据；generation 等待补指导也 SHALL 使用只读同源指导能力，不调用会追加审计的 registry verify 或 ensure

#### Scenario: 新根首次生成带指导但不自动登记

- **WHEN** 全新 canonical LC root 无 Codex/Kimi trust，用户显式首次生成且其它既有材料/能力门满足，generation 在确认门出现前因缺 trust 被拒收
- **THEN** 系统 SHALL 仍 fail-closed、provider 零启动且 home/ownership/trust audit 不变，将同源 `manual_guidance`、原始原因、真实目标与「人工登记后显式重新生成」的 retry_action 带入既有 generation waiting/Prepare 回落面；不得只有无指导的 ProviderUnavailable 裸错，不新增 generation 自动登记或保证能自动抵达确认门

#### Scenario: owner 删除条目后重开确认自动恢复

- **WHEN** controller 在授权测试实例记录真实 project/LC/provider/root，备份移走目标 trust 条目，重开已经能抵达合法待确认门的会话并从原收件箱显式确认
- **THEN** 系统 SHALL 自动补同根条目，Kimi key/JSON 或 Codex 目标表正确，`ensure/registered` 审计可关联本次会话，原门恰一次关闭且其它观察连接同步可见；该验收 SHALL 不靠手工预写条目、不删除无关 home 内容，且与「新根首次生成带指导」场景分别留证，不伪称首次生成自动可用
