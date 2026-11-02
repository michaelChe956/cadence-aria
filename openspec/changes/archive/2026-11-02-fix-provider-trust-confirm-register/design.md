# Design: fix-provider-trust-confirm-register

## Context

基础架构设计直接引用 `cadence/designs/2026-10-11_方案设计_provider信任批量确认按需登记_v1.0.md`（下称「获批设计」），不重写其 §1–§11。动机/范围以 `proposal.md` 为准，行为以 `specs/logical-codebase-registration/spec.md` 的 REQ-REG-15 为准；本文只持久化 owner 的方案 A 裁决、Oracle `adopt_with_changes` 与 Max `feasible_with_changes` 必修增量。本文优先于获批设计中与这些增量不一致的表述。

### 已核验的增量落点

| 源码锚 | 当前事实/边界 |
|---|---|
| `src/product/workspace_engine/conversational_gate.rs::close_human_gate`，494–598 | 502–555 已检查 SC/type/stage、durable WaitingForHuman、Busy、候选完整性；557–560 的 Approve 臂在原 CAS 前，但尚未检查批准 phase/snapshot。 |
| `src/product/lifecycle_store/workspace.rs::compare_and_save_human_gate_close`，687–749 | approve 静态资格为 Approval/Evaluate 且 snapshot 非空；锁内比较完整 expected，胜者提升 Approval。原判据与迟到翻译不得改。 |
| `src/product/workspace_engine/controls.rs::handle_confirm`，31–65 | 非 SC 的合法 HumanConfirm 在 aggregate spec 校验后、定稿/创建子会话前接共用准备；Running 普通 confirm 不登记。 |
| `src/web/workspace_ws_handler/decisions.rs::apply_human_gate_termination`，180–219；`src/web/handlers/workspace_human_action.rs::handle_request`，230–265 | WS 与 REST Approve 共用 effect；新增 typed TrustWaiting 必须更新 REST 穷举映射，不能只改 WS。 |
| `src/web/workspace_session/manager/mod.rs::WorkspaceSessionManager::create`，380–394；`src/web/handlers/aggregate_initialization/production_dependencies.inc.rs::production_provider_trust_precondition`，684–719 | manager 已解析 issue LC 并构造只读 gateway，可复用 state 缓存 precondition；fake runtime 写 home=workspace_root。 |
| `src/product/logical_codebase/provider_trust.rs::ensure_before_recipe`，817–883；`ProviderTrustError::transient`，37–42 | ensure 之后在外围写 waiting/audit，有较晚失败复活陈旧 waiting 的窗口；retry_action 硬编码「重试初始化」，须场景化。 |
| `provider_trust.rs::ensure_trusted/reuse_trusted_entry/revoke_trusted`，458–648、707–813；`provider_trust_store.rs`，22–66、99–101、135–151 | Trusted 仍读改归属/clear waiting/追加审计；单次 fact 锁不覆盖全生命周期，真实 schema 为 ensure/registered、replay、reuse_user_entry 与 gate_waiting。 |
| `src/product/coding_attempt_store/locking.rs::ExclusiveFileLock`，53–112、235–299；tests，313–379 | 有稳定 sidecar、同步/异步获取与取消安全先例；非 Unix 不支持锁，须 fail-closed。 |
| `src/web/workspace_ws_handler/run/single_candidate.rs::preflight_lc_member_rules_before_author_run`，491–578 | 既有 AdmissionWaiting 会 `persist_single_candidate_waiting_phase` 回落 Prepare/Open；获批设计所引用的旧 491–495 已移动，以此符号/真实分支为准。 |
| `src/web/gateway_factory.rs::build_scoped`，203–222；`provider_trust.rs::ReadonlyProviderTrustSource`，94–240 | factory 从生产 HOME 装只读 source，与 fake registry 的 workspace_root 可能不同；纯 verify 禁止 ensure/revoke/ownership/audit 写。 |
| `web/src/components/chat-workspace/cockpit/CockpitInbox.tsx::isSelectableGate`，869–881 | 只可选合法开态 human_gate，排除 AuthorConfirm、batch_confirm、compile_recovery 与 blocked/非法身份项；不扩大 bulk eligible 门。 |
| `web/src/state/bulk-confirm-runner.ts`，4–7、58–61、107–173、179–204 | 各远端会话一条 driver 短连接、一次裸 confirm；并发 3、8 秒超时，原 error detail 只有 code；当前会话另走 live driver。 |

CodeGraph 精确符号/路径查询返回了无关源码，已报告工具问题；上述针对已知锚的核验降级为 ast-grep outline 与定点读取。此为设计接地，不是产品测试证据。

## Goals / Non-Goals

**目标：** 延伸现有 registry、engine 确认裁决、WS/REST effect 和 generation waiting 投影，不建立第二套登记算法或状态机；确认授权与无副作用资格校验在产品层统一。

**设计边界：** 仅公共 WS confirm 与 REST human action approve 自动登记；独立 HTTP confirm、start_generation 登记、REST SC recovery claim/replay 变更不在方案 A 内。generation 附带项只补同源只读指引与已有等待投影，gateway/factory/GET 的 trust 事实读写边界不变。其它非目标见 proposal。

## Decisions

### D1. 公共批准链与无副作用合法性预检

沿获批设计 §5 的产品层落点，不在 inbound/socket 分发前登记。SC Approve 在现有类型/开态/Busy/候选检查之后、原 CAS 之前，对**已加载的 expected** 再纯检查 phase∈{Approval,Evaluate} 且 snapshot 非空；不满足则构造原同类 Conflict 并走既有翻译，零 ensure/home/ownership/audit。不调用带写入的 CAS 充当预检，也不更改 CAS 判据。

全部 trust Ready 后仍将同一 expected 交给原 CAS；不得重读替换 expected 消除竞争。preparation 不改 workspace status/phase/snapshot、节点、预算、turn、compile reservation/start ledger；只允许合法确认对应的 trust 工件/事实变化。CAS 败者不 compile/child/第二次 close；SC 成功仍在 deterministic compile 与 durable Confirmed/Completed 核验后才发唯一 close。

非 SC 仅在 `handle_confirm` 的 HumanConfirm 臂、`validate_confirm_aggregate_spec_gate` 之后准备，Running 臂不动。弃用 inbound 前登记方案，因为快照陈旧且资格尚未核定；不把“bulk”伪装成产品能区分的来源。

### D2. 注入、权威身份与中立 preparation

沿获批设计 §6/§11.1，manager 使用同一 `WebAppState` 缓存的 aggregate initialization dependencies/trust 实例，注入确认 context；包含 issue 原始 `Option<lc_id>` 与 registry 的显式/既有 legacy LC scope。root 唯一权威仍是冻结 gateway `authority_root()`，不另造浏览器/root authority。

准备时重解析原始 issue 归属、校验同 scope manifest root、canonicalize 后与冻结 root 相等；LC 缺 context/precondition fail-closed，ContextWaiting 不伪造未知 provider/key/root、不提供错误根 trust 指引。有效 provider 使用 SC durable expected 或非 SC mutex 内当前锁定 author/effective reviewer；尊重 disabled reviewer，排除 provisional/default/installed provider。

将 `ensure_before_recipe` clean-cutover 到中立 `ensure_ready`，保留现有过滤去重与单 provider registry 算法，迁移 handler、coordinator、fake/测试全部 caller，不留别名/shim。**修正获批设计 §6.2「参数不变」：** 场景信息须进入 preparation/等待投影，使 recipe、confirmation 的 retry_action 各自正确；generation 的同源只读指导投影也使用生成场景。具体新类型名属于后续 Plan 的命名展开，不改变此场景契约。reason_code 原样保留，不靠 message 前缀猜 trust waiting，不让冲突 retryable=false 误解为永不可恢复。

### D3. 全生命周期锁、waiting 排序与取消边界

沿获批设计 §7 的必要 sidecar 配套，但锁范围明确扩到 **ensure_trusted/revoke_trusted 全体**：首次 digest/read_state 之前获取，覆盖 Trusted 的 load_ownership→record_ownership→clear_waiting→audit，以及新登记/撤销事实提交。Codex 按整个 config.toml 目标共锁，Kimi 按实际 trust record 锁稳定 sidecar，不锁会被 rename 更换 inode 的数据文件。固定顺序 **home 工件锁→LC fact 锁**，不得反向嵌套；继续保留 digest CAS/atomic rename。

**waiting 协议选择：** 把 `ensure_ready` 每个 provider 的失败 record_waiting/gate_waiting 纳入同一 home 临界区，而非 ensure 返回释放锁后再写当前 waiting。公有 ensure/revoke 与准备循环共用已持锁核心，避免二次加同一非重入锁。成功 clear_waiting 与失败当前态按此串行顺序生效，旧失败不得晚于较新 Ready 留下陈旧 waiting；历史失败审计仍可保留。缺 adapter/锁获取失败没有 home 临界区可用时，仍返回可见 Waiting，历史 gate_waiting 审计尽力而为，但不得无序改写该键的当前 waiting 或进行无保护 home 写入；不伪称完整持久化审计。

同步 IO 放在 `spawn_blocking`，但**不可**只把无界阻塞 flock 丢进可被父 future 取消的闭包：锁等待沿既有 `ExclusiveFileLock::acquire_async` 的可取消语义；等待取消不复活业务、不阻塞 runtime 退出。开始临界提交后，锁与确认 engine 互斥的所有权必须覆盖该次提交至结束，即使调用 future 被取消也不得释放互斥后让 detached work 关闭另一门；已开始提交允许安全收尾、不自动回滚。Ready/Waiting 返回时释放 home 锁，绝不带入 compile/网络/provider run，取消的 caller 不再推进原 CAS。后续 Plan 展开私有持锁核心与所有权传递方式，但不得削弱这些不变量或增设登记自动重试循环。

### D4. typed waiting、指引与 WS/REST/前端投影

沿获批设计 §6.2/§8，HumanGateCloseOutcome 与 WorkspaceConfirmOutcome 增 typed TrustWaiting，区分 HomeWaiting 与 ContextWaiting；`HumanGateTerminationEffect` 同样保留 typed waiting。WS 转既有 ProtocolError，REST `workspace_human_action.rs` 转稳定 ApiError，统一 code=`workspace_confirmation_trust_waiting`；所有 exhaustive match 更新，compile findings、Busy/AlreadyClosed、其它错误与成功广播保持原语义。

HomeWaiting 至少含 session/project/LC/provider/root/key、原 reason、retry_action、manual_guidance、retryable:true；后者表示修复后可显式确认，不表示冲突可无条件立即重试。ContextWaiting 只返回已知身份。operation_id=`workspace_confirm:<session_id>` 仅用于关联，非确认幂等键。

manual_guidance 延伸实际 home adapter 的窄只读能力，同一生成器供 registry waiting 与 generation 使用，不复制 Kimi key 算法/JSON 或 Codex TOML 转义。指导区分 home 权限/格式/冲突与 fact IO；无 adapter 只给装配修复。中文 message 本身要可读，包含门未关闭、原因、人工处置和修复后显式确认，不把指导只藏 context。

前端仅对新 waiting code 最小读取 message/context，detail 去重展示必要指导；markRejected 仍只存稳定 code，其它 protocol_error detail=code。live driver 同样显示 message。报告仍 rejected，不新增长期 waiting/重试按钮/自动重发，8 秒超时仍结果未知。

### D5. owner 方案 A 附带要求：generation 指导而非登记

首次 author/reviewer 生成先于批准门；确认插入点不能修复从未登记 root 的首 admission。owner 已选 A，但不接受裸错：在实际 trust 拒收的 generation waiting 投影桥接同源 manual_guidance，复用既有 AdmissionWaiting/Prepare/Open 回落，不在 start_generation 插入 ensure，不改变 provider freeze/rearm/claim/replay。

指导来源只读且绑定已核验 scope/root/provider；不得调用会写审计的 `ProviderTrustRegistry::verify_trusted`，不得调用 ensure、写 waiting/ownership/trust audit 或给错误 root 指导。针对 trust 失败用类型化/既有判据传递原因，不能把任意 ProviderUnavailable 都冒充 entry absent。retry_action 是「人工登记/修复后显式重新生成」，不是重试初始化/确认。能力、政策、成员材料等其它门及其 waiting 原样保持；满足其它门的新根无预登记用例必须观察到 provider 零启动、home/trust durable 快照不变与真实指导。

### D6. 测试锚纠偏与规格时序

不照抄获批设计 §10 的“已有 readonly source tests”：`provider_trust.rs` 实际只有 4 项 registry tests，`provider_trust_adapters.rs` 也为 4 项；`gateway_factory.rs` 的 7 项现有测试没有直接断言只读零 home/ownership/audit 副作用。新增 missing/conflict/登记后删除三态的 readonly verify/build、GET/early/admission/spawn 快照锚，不能把既有绿灯冒充此保证。

至少一个 isolated fixture 必须经过真实 WebAppState/manager create→角色仲裁→Confirm→registry→journal/broadcast，不只用 manager test_fixture。fake runtime registry home=workspace_root，而 factory readonly source 使用实际 HOME，测试须让两面命中同一个隔离 home，避免读取用户条目或写用户 HOME。

并发用 barrier/锁 attempt hook 控制窗口，不用 sleep 概率碰撞；明确同 root 同 LC owned=true 不降级、同 root 不同 LC owned=false、不同 root 共享 Codex config、ensure/revoke、原 CAS 中途漂移、旧 waiting 晚写、取消与客户端超时。Trusted 幂等须断言 adapter 写次数=0，不能仅比较最终字节。

REQ-REG-14 与 REQ-LCG 标题来源/ADDED REQ-REG-15 策略见 proposal；本次不修改旧 change 或主规格。源 change 先归档也不会将本新增 requirement 改成 MODIFIED；同步时重新核对基线并保留独立合同。

## Risks / Trade-offs

- **[限制] 外部非协作写方不遵守 sidecar。** → 只承诺同协议 Aria 写方安全，保留 CAS 的漂移检测，不能宣称检查后到 rename 的外部窗口已消除。
- **[限制] home/facts 非跨介质事务。** → 失败门开、已写条目保留，缺归属时保守 user-owned；不自动补偿删除，不保证失败审计必落盘。
- **[限制] Unix 文件锁前提。** → 非 Unix 或锁失败按真实错误等待，不无锁降级，不为本修复新造跨平台锁框架。
- **[限制] 首次生成仍可能到不了确认门。** → 单独验收“新根首次生成带指导”，不能称首次自动可用，也不能预写条目藏缺口。
- **[竞争] 准备成功不等于确认成功。** → 原 expected/CAS/compile/Confirmed 唯一 close 不动；取消/超时用户先观察权威门态，禁止自动重发。

## Migration Plan

按 tasks 依赖序先以 TDD 锚定批准资格，再中立化/同源指导与全锁，接引擎/生产 manager、WS/REST 与前端、generation 只读指导，新增只读快照和真实接线锚；每项绿后独立审查。精确文件步骤/命令/提交边界由书面契约获批后的 writing-plans 单独制定，不在本 change 执行。

不得保留 recipe 专属旧接口别名，不改变五步行为。失败不撤销已提交 trust；若部署回退，不自动清用户 home 条目。实施完成后由 Main 统一验证、controller 授权现场复验，再走规格同步/归档。

## 获批设计与双审条款映射

| 条款/来源 | 本 change 落点 | 可验收锚 |
|---|---|---|
| 获批设计 §2/§3/§5；Oracle conditions 1–3、Max must_fix 2/3；owner A | proposal What Changes/Non-Goals；D1/D5；REQ-REG-15「公共 WS 与 REST」「新根首次生成」 | tasks 1.1、4.1、5.1、6.1、8.2 |
| Oracle condition 4；Max must_fix 1 | D1；「不合法确认在登记前拒收」「准备成功后仍使用原关门竞争判据」 | tasks 1.1、4.1，phase/snapshot 矩阵及受控 CAS 漂移 |
| 获批设计 §6；Oracle condition 7；Max must_fix 5 | D2/D4；「有效 provider 集」「冻结身份」「确认登记失败」「provider 特定人工指导」 | tasks 2.1、2.2、4.1，场景化 retry_action、原 reason 与实际路径 |
| 获批设计 §7；Oracle condition 5；Max must_fix 4 | D3；「已可信条目复用」「Aria 协作写方并发」「取消锁等待」 | tasks 3.1/3.2，全 ensure/revoke、锁序、owned 与取消矩阵 |
| Oracle condition 6；Max concurrency/test_gaps | D3；「失败 waiting 不覆盖较新成功当前态」 | tasks 3.2，barrier 控制旧失败/新 Ready 排序 |
| 获批设计 §8/§9；Oracle condition 10；Max must_fix 2/5 | D4；「确认登记失败」「批量入口」「部分提交」 | tasks 2.2、4.1、5.1、5.2，真实 audit action/outcome、稳定 code |
| 获批设计 §10 锚纠偏；Oracle condition 8；Max must_fix 6 | D6；「GET 与 gateway 复验仍零 trust 副作用」 | tasks 7.1/7.2，新增 source/factory 快照与真实生产 manager 测试 |
| Oracle condition 9（仅 B）；owner 明确 A | proposal Non-Goals、D5；本 change **不采纳 B 的 generation 登记/REST claim 修改** | tasks 6.1 只读指导、8.2 A 附带验收，不伪列 B 已实现 |
| Oracle conditions 10–11/spec_baseline；Max 源标题映射 | proposal 规格时序、delta 顶部、D6；ADDED REQ-REG-15 交叉引用活跃 REQ-REG-14 | tasks 8.3，strict 与源基线核对，不制造 REQ-LCG 编号 |
| 获批设计 §10 owner 锚与 §11 风险；Max 全测试清单 | D6/Risks；「owner 删除条目后重开确认」「新根首次生成」 | tasks 8.1/8.2，分别留自动补/审计/唯一关门与只读指导证据 |
