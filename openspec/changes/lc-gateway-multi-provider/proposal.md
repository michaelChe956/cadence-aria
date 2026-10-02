# Proposal

## Why

LC 根初始化已经交付 canonical root、cwd/target 分离、trust、D1-D4、预算门和 Claude Code 固定 recipe，但后续 Story、Design、Plan、Coding、Review 的真实 provider 仍被 gateway 限定为 Claude/Codex，Pi/KimiCode 直接拒绝，Codex Coding 仍受 `danger-full-access` 硬门阻断。四家 CLI 已在真实根 fixture 中证明可发现规则与 MCP；现在需要把这些 provider 接入同一套 capability、policy projection、spawn 前复验、resume 和审计链，并在证据不足时保持 fail-closed，避免以“CLI 能启动”冒充 LC 会话可用。

## What Changes

- 建立 LC gateway 的四家显式 provider 矩阵：Claude Code、Codex、Pi、Kimi Code 各自拥有 `ProviderRefType`、`ProviderDialect`、wire dialect、版本和真实 adapter；未知 provider、Fake 与未来值不得静默回退到 Claude 或 legacy direct path。
- 将 capability 记录扩展为 `action × resume × write_boundary` 三态（`Confirmed`/`Denied(reason)`/`Unknown`）并保存 exact version、dialect、projection digest、证据引用和 provenance。旧记录缺少矩阵时按 Unknown 读取；Unknown/Denied 在 provider spawn 前阻断，明确 resume 请求另需 resume=Confirmed。
- 统一 LC session admission：复用已交付的 canonical `working_directory`（manifest `provider_context_root`）与独立 target/worktree 合同，在 validate→spawn 之间重新读取 authority、policy、target、cwd、trust、capability、projection、availability 和 D4 freshness；漂移时零 spawn。
- 为四家 provider 生成不可伪造的 policy projection，把 action、role、semantic tool policy、permission/approval、sandbox、MCP/config、trust、target 和 `writable_roots` 投影到真实 CLI/RPC/ACP 权限；逻辑 writable root 仍不是 OS 隔离证明。
- 收紧 Codex LC 安全模型：`danger-full-access` 永久禁止；Planning/Review 使用已验证的 read-only + on-request 组合；Coding 只在 `workspace-write`、protocol cwd/target 和 target-only boundary probe 均被当前版本证实后放行，否则保持 Denied/Unknown。修订 REQ-ENV-05，但不以 UI、resume 或 permission mode 覆盖硬门。
- 对齐 Claude/Pi/Codex 的 `validate_tool_policy_for_role` spawn 前双向 guard 与物理参数翻译；Kimi 不消费通用 tool-policy 字段，继续使用其独立 `ClientServicePolicy`，非法混用时 fail-closed。保持作者/评审 deny 与 Executor/Coder 无通用 deny 的 D1-D4 锁定语义。
- 同步更新 `automation_gateway_preflight` role-chain：一次列出所有违规 role/provider/action/reason/capability/projection 引用，覆盖 provider 映射、capability、trust、Codex 模式、write boundary、adapter dialect 和 tool policy；LC `gateway_required=false` 不得成为旁路。
- 为 Claude Code、Codex、Pi、Kimi Code 定义并验证原生 resume 形态及 fail-closed 规则，扩展 fingerprint 到 canonical cwd、tool-policy/projection、trust、MCP bundle（仅 Aria 注入通道）和 action-row evidence；漂移不续接旧 session，不把 resume 静默改为 fresh。
- 接通 streaming 与同步 adapter 栈，所有 LC 入口（Story、Design、Plan/split、Coding、Review）使用 validated gateway launch；不得继续使用裸 input、无政策 legacy bridge 或 provider 间自动切换。单仓 direct topology、参数、cwd 和输出契约保持不变。
- 建立四家真实 LC E2E 验收矩阵：五阶段 fresh/resume、真实 argv/RPC/ACP、tool calls/approval、policy/projection/trust/capability digest、target 正向写、root/非 target/`.git`/`.aria` 越界负向写、pre/post snapshot、D4 baseline 和每个失败场景的零 spawn 证据。
- 处理 E2E 缺陷 #8（聚合政策正文没有 authority-root 发布通道）的边界：本 proposal 推荐另立一个小 change（或由 `lc-root-initialization` 的收尾 change 归属）实现 policy body 的 canonical root 发布；本 change 只消费并在 admission 中校验该事实，不在 provider 启动时临时物化或复制政策。该前置未完成时四家 LC E2E 按缺材料 fail-closed，不以 fixture workaround 计为通过。

## Capabilities

### New Capabilities

- `lc-gateway-multi-provider`: 为 LC 的 Story/Design/Plan/Coding/Review 提供四家 provider 的统一、可审计、证据驱动 gateway admission、projection、resume 和 spawn 前 fail-closed 行为。

### Modified Capabilities

- `session-policy-envelope`: 扩展四家 dialect、action×resume/write-boundary evidence、provider projection、Codex 受限 sandbox、tool-policy/projection fingerprint 与 LC gateway 入口约束；保留 root-cwd/target、D4、Kimi native discovery 和 Aria bundle 边界。
- `logical-codebase-registration`: 将 provider capability/trust/projection/readiness 缺失纳入 provider 启动前的可操作等待；不伪造 capability，不从成员仓或 legacy layout 回退。
- `logical-codebase-aggregate-planning`: 使 Story、Design、Plan、split、Review 的 LC provider 入口统一经过 root-cwd + independent target + gateway；规划只读仍按 best-effort/证据级别表述。
- `multi-target-group-coding`: 保持每个 target-attempt 的单 target 快照、人工显式 StartCoding 与 D4 baseline；Coding provider 必须走 gateway，不增加跨 target 自动编排。

## Non-Goals

- 不改变 LC 根初始化 recipe；recipe provider 继续固定为 Claude Code，不把 Codex、Pi、KimiCode 加入初始化五步。
- 不改变传统单仓初始化、单仓 direct provider topology、单仓 cwd/target 映射、GitFinalize、既有 provider fallback 语义或单仓输出契约。
- 不新增多 target 自动化、跨 attempt 依赖编排、隐式 StartCoding、按依赖拉起其它 target-attempt 或一个 run 承载多个 target。
- 不把 logical `writable_roots`、MCP 成功、模型自述、`git status` 或 D4 post-check 单独当成 OS 级写隔离；缺少 provider/native 或 Aria-owned boundary 证据时不宣称支持。
- 不通过 prompt、环境变量、sidecar、规则复制、成员 thin pointer 或备用 cwd 伪造 provider 配置发现；不在 LC 中启用 provider 自动切换或无政策 fallback。
- 不处理 Kimi 原生 MCP 自发现与 Aria 注入 bundle 的边界之外的新配置来源；不改变既有 Kimi client-service policy。
- 不在本 change 内修复 E2E #10 僵死 run 注册、#13 的 Fake 默认 provider 策略或其它未直接影响 gateway 合同的台账项。
- 不把未覆盖的多版本 CLI、OS boundary launcher 可用性、Codex protocol cwd 兼容性或大 LC 新预算默认为 allow；均须以实施期证据决定 capability 状态。

## Impact

本 change 依赖 `lc-root-initialization` 已交付的 canonical root、cwd/target、trust、D1-D4、预算门和 readiness 合同；权威设计为 `cadence/designs/2026-10-01_方案设计_LC网关多provider接入_v1.0.md`，但其中旧的依赖落地描述和估时以当前 HEAD 事实校正。预研与实测证据包括四家 provider 根发现报告、provider-CLI 矩阵、36 成员写边界/版本报告及 LC E2E v1.1/v1.2；这些证据证明发现和前置合同，不替代本 change 的 gateway、resume、write-boundary 或五阶段 E2E 证据。

按 root initialization 已完成、现有 adapter/guard/D4/预算设施可复用的当前基线，本 change 重新估算 **12–18 人日**：能力与存储迁移 2–3 日，四家 projection/registry/sync-stream 接线 3–4 日，Codex 受限模式与四家 boundary evidence 3–4 日，admission/tool-policy/role-chain/resume 2–3 日，四家五阶段 E2E 与单仓对照 2–4 日；外部 CLI/OS 现场等待不计开发产能，但证据缺失必须阻断支持结论。
