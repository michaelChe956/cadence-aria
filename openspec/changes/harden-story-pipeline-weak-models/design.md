## Context

- Story author prompt（`prompts.rs::build_prompt`、`prompts/revision.rs::build_revision_full_prompt`）每轮全量重放历史；reviewer（`prompts/review.rs::build_review_input`）已有 context filter 但仍拼 Issue 全文与完整 artifact。
- 结构化协议（`cross_cutting/structured_output.rs`）要求结束标签重复 nonce；`product/workspace_engine/parsers.rs` 另有一套重复的 sentinel 实现（含自己的结束 nonce 校验）——协议变更必须两处收敛，且消费方覆盖 workspace review、aggregate author、coding review、image prompt iteration、work-item split、streaming fake。
- Kimi adapter `initialize` 空能力、未实现反向 `terminal/*`、`fs/*`；`mcpServers` 硬编码 `[]`；`json_rpc_peer` 未知请求统一回 Method not found，无并发请求分发。二进制逆向与裸 ACP 实测确认：kimi 要求 `terminal === true`（布尔 dialect），bash/grep 失败文本固定 `"ACP terminal capability is unavailable"`。
- severity 现为 6 档，后端 DTO、WebSocket 类型（`web/workspace_ws_types/review.rs`）与前端（`workspace-ws-store-types.ts`、`ReviewVerdictEntry.tsx`）均依赖。
- 基线证据：`.aria/projects/project_0001/issues/issue_0001`。

## Goals / Non-Goals

**Goals:**
- 弱模型 story 全链路一次成功率 ≥95%（20 样本/组合 release gate）
- 修订轮 provider 实际 input-token usage 降 ≥40%（usage 口径，fresh/resume 分列）
- kimi bash/grep/文件工具可用且不突破权限/沙箱

**Non-Goals:**
- 不改 daemon canonical writeback / 人工确认 gate 边界
- 不改 story spec artifact schema（heading/ID/追踪 token 契约不动）
- 不做 provider 差异化 prompt 降级
- 不追求产物逐字节一致（规范化 golden 语义等价）
- Design 阶段的 design spec/review 优化不在本 change

## Decisions

1. **协议拆分**：author"一次成功"以 artifact gate 口径单列 requirement；sentinel 协议单列 requirement 并枚举全部消费方，同一工作包内原子切换（parser 收敛 + 全部 prompt 文案 + fake/test 控制同步），避免中间态使 coding 链路不可用。
2. **nonce 语义**：JSON 顶层 `"nonce"` 是 envelope 字段，parser 校验后剥离，不进入业务 payload 反序列化；开始标签属性为主校验点，JSON 内字段为冗余防注入。**不做旧格式兼容**（结束标签带 nonce 一律拒绝），消费方在同一次提交原子切换。
3. **few-shot 防照抄分两类**：sentinel 消费方（reviewer 等）示例 nonce 用固定 `EXAMPLE_NONCE` 占位（任何请求不会派发该值），真实 nonce 只出现在示例后的输出模板；author 示例用无稳定 ID/追踪 token 的 artifact 结构骨架，不引入 sentinel nonce、不改持久化 schema。"照抄示例必须失败"进单测（reviewer 走 nonce 校验、author 走 artifact gate）。
4. **JSON 恢复 fail-closed**：仅在 nonce 匹配的 sentinel 内容内做 string-aware 唯一顶层对象提取 + 一层 fence 剥离；多候选/超限即失败。不做尾随逗号修复（原 task 删除该用例）。
5. **severity 双入口**：live payload 三档严格；历史回放归一化后不再写出旧值；`impact` 并入规则 `"\n影响：" + impact`（空值不追加），前端类型/渲染/DTO 同步，含 round-trip 测试与 `pnpm test`/`tsc -b`。
6. **滑动窗口双路径**：`build_prompt`、`build_revision_full_prompt`（修订主入口）、`build_review_input` 三处接入；摘要为确定性 schema（round、artifact 版本、verdict、finding ID/severity/required_action、未关闭强 finding 全文、choice ID+答案+绑定、retry 原因）；未关闭 blocking/must_fix 全文与 choice 审计永不压缩；diff 摘要基于相邻 artifact 版本，生成失败保留全文；摘要生成失败 fail-closed 退回全量。**resume 策略先实测**：测各 provider resume 的 usage，若服务端重放历史使窗口无效，则窗口边界切 fresh session + 压缩上下文包。
7. **kimi wire 先行**：阶段 5 首任务是用真实 kimi ACP transcript 冻结 `terminal/create|kill|release|wait_for_exit`、`fs/read_text_file|write_text_file` 的请求/响应/输出通道 fixture；实现按 fixture 驱动。
8. **终端执行默认 pipe + 每命令 argv 封闭 grammar + OS 级隔离**（独立 stdout/stderr、进程组、每命令超时、输出上限 + backpressure 截断）：服务端按子命令构造 argv、用户值词法校验（`-` 开头拒绝、`--` 固定位置、`<N>` 数字）、二进制可信绝对路径/固定 PATH；git 配置隔离用 `GIT_CONFIG_GLOBAL=/dev/null`+`GIT_CONFIG_SYSTEM=/dev/null`+`GIT_CONFIG_NOSYSTEM=1`+`GIT_CONFIG_COUNT=0`（不是 unset）+ 经验证只读/净化 gitdir/config 视图禁用 `core.fsmonitor`/`core.externalDiff`/`core.textconv`/自定义 helper + 子进程 `clearenv` + 环境 allowlist；固定 `--no-pager`（git 全局选项）、`--no-ext-diff`/`--no-textconv` 仅 diff。auto 模式 terminal 在 bubblewrap 只读隔离中运行（授权根 bind-mount、无网络、无新特权），bwrap 不可用则 auto 拒绝 terminal（fail-closed）、仅 supervised 经 ApprovalBridge 批准「合规模板内」命令非隔离执行并记录（grammar 对 auto/supervised 均强制，supervised 不提供任意命令旁路）。仅 fixture 证明依赖 TTY 才引入 pty（`TERM=dumb`），模式记入审计。独立 `KimiClientServiceDispatcher` 并发分发，`wait_for_exit` 不阻塞 kill。攻击性测试覆盖 option-smuggling、`cat /etc/passwd`、`git -C ~/.ssh`、`git diff --output=x`、`find -exec sh`、`sed -n -i '1p'`、`sed -n '1e...'`、`rg --pre`、`rg --follow`、`grep -R`、`find -L`、`>` 写文件、bwrap 缺失时 auto 拒绝。
9. **权限与沙箱**：terminal/fs 全部过 `ProviderPermissionMode` + 角色 policy（reviewer 只读、coding 限 worktree）；授权根 canonicalize + no-follow 封装，覆盖 symlink/新建父目录用例。
10. **MCP 走 envelope**：`mcpServers` 由 policy envelope 校验的受控 bundle 派生（allowlist/digest/脱敏/argv 审计），session/load 校验 digest 一致性；不从 provider 普通配置透传。
11. **campaign 设计（用户已确认 B）**：固定 ≥5 需求形态语料（含待确认项、含返修轮、含用户 choice）；开发迭代 5 样本/组合；最终 gate 20 样本/组合（5 形态 × 4 样本，每形态 ≥3）、独立 issue/session；author/reviewer/full-chain 三口径 + retry 分布 + 失败分类 + usage（fresh/resume 分列，cache_read 单列）；token gate 唯一公式：每 provider×strategy 配对样本均值 `revised_input/baseline_input ≤ 0.60`，retry 样本不入分母但入 manifest；golden 规范化 diff（冻结只读带 digest）；manifest 为 machine-readable schema，预算与中止条件预设。
12. **验证顺序**：阶段 0 基线（含 resume usage 实测与 kimi transcript 采集）→ P0（协议+示例+恢复+severity 全栈）→ P1（窗口）→ P2a（kimi 服务）→ P2b（MCP）→ gate campaign。每阶段定向测试 + 阶段退出条件，不全推迟到最后。

## Risks / Trade-offs

- **安全（最高优先）**：terminal/fs 赋予模型宿主执行/写入能力——强制缓解：权限门、角色只读、授权根沙箱、no-follow、输出/进程数上限、进程组 kill、supervised 审批；攻击性路径用例（symlink 文件/目录、越界 cwd、新建父目录）为必须测试。
- **资源耗尽**：长驻进程、子进程树、无限输出可能塞爆 bounded RPC 队列——最大终端数、输出截断、backpressure、session drop 清理、压力测试。
- **token 指标虚高**：resume 服务端历史不受窗口控制——usage 实测前置，无法观测 usage 的 provider 不得宣称达成 token 目标。
- **MCP 供应链**：mcpServers 可执行本地命令/携带凭据——仅 envelope 受控 bundle、digest、脱敏、禁任意 shell 字符串、resume 漂移拒绝。
- **campaign 非确定性/成本**：模型漂移、缓存、限流——记录 model/version/时间/失败分类、golden 只读带 digest、独立 issue/session、预算与中止条件。
- **协议全局变更**：消费方清单原子切换 + 全量回归；风险可控（无持久化旧格式数据，nonce 每次新生成）。
- **压缩丢审计细节**：确定性摘要 schema + 未关闭 finding 全文保底 + golden diff 验收；矩阵出现裁决发散时回调窗口。
- **pty 跨平台**：默认 pipe 避免；若需 pty，首发限 Linux，其余平台 fail-closed 并提示。
- **95% 是激进目标**：20 样本 gate 下 19/20 为最低线；retry 通道保留为兜底，不删除。
