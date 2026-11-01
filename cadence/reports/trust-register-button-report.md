# trust 指引卡「登记并重试」按钮与可复制登记命令 — 实施报告

- 日期：2026-10-12
- 变更：owner 直批小变更（免双审裁定已入账）——指引卡增强：①WS `RegisterTrustEntry` 动作（服务端解析 root → registry ensure → typed 回执 + 审计）②前端按钮 + 成功态 ③指导文案附可直接复制的登记命令
- 纪律：TDD 先红（缺类型编译红 + 断言红）后绿；不自动登记（按钮=显式授权例外）；不自动重新生成；guidance 不泄露凭据

## 1. 服务端（Rust）

### 1.1 WS 动作与 typed 回执
- `WsInMessage::RegisterTrustEntry { provider }`（`workspace_ws_types/in_.rs`）：wire `{"type":"register_trust_entry","provider":"codex"|"kimi_code"}`，**不携带客户端路径字段**（结构上不可传）。
- `WsOutMessage::TrustEntryRegistered { provider, action, trust_key, canonical_root, registered_at }`（`out.rs`）+ `TrustEntryRegistrationActionDto`（registered/replay/reuse_user_entry，与 ensure 审计 outcome 同词表）。仅回执，不自动触发重新生成。

### 1.2 处理链（decisions→registry ensure，场景=显式按钮授权）
- 新模块 `workspace_ws_handler/decisions/register_trust_entry.rs`：
  - 非 trust provider（Claude/Pi/Fake）→ typed 拒收 `provider_trust_not_required`（零 ensure 调用）；
  - 引擎锁短持有做解析快照，home IO（ensure）在锁外执行（不楔死引擎/run）；
  - `ensure_ready_cancellable`（既有 T2 中立 ensure 链 + T3 sidecar 锁 + 幂等/审计/归属隔离）复用，场景=新增 `ProviderTrustScenario::ExplicitRegistration`，operation_id=`register_trust_entry:<session>` 关联审计；
  - 失败：稳定码 `trust_registration_waiting` ProtocolError，message 含原 waiting.message + 同源 manual_guidance + retry_action；context 携 reason_code/trust_key/canonical_root/retryable（原样不折叠）。
- `inbound.rs` 分发臂 + `protocol.rs`：`message_type` 臂 + `requires_stage_validation` 豁免（带外修复动作；Prepare 停等与确认门开态——`workspace_confirmation_trust_waiting` 场景——按钮可达）。

### 1.3 root 服务端解析（与确认链同一 authority）
- 引擎新方法 `WorkspaceEngine::prepare_explicit_trust_registration(fallback_precondition)`（`workspace_engine/lifecycle.rs` + `types.rs`）：
  - LC 会话：确认上下文冻结根 + 同序三项漂移重验（issue 原始 LC 归属/冻结根 canonicalize/manifest 根一致），precondition 复用确认链注入实例；
  - 单仓会话：root=canonicalized repository 根，lc 标签=legacy 别名（与 coordinator 单仓 scope 同源），precondition 取 WebAppState 聚合初始化依赖（新增安全可空读取器 `aggregate_initialization_trust`）；
  - 解析失败 fail-closed：`trust_registration_context_unavailable/scope_changed/root_unavailable/root_changed`，零 home 调用。
  - 确认链既有实现零改动（不扰动其锁定的消息断言）。

### 1.4 指导命令追加（生成器内，按 provider）
- `ProviderTrustArtifactHint.manual_command`（`provider_trust_adapters.rs`，adapter 同源真实值）：
  - Kimi：单行 shell `mkdir -p '<dir>' && printf '{"root":"<root>","trustedAt":%s}' "$(date +%s%3N)" > '<record>'`（serde 转义 root，POSIX 单引号转义路径）；
  - Codex：TOML 不可一行安全写 →「config.toml 路径 + 完整 `[projects."<root>"]`/`trust_level = "trusted"` 片段」；
- `provider_trust_manual_guidance` 仅对 `EntryMissing` 分型追加「可直接复制的登记命令：…」（冲突/工件损坏不给一键命令，避免弱化「不覆盖用户条目」）。

## 2. 前端（TS/React）

- wire：`register_trust_entry` 入站 + `trust_entry_registered` 出站（`api/types/workspace.ts`）。
- 发送器 `useWorkspaceWs.sendRegisterTrustEntry`（沿 sendJson 既有 lease/connection 守卫，无乐观本地变更）；门面 `CockpitActionFacade.registerTrustEntry?`（可选面，未接线 fail-closed）；ChatCockpitPage 接线。
- 回执对账：消息处理器 → `store.applyTrustEntryRegistered`；`WorkspaceWsState.trustRegistration`（reset/切会话清空，同会话保留）。
- 投影：`selectCockpitInbox` 对 `state.error` 错误条按稳定码（provider_trust_missing/trust_entry_missing）+ provider wire 标签（codex/kimi_code）识别 → `trustMissing`；回执 provider 匹配 → `trustRegistered`。
- 卡面（`CockpitInbox.tsx` HardErrorInboxRow）：识别为 trust 缺失且未登记 → 「登记并重试」按钮（data-testid=cockpit-trust-register-button，经门面以 provider 出站）；登记成功 → 「已登记，请显式重新生成」状态（data-testid=cockpit-trust-registered-hint），按钮不再渲染；非 trust 缺失卡不投登记面。

## 3. 测试（TDD 证据）

红：先落 `tests/register_trust_entry.rs`（+mod 声明）与 provider_trust.rs 命令测试，编译期缺类型红（E0599 RegisterTrustEntry/TrustEntryRegistered/ExplicitRegistration/prepare_explicit_trust_registration/TrustEntryRegistrationActionDto 逐一确认）。

绿（命令与结果）：
- `cargo test --lib register_trust_entry` — WS 动作族：幂等两连击 registered→replay + ensure 审计两笔（operation_id=register_trust_entry:<session>）+ 冻结根回执；reuse_user_entry 不覆盖用户条目（home 字节不变）；非 trust provider typed 拒收零审计；waiting 稳定码+指导原样上浮；缺上下文 fail-closed；wire 仅 provider 字段；单仓 repository 根解析 + legacy lc 别名。
- `cargo test --lib provider_trust_` — 指导命令三例（Kimi 单行 shell 逐字断言/Codex 片段+路径逐字断言/冲突不给命令）+ ExplicitRegistration 等待面 retry_action 逐字断言。
- 前端 `npx vitest --run src/components/chat-workspace/cockpit/CockpitInbox.test.tsx src/state/workspace-cockpit-projection.test.ts` — 107 passed（含新增 6 例：按钮渲染+点击出站/成功态/非信任卡不投面；投影识别 codex/kimi_code/普通错误不投/回执匹配与不匹配）。

## 4. 回归与风险

- 确认链（conversational gate trust）零代码改动（新增方法并行实现，未触碰 `prepare_confirmation_trust_request`/`ensure_confirmation_trust_ready`）；trust 登记链（ensure/home 锁/审计）零改动，仅消费。
- 指导生成器新增段仅 EntryMissing 分型可见——既有 Confirmation/Recipe/Coding/Generation 场景断言的文案前缀不变（命令段追加在尾部）。
- 阶段矩阵：`RegisterTrustEntry` 豁免 stage 校验（带外修复动作；driver 租约仲裁仍生效——写面消息需 driver）。
- 已知边界：成功态依赖 WS 回执（内存态）；页面刷新后回到未登记卡面（error 原文仍在，可再点按钮，服务端幂等 replay）——符合「已登记，请显式重新生成」不自动重生成语义。

## 5. 文件清单

Rust：provider_trust.rs、provider_trust_adapters.rs、workspace_engine/types.rs、workspace_engine/lifecycle.rs、workspace_engine/mod.rs、web/state.rs、workspace_ws_types/in_.rs、workspace_ws_types/out.rs、workspace_ws_handler/mod.rs、decisions.rs、decisions/inbound.rs、decisions/register_trust_entry.rs（新）、protocol.rs、tests.rs、tests/register_trust_entry.rs（新）
TS：api/types/workspace.ts、hooks/useWorkspaceWs.ts、hooks/workspace-ws-message-handler.ts、state/workspace-ws-store-types.ts、state/workspace-ws-store.ts、state/workspace-cockpit-projection.ts、state/cockpit-action-routing.ts、components/chat-workspace/cockpit/CockpitInbox.tsx、CockpitInbox.test.tsx、CockpitInbox.test-utils.ts、state/workspace-cockpit-projection.test.ts、pages/ChatCockpitPage.tsx

## 6. Fix round 收口（2026-10-12，Main 定界后）

e726e80d 的两处「替换而非新增」编辑事故与类型错（VerifyFrontCount/Main 复现定界；05078435 已由 Main 修复 return 对象一处）：

1. `workspace-ws-message-handler.ts`：恢复被覆盖的 `import type { ChatEntry, ChatEntryRole } from "../state/chat-entries";`（与 TrustRegistrationReceipt import 并存，消除 3×TS2304）。
2. `CockpitInbox.tsx`：登记按钮 onClick 改为 provider 收窄后调用（`provider as WorkspaceProviderName`），消除 `?? ""` 传 string 的 TS2345。
3. `ChatWorkspacePage.test-utils.tsx`：mockWorkspaceWs 补 `sendRegisterTrustEntry: vi.fn(() => false)`（TS2322）。

验收（worktree/web）：
- `npx tsc -p .` → 残余恰 2 条既有 TS7053（CockpitInbox.tsx:319/:345，D 侧移交项，非本变更引入）。
- `npx vitest --run` → **212 files / 2093 tests 全绿**（此前 2 failed 为 sendRequestRevision 缺失族，05078435 修复后随全量转绿）。

教训入账：实施必跑 `tsc` 类型门——vitest 不做类型检查，替换式编辑事故只有类型门能拦。

## 7. P3 硬化与构建解锁（2026-10-12，Main 定界/授权）

【P3·k3 审查】Kimi 可复制命令 printf 转义（provider_trust_adapters.rs）：
- root 中字面 `%` 一律转义 `%%`（serde 序列化 root JSON 后转义；`%s` 占位符在格式字面量独立插入，不与转义互扰）——复制执行不写坏 JSON；
- 命令尾部注明 `# GNU/Linux（date +%s%3N 需 GNU date）`（macOS/BSD 失效提示）。
- 红先：`provider_trust_kimi_command_escapes_percent_and_notes_platform`（root=`100%s-%d-root` 夹具）——转义 payload 逐字断言 + 忠实 printf walker 回放（%%→%、%s←参数、占位恰一）产物 serde 解析为合法 Kimi 记录；修前红（payload 未转义、无尾注），修后绿。既有逐字断言同步补尾注。
- 回归：`cargo test --lib provider_trust` 36/36。

【构建解锁·Main 追加授权】CockpitInbox KIND_GLYPH/KIND_CLASS 补 `lc_resume_recovery` 中性机械条目（glyph 复用 RotateCcw，class 沿用 amber 卡片风格；D7.1 真正渲染恢复卡时复用或改）。
- 验收：`npx tsc -p .` 0 错（此前残余 2 条 TS7053 消除）；`npm run build` ✓（仅既有 chunk 体积告警）；CockpitInbox.test.tsx 41/41。
