## 1. 基线与 fixture 采集（P3 前置）

- [ ] 1.1 固定语料：整理 ≥5 个不同形态需求（含待确认项、含返修轮、含用户 choice），写入 campaign 语料文件并冻结 digest。验证：语料文件落盘 `cadence/reports/story-weak-model-campaign/`，含 digest 记录。
- [ ] 1.2 基线 campaign（5 样本/组合 × 三组合）：{claude code+glm-5.3, kimi+deepseek-v4-flash, pi+deepseek-v4-flash} 跑 story author+review；采集 author/reviewer/full-chain 三口径一次成功率、retry 分布、失败分类、input-token usage（fresh/resume 分列，`case_id`/`run_kind=baseline` 入 manifest）；记录 kimi bash/grep 失败现象为 evidence。验证：基线报告与 manifest 落盘。
- [ ] 1.3 resume usage 实测：各组合在有 provider session 历史时测修订轮 usage，判定是否需要 fresh-session 窗口切换策略（结论写入报告）。验证：报告含各 provider resume 行为结论。
- [ ] 1.4 golden 规范化基线：从 issue_0001 与基线 campaign 提取字段清单（heading/REQ/AC/NFR/decision/source），实现规范化 diff 脚本（machine-readable pass/fail）。验证：脚本对基线产物可运行并输出判定。
- [ ] 1.5 kimi ACP transcript 冻结：在专用非敏感 fixture worktree 真机采集 `terminal/create|kill|release|wait_for_exit`、`fs/read_text_file|write_text_file` 的请求/响应/输出通道 JSON fixture，字段级脱敏（禁止真实凭据/绝对路径/工作区内容），提交前 secret 扫描。验证：fixture 文件落盘 tests 目录且脱敏断言通过。

## 2. P0 协议与解析（原子工作包）

- [ ] 2.1 `cross_cutting/structured_output.rs`：新协议（结束标签无 nonce、JSON envelope nonce 剥离、**不做旧格式兼容**、闭合标签后尾随文本剥离进 readable、sentinel 内 JSON 与闭合标签间仅允许空白）+ 收敛 `workspace_engine/parsers.rs` 重复实现到单一实现。验证：RED 测试先行（新格式/JSON nonce 不一致/旧格式拒绝/envelope 剥离/照抄 EXAMPLE_NONCE 拒绝/闭合标签后尾随文本剥离/sentinel 内非空白拒绝），且断言 nonce 缺失/不一致/旧格式闭合不合法的稳定错误码可区分（供 repair prompt 分类），`cargo test --locked --lib structured_output` 与 parsers 相关单测全绿。
- [ ] 2.2 JSON 受限恢复（sentinel 内 string-aware 唯一顶层对象 + 一层 fence 剥离；多候选/超限失败；`MAX_JSON_BYTES=65536`/`MAX_JSON_DEPTH=32`）。验证：单测覆盖 fence 包裹、多候选失败、字符串内 `{}`、深度临界(32)与超深(33)、字节临界与 +1、错配括号、escaped quote/backslash、fence 外非空文本拒绝、超大输入。
- [ ] 2.3 全部消费方原子切换：workspace review、aggregate author（`aggregate_author_output_contract`）、reviewer（`reviewer_output_contract`）、review repair（`prompts/review_repair.rs`）、coding review、image prompt iteration、work-item split、streaming fake、web test controls（`web/test_controls/provider.rs`、`plan_repair/provider_matrix.rs`）的 prompt 文案与测试同步新协议，且每个消费方 fixture 写入并校验 JSON envelope nonce（contract-driven 回归断言，而非仅 grep）。验证：`rg -n 'ARIA_STRUCTURED_OUTPUT' src` 无旧双闭合指令；上述模块测试全绿。
- [ ] 2.4 few-shot 示例注入（sentinel 消费方用 EXAMPLE_NONCE 占位 + 真实 nonce 输出模板；author 用无稳定 ID/追踪 token 的结构骨架）：author 三个注入点（初次、`build_revision_full_prompt`、artifact retry）与 reviewer 裁决契约。验证：单测断言各注入点示例存在、reviewer 示例 nonce 不可通过校验、author 骨架不能通过 artifact gate。

## 3. P0-4 severity 三档（全栈）

- [ ] 3.1 后端：live 三档严格 + 历史回放归一化（完整映射含恒等映射与未知值拒绝，不再写回旧值）+ `impact` 并入规则（幂等只追加一次）；验证 live 解析路径无 6 档枚举分支（兼容映射与 fixture/历史回放测试中允许出现 `strong_recommend_fix` 等旧值字符串，不属于残留）。验证：单测覆盖 live 拒绝旧值、回放归一化、未知旧值拒绝、round-trip、impact 幂等。
- [ ] 3.2 Web 栈：`web/workspace_ws_types/review.rs`、`web/src/api/types/common.ts`、`web/src/state/workspace-ws-store-types.ts`、`web/src/components/chat-workspace/entries/ReviewVerdictEntry.tsx`、`web/src/components/chat-workspace/entries/ProviderStreamEntry.tsx`（含 `SEVERITY_RANK` 排序）与页面 optional 判定同步三档。验证：`cd web && pnpm test && pnpm tsc -b` 全绿，且 `rg -n 'strong_recommend_fix' web` 无残留。

## 4. P1 滑动窗口

- [ ] 4.1 确定性摘要生成器（design 决策 6 的 schema；未关闭强 finding 全文与 choice 审计永不压缩；fail-closed 退回全量）。验证：单测覆盖 ≥3 轮压缩、摘要失败退回、未关闭 finding 第 4 轮可定位。
- [ ] 4.2 接入三入口：`build_prompt`、`build_revision_full_prompt`、`build_review_input`（reviewer 保留 canonical_inputs 与未关闭 finding 全文，中间 artifact 相邻版本 diff 摘要、失败保留全文）。验证：三入口 prompt 单测 + 字符数下降断言（代理指标，标注）；1.3 若判定需要 fresh-session 切换，实现窗口边界 fresh + 压缩上下文包并有单测。

## 5. P2a kimi ACP 客户端服务

- [ ] 5.1 initialize 能力声明（kimi 布尔 dialect 常量）+ `KimiClientServiceDispatcher` 并发分发（wait_for_exit 与 kill 并发）。验证：adapter 单测断言 initialize payload；fixture 驱动并发用例（wait 挂起时 kill）。
- [ ] 5.2 终端实现（默认 pipe + 进程组 + 超时(120s) + 输出上限(1MiB，stdout+stderr 合计，1048576 完整返回/1048577 截断且只标记一次)/并发上限 4（第 1~4 个成功、第 5 个拒绝）+ 服务端按子命令构造 argv + 词法校验 + 可信 PATH + git 配置隔离（GIT_CONFIG_*=/dev/null + NOSYSTEM=1 + COUNT=0 + 净化 gitdir/config 视图禁用 fsmonitor/helper + 子进程 clearenv + 环境 allowlist）+ auto 模式 bubblewrap 只读隔离（bwrap 不可用则 auto 拒绝、supervised 非隔离并记录）+ grammar 对 auto/supervised 均强制（模板外一律拒绝，不因审批放宽）；fixture 证明需 TTY 才引入 pty 并记审计）：create→输出→wait→release 全链路、重复 kill/release 幂等、未知 ID 错误、会话取消进程组清理、execution event 审计、超时后进程组终止且 RPC 队列继续服务。验证：fixture 集成测试（含 option-smuggling、`cat /etc/passwd`、`git -C ~/.ssh`、`git diff --output=x`、`find . -exec sh`、`sed -n -i '1p'`、`sed -n '1e...'`、`rg --pre`、`rg --follow`、`grep -R`、`find -L`、`>` 写文件、supervised 下模板外命令仍拒绝、bwrap 缺失时 auto 拒绝、bwrap 可用时根外不可见/授权根不可写/网络隔离、cwd 验证后被替换为 symlink 仍锚定 FD、本地 core.fsmonitor 探针不启动）+ 真机 bash/grep 成功记录（无 "ACP terminal capability is unavailable"）。
- [ ] 5.3 fs 实现（授权根 canonicalize + no-follow、新建父目录校验、越界/symlink 拒绝、UTF-8 错误、权限拒绝）。验证：单测覆盖正常读写与全部拒绝用例。
- [ ] 5.4 权限与角色 policy 接入（supervised 走 ApprovalBridge；reviewer 默认拒 terminal/fs 写；coding 限目标 worktree）。验证：单测覆盖三角色行为。

## 6. P2b kimi MCP 受控注入

- [ ] 6.1 envelope bundle 派生 `mcpServers`（allowlist/digest/脱敏/argv 审计；session/load digest 不一致时拒绝 + 启动新会话 + 旧会话 superseded；未配置保持 `[]`）。验证：单测覆盖注入、resume 漂移（含新会话 + superseded 审计）、无 bundle 行为 + 真机 CodeGraph 可调用记录（若环境可用）。

## 7. 全量验证与 gate campaign

- [ ] 7.1 四件套 + `openspec validate harden-story-pipeline-weak-models --strict` + `cd web && pnpm test && pnpm tsc -b` 全绿。
- [ ] 7.2 gate campaign：每个受支持 provider×strategy 跑 20 个唯一 `case_id` 的 baseline/revised 配对（gate 前补齐缺失的 matching baseline，基线 1.2 的 5 样本可按 case_id 复用但不重复计数）；manifest 校验器落盘为 `cadence/reports/story-weak-model-campaign/validate_manifest.py` + `manifest.schema.json`，按 `(provider, model/version, strategy, case_id, run_kind)` 去重，拒绝缺边/重复 pair/版本或 strategy 或 round 不一致/retry 入分母/缺失或零 input-token usage/缺非 retry 配对（整组剔除而非择优取样）。输出：三口径成功率、retry 分布、失败分类、usage 对比（fresh/resume 分列，均值 ≤0.60 为 gate）、golden 规范化 diff。验证：`python3 validate_manifest.py` 对最终 manifest 通过；任一组合 full-chain 一次成功 <19/20、token gate 未达、或 golden diff 出现禁止差异/golden digest 不匹配时本任务不得勾选。
