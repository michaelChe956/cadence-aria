# Proposal: fix-provider-trust-confirm-register

## Why

owner 在新聚合根的 Codex/Kimi 会话中确认开态门时，gateway 因 `entry absent` 拒收；手工登记只能恢复特定根，不能修复人工确认链。既有 registry 已有登记、归属和审计能力，但公共确认写链没有按本次会话的有效 provider 与冻结 LC root 调用它。

## What Changes

- 持久化已批准方案 **A：仅确认链自动登记**。人工确认动作即本会话有效 provider 的按需登记授权，覆盖收件箱短连接批量 runner、当前会话 live driver、同形单会话 WS confirm，以及共享引擎裁决的 REST human action approve；不新增 bulk 来源字段或整批 endpoint。
- 在原确认合法性校验之后、真正关门 CAS/定稿/创建子会话之前，按服务端权威会话的 author 与有效 reviewer 过滤、去重 Codex/Kimi，为冻结 canonical `provider_context_root` 复用既有 registry。SC 额外无副作用预检 approve 的静态 CAS 前置；原 CAS 与迟到/Busy/候选完整性裁决不变。
- 复用生产依赖并 clean-cutover 中立化 recipe 专属 preparation 接口，迁移全部调用方。共享 home sidecar 锁覆盖 ensure/revoke 全体及失败 waiting 的排序，保留 CAS、ownership 与真实审计 schema。
- trust 不 Ready 时保持门等待，用稳定码 `workspace_confirmation_trust_waiting` 返回 typed waiting、原始 reason、真实身份与场景化重试/手工处置指导；WS 与 REST 都不得伪成功。批量结果最小透传指导，保留逐项 rejected、恰一次发送、无自动重试及既有仲裁/广播。
- **A 的附带要求：新根首次生成缺 trust 时也带指导。** 仅将同源 registry/adapter 的 `manual_guidance` 带入既有 generation waiting/Prepare 回落面；不在 `start_generation`、early admission 或 spawn 自动登记，不承诺新根首次生成自动可用。
- 补实只读冻结与受控并发测试，覆盖无效确认零副作用、Trusted ownership 不降级、旧 waiting 不覆盖较新 Ready、REST 接线与 owner 的「删条目→重开会话确认→自动补、审计、放行」场景。

## Capabilities

### New Capabilities

无；复用现有 capability。

### Modified Capabilities

- `logical-codebase-registration`：以 **ADDED** Requirement「人工确认链按需登记（REQ-REG-15）」增加公共确认授权、登记等待、安全配套及 A 的 generation 指导验收；交叉引用未归档 `lc-root-initialization` delta 的 REQ-REG-14，不修改主规格不存在的条目。

## 规格来源与时序

创建本 change 时，主规格的编号条目为 REQ-REG-01…13（05/06 位于其它 capability），REQ-REG-14「LC provider 信任登记生命周期」在 `openspec/changes/lc-root-initialization/specs/logical-codebase-registration/spec.md:36`，该 change 未归档。本 change 新增 REQ-REG-15，不改旧 change、不同步主规格、不为通过校验伪造 MODIFIED REQ-REG-14。后续同步/归档须保留 REQ-REG-14 与 15 的独立场景，避免旧 delta 覆盖新确认合同。

只读 gateway 约束引用 `lc-gateway-multi-provider/specs/lc-gateway-multi-provider/spec.md` 的语义标题「LC admission 与 spawn 前复验」「可验证的 provider policy projection」；REQ-LCG-02/03 是该源 change `tasks.md` §1.1/§1.3 的工作包映射编号，delta 本文没有这些编号，不能重造编号 Requirement。驾驶舱冻结合同沿用主规格 REQ-CFC-06，不新增或修改该 requirement。

## Non-Goals

- 不扩展方案 B：不在 `start_generation` 用户写链自动登记，不改 REST SC recovery claim/replay 合同；新根首次生成可等待，但不得只有无指导裸错。
- 不改变五步 Claude recipe、不枚举已安装/默认/provisional provider、不扩大到无 LC gateway 的单仓会话，不让前端提交 root/provider 作为权威。
- 不自动纳入 Story/Design AuthorConfirm 或 batch_confirm 的独立 HTTP confirm；原收件箱只能选择合法 human_gate，compile_recovery 等非 WS confirm 门仍不可勾选。
- 不给 GET、eligibility、factory、early admission、gateway 或 spawn 写 home/ownership/trust audit 的权限，不把 trust 视为 capability，不绕过真实 launch/resume/write-boundary 证据。
- 不覆盖用户 untrusted/conflict 值，不认领用户自建或其它 LC 的条目；不引入跨会话批事务、回滚、自动重试、后台登记器或新长期 waiting UI。
- 不承诺 home 与 durable facts 跨介质原子事务，也不承诺 sidecar 锁保护外部非协作编辑器/CLI 的强原子 CAS。
- 本次交付仅为 change 文档；不创建实施 Plan、不改产品代码/测试、不 commit。书面契约获批后单独进入 writing-plans。

## Impact

- 产品面：registry/precondition/home adapter 的中立 preparation、指导、锁与事实排序；workspace engine 的确认上下文、合法性预检、typed waiting 与既有 CAS 前接线；生产 session manager 复用缓存依赖。
- 协议面：WS 确认 effect 与 REST `workspace_human_action` 的同一 waiting 映射；generation 仅复用原 waiting 投影补同源指导；不新增 WS 消息族或 REST 登记 endpoint。
- 前端面：bulk runner 保留稳定审计 code、展示 waiting 的 message/context；当前 live driver 同样能看到指导，不增加重试行为。
- 安全面：按 home 工件稳定 sidecar 串行化所有 Aria 写方，固定 home→LC fact 锁序；生产依赖现有 Unix 文件锁，平台不支持或锁失败时 fail-closed waiting。
- 验收面：isolated HOME 的 TDD、真实公共 approve/REST 链、只读前后快照、受控锁/事实交错与部分提交；终验由 controller 在授权测试实例操作，不触碰无关 home 条目或用预登记 fixture 冒充自动登记。
