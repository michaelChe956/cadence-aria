# Trust 指引卡简化：两行说明＋可复制命令窗（owner 2026-10-12 直批冻结设计）

- 日期：2026-10-12
- 执行：SimplifyTrustCard（glm53-task，独占 trust 指导生成器＋前端卡面）
- 范围：owner 验收嫌 trust 未信任指引卡过于冗长（分类码＋原拒绝串＋路径/键/JSON 形状/retry_action 长文全平铺）——按冻结设计重做为「2-3 行简洁说明＋可复制 Markdown 命令窗」，冗长细节收进「查看诊断详情」折叠；`provider_trust_missing` 等稳定码协议不变。
- 结论速览：**通过。** 后端命令段结构化（wire 新帧 `trust_missing_guidance`＋三处既有 context 增量字段 `command_markdown`，同源 adapter `manual_command`）；前端卡面重排（两行说明＋命令窗＋复制按钮＋保留「登记并重试」＋诊断折叠）。TDD 红先（7 前端红＋7 Rust 编译红）→ 绿 → 全量回归（web 2110/2110；Rust trust 族 256+38 全绿）→ web build → cargo build（rust-embed 重嵌入新 dist）。

## §1 设计落地对照（owner 冻结四点）

| # | 冻结设计 | 实现 | 验证 |
|---|---|---|---|
| 1 | 第一行直白告知「该目录(root)尚未被 Kimi/Codex 信任」（按实际 provider 单复数） | `TrustMissingPanel`：`该目录（{canonical_root}）尚未被 {Kimi 与 Codex} 信任`——providers 列表来自 wire 帧，单复数自然展开 | `CockpitInbox.test.tsx` 单 provider/多 provider 两断言 |
| 2 | 第二行怎么做「点击下方按钮一键登记；或复制命令自行登记后重新生成」 | 固定文案行 `cockpit-trust-missing-action` | 同上 |
| 3 | 可复制 Markdown 命令窗（Kimi 一行 shell／Codex toml 片段，后端已生成此文本）带复制按钮 | `cockpit-trust-command-window`（`<pre>` mono 渲染 `command_markdown`）＋`cockpit-trust-command-copy`（clipboard API，成功切「已复制」，失败不伪称） | 复制交互断言 `writeText(trustCommand)` |
| 4 | 「登记并重试」保留；冗长细节收「查看诊断详情」折叠；稳定码不变 | 按钮原样保留（`registerTrustEntry(provider)`）；原拒绝串/长指导只在折叠内（trust 行折叠移至卡尾）；`provider_trust_missing`/`trust_entry_missing`/`codex_trust_key_conflict` 原样上浮 | 既有 register 按钮测试＋折叠内容断言 |

## §2 后端：命令段结构化（避免前端解析字符串——设计禁令）

- **同源生成器**（`provider_trust.rs`）：新增 `trust_command_markdown()`——entry 缺失分型取 adapter `manual_hint().manual_command`（Kimi 单行 shell／Codex TOML 片段），冲突/工件损坏等其余分型 `None`（一键命令不得弱化「不覆盖用户条目」）。`ProviderTrustWaiting` 新增 `command_markdown: Option<String>`，两处构造点（`scenario_waiting`/`waiting_surface`）填充；平铺 `manual_guidance` 的命令段改由同一 helper 生成（单一事实源，不手抄）。
- **wire 新帧**（`out.rs`）：`WsOutMessage::TrustMissingGuidance { providers, canonical_root, reason_code, retry_action, command_markdown }`——`command_markdown` 缺席时省略字段（serde 兼容）。SC admission waiting 分支（`single_candidate.rs` → `provider_run.rs`）在 error 帧**之后**紧随发送；平铺 error message 与 `manual_guidance` 原样保留（旧客户端兼容，帧顺序保证「新 error 清旧指导」配对纪律）。构造走 `WsOutMessage::trust_missing_guidance_from_waiting()`（wire 层不二次推导）。
- **既有结构化 context 增量**：`decisions.rs`（`workspace_confirmation_trust_waiting`）、`register_trust_entry.rs`（`trust_registration_waiting`）、`aggregate_initialization` handler 三处 context 各增 `command_markdown`（additive，原 `manual_guidance` 保留）。
- **不动面**：`classify_launch_trust_failure_details` 分类桥（author 族共享边界，FixAuthorAnchor 禁碰范围）零改动——author/reviewer/coding 启动臂错误仍走平铺文本，前端以既有稳定码文本识别回落（无命令窗，按钮仍在）。

## §3 前端：卡面重排与配对纪律

- **store**：`trustMissingGuidance: TrustMissingGuidanceState | null`（wire 镜像）；`case "trust_missing_guidance"` 落 store；`case "error"` 先清旧指导（只配对最近一次错误）；reset/切会话/SessionState 全量替换清空（error 本身也不在这些路径存活，无孤儿态）。
- **投影**（`workspace-cockpit-projection.ts`）：`TrustMissingProjection { provider, providers?, canonicalRoot?, commandMarkdown?, retryAction? }`；检测顺序 `trustMissingFromGuidance(state) ?? trustMissingFromErrorText(state.error)`——结构化优先，旧服务端文本识别回落（`undefined` 视同 `null`，兼容 observer 部分构造态）。
- **卡面**（`CockpitInbox.tsx`）：trust 行=两行说明→命令窗→「登记并重试」→诊断折叠（卡尾）；非 trust 硬错误条维持既有布局（`HardErrorDiagnostics` 局部组件复用）。已登记态维持「已登记，请显式重新生成」。
- observer 连接（只读视图）default 分支静默透传新帧，无协议漂移诊断噪音；driver cockpit 为唯一消费面。

## §4 测试（TDD 红先）

红（先写、先跑失败）：

- vitest 7 失败：投影结构化优先/回落/error 缺席不投卡；handler 落 store/新 error 清旧指导；卡面两行/命令窗/复制交互/多 provider/无命令降级。
- Rust 7 编译错（E0599 缺变体/E0609 缺字段）即为红。

绿与回归：

| 套件 | 命令 | 结果 |
|---|---|---|
| Rust 同源生成器 | `cargo test --lib provider_trust_waiting_structures_command_markdown_same_source_as_adapter` | 1/1 |
| Rust wire serde | `cargo test --lib trust_missing_guidance_frame_roundtrips_and_omits_absent_command` | 1/1 |
| Rust WS 全链（error 帧+指导帧；Kimi entry 缺失给命令、Codex 冲突不给） | `cargo test --lib lc_first_generation_trust_missing_emits_structured_guidance_frame` | 1/1 |
| Rust trust 族回归（provider_trust/single_candidate_lc_admission/register_trust_entry/conversational_gate/aggregate_initialization/trust_route） | `cargo test --lib -- <过滤集>` | 256/256 |
| Rust fixture 涉及模块回归（coordinator_tests_profile_trust/workspace_human_action/conversational_gate_close） | 同上 | 38/38 |
| web 全量 | `npm test`（vitest --run） | 2110/2110 |

## §5 发布链与环境纪律

- web build（`tsc -b && vite build`，产物 `dist/assets/*`）→ cargo build（build.rs rust-embed 编译期嵌入 `web/dist`，`rerun-if-changed` 触发重嵌入）——顺序执行（worktree 磁盘产物链证据见 §7；运行服务侧由 Main 补 build＋重启收口）。
- cargo build 警告均为并行工作预存（来源文件均非本任务改动面）；controller 重启按「服务勿动」纪律留给 Main 执行发布。
- 提交：`feat(trust): 指引卡简化为两行说明与可复制命令窗`（显式列文件，不带并行 worker 的未提交改动）。

## §6 遗留与边界（如实登记）

1. **单复数来源**：当前 SC admission waiting 每 provider 一条等待帧（单元素 providers）；多 provider 并列渲染已就绪（wire 列表+前端 join「与」），但「同帧多 provider」聚合尚无生产产生点——按冻结设计实现为列表，未来确认面/批量场景可直接受益。
2. **冲突分型卡面**：结构化帧给 `providers/canonical_root/retry_action` 但无命令——卡面降级为两行说明+登记按钮（无命令窗），符合「不覆盖用户条目」纪律；登记按钮点击后走 ensure 链可能再落 `trust_registration_waiting` 平铺指导（既有行为）。
3. **bulk-confirm 确认面**（`bulk-confirm-runner.ts`）仍消费平铺 `manual_guidance` 文本（结果列表行，非指引卡）——`command_markdown` 已随 context 下发，卡面化留给后续（不在本次冻结设计范围）。

## §7 勘误：部署链证据纪律（Main 2026-10-12 11:05 勘误入账）

本报告初版「无旧 dist 坑」表述不严谨，勘误如下：

1. **属实部分（磁盘产物链，事后补证）**：worktree `web/dist/index.html` mtime `2026-10-12 10:54:32`，引用 `index-D7nkaPEs.js`（同 mtime存在）；worktree `target/debug/aria` mtime `2026-10-12 10:54:59` ≥ dist mtime——dev 构建确经 `rerun-if-changed` 重嵌入新 dist。
2. **不采信部分**：运行中的服务进程曾持续嵌旧包（进程内嵌早于本任务 dist 的版本），直至 Main 补 cargo build＋重启（bg_522，期望 served bundle=`index-D7nkaPEs.js`）——**磁盘二进制更新 ≠ 运行服务更新**；初版仅口头声称「按序执行」，未附证据，按新纪律不采信。
3. **流程教训（第二次同型事故，升级）**：凡报告「已构建/已部署」，必须附证据三元组：①服务二进制 mtime ≥ dist mtime；②服务 served bundle hash = dist `index.html` 引用 hash；③（涉及重启）服务进程 start time 晚于二进制 mtime。口头「已按序执行」不再作为完成依据。
