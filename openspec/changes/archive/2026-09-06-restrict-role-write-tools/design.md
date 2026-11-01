# Design: restrict-role-write-tools

## Context

- 越权面现状(2026-09-04 摸底+scout 核实 `f3-scout-verify.md`):pi 无工具限制;claude 无限制+bridge Auto 放行(`claude_code_provider/mod.rs build_args`、`approval_bridge/mod.rs:94`);codex `CODEX_DEFAULT_SANDBOX_MODE="danger-full-access"` 硬编码(`codex_provider/mod.rs:35`,thread/start+resume 两路);kimi client services 已是目标语义(`client_services/policy.rs`)。
- 启动链路形态(scout):SC author/revision=直连 `provider.start`;SC reviewer legacy=直连,logical=经 gateway(`review/drive.rs:891-916`,ProviderRef 固定 Claude);WorkItemPlan author 全形态=StreamingProviderInput(legacy 直连或 gateway→`adapter.start`);coding coder/code reviewer/internal reviewer=engine builders;同步 `AdapterInput` 路径无生产调用方(defer)。**enforcement 落 adapter 层 ⇒ 直连与 gateway 传递两路同受保护;gateway 自身(路由阻断/内存审计)不动。**
- CLI 杠杆(本机核实+spike):pi `--exclude-tools`;claude `--disallowedTools`(支持模式);codex `-s sandbox`(read-only/workspace-write/danger-full-access)。
- CO-3 审批 spike(`f3-co3-codex-approval-spike.md`):codex 审批=授权逃逸沙箱(批准后文件落盘);三类审批 wire 互斥可判别——`item/commandExecution/requestApproval`/`item/fileChange/requestApproval`/`mcpServer/elicitation/request`+`_meta.codex_approval_kind="mcp_tool_call"`(🔴 必须看 `_meta`,方法名是通用通道);read-only 下只读命令零审批直行;MCP 批准后调用成功;`reason` 字段为自然语言不可作判别;server→client request id 从 0 起与 client id 空间重叠。
- 仓库审批链路(`f3-co3-recon-supervised.md`):codex adapter 现仅识别 commandExecution 两代→`bridge.request_tool(High)`→accept/decline;approvalPolicy on-request 接线已在;MCP/fileChange 现不识别不应答(挂死风险);claude 有「特殊交互工具分类」先例(AskUserQuestion→Choice)。
- MCP 生态(`f3-scout-mcp.md`/`f3-mcp-spike-codex-kimi.md`):pi/claude=项目 `.mcp.json` 自发现;kimi=原生认项目 `.mcp.json`(最近 .git 根)/`.kimi-code/mcp.json`+ACP 自动加载+一次性交互 Trust 门控;codex=全局+项目级 `.codex/config.toml` 自发现。四 provider MCP 可用性均不依赖 Aria 注入。
- 裁决链:用户否决 allowlist→四 provider 统一→R1 审查(5C/6I/3M)→scout 核实→oracle D1/D2/D3+A 条件→CO-3 调查(可做,须精确分类)→R2 审查(4C/8I/2M)→**用户定案:全 provider 统一「角色×工具黑名单」,评审范围 A(coding 评审也收),Coder 黑名单空**→R3。

## Goals / Non-Goals

**Goals:**

- 角色×黑名单矩阵(见 D2):非编码角色文件写不可达;Coder 全工具;**对 pi/claude/codex,黑名单之外既有非写工具面不变;kimi 维持现状角色策略(不提升不收紧)**(kimi WorkItemSplitter 全拒/Reviewer 无 terminal 为既有更严形态,属 provider 现状,非本 change 保证项)。
- codex 审批分类:非编码角色 MCP 可用(自动批准),写/命令升级/未知拒绝;fail-closed。
- 双向守卫+冻结 digest resume 比对+durable 启动/审批审计。

**Non-Goals:**

- 不改 permission 映射/ApprovalBridge 语义/gateway 路由/REQ-ENV-05/GatewayRunAudit。
- **不动启动拓扑**:REQ-ENV-01/02(逻辑代码库流程 gateway 入口+禁止裸 input fallback)维持现状;`tool_policy` 是 StreamingProviderInput 的组成部分(builder 工厂设置,随 input 到达 adapter,直连或经 gateway 传递同构),不构成新的无政策 fallback 通道,adapter 守卫为第二道防线;非逻辑/legacy 流程的既有直连拓扑不变。
- 不做 MCP 注入/管理;不对 MCP 可用性作正向保证(否定式约束:本策略不禁用非写工具);provider 自发现通道(项目 `.mcp.json`/`.codex/config.toml`)的配置管控不在范围(用户裁决 2026-09-04:Aria 不管 MCP 使用),其配置来源隔离弱化登记为已知 gap(REQ-ENV-06/08 的 bundle 注入要求维持不变,kimi 生产 `mcpServers: []` 与「无 bundle 时为空数组」一致)。
- 不覆盖同步 AdapterInput 死路径;不修 gateway 硬编码 ClaudeCode(另账);不做逐工具审批门;不实现真只读(defer)。

## Decisions

### D1: 语义层 ToolPolicyIntent + provider translator + canonical digest 规范

`ProviderToolPolicy` 承载 `DenyFileWriteBuiltins`(本期唯一合法意图,其他枚举构造即拒)。translator 映射:pi=`--exclude-tools edit,write`;claude=`--disallowedTools Edit,Write,NotebookEdit`(冻结,task 0.1 实测门禁复核,不符→change 停止上报);codex=三联动片段(`sandbox=read-only`+`approvalPolicy=on-request`+审批分类策略)。
**canonical digest 规范(冻结,R3 I-R3-5 补)**:sha256;输入=`"tp-v1"(schema_version)`+`\x1f`+provider 名+`\x1f`+canonical 片段 token 序列(argv flag 与 value 按出现顺序原样、大小写保留、以 `\x1f` 连接)+`\x1f`+审批分类规则版本常量 `"ap-v1"`(codex 三联动片段的行为组成部分;规则集变更必须升版本常量);digest 语义=物理片段或审批规则变化必然改变 digest。实施以单测钉死向量。

### D2: 角色×策略矩阵与入口×route 全表

| 入口 | builder(锚点) | role | 策略 | route |
|---|---|---|---|---|
| Story/Design/SC(SingleCandidate)首轮 author(fresh/resume;三链共用 builder) | `workspace_engine/prompts.rs build_streaming_input` | Orchestrator | 必带 DenyFileWriteBuiltins | 直连 `provider.start` |
| author feedback revision | `prompts/revision.rs` | Orchestrator | 必带 | 直连 |
| WorkItemPlan author 普通/fresh/resume/with-session | `prompts.rs build_work_item_plan_streaming_input*` | WorkItemSplitter | 必带 | legacy 直连/logical 经 gateway→adapter.start |
| WorkItemPlan serial/batch draft | `draft_batch/runs.rs`(调上者) | WorkItemSplitter | 必带 | 同上 |
| Story/Design/SC reviewer / review repair | `prompts/review.rs`/`review_repair.rs`(三链共用) | Reviewer | 必带 | legacy 直连/logical 经 gateway(ProviderRef 固定 Claude,不变) |
| coding CodeReviewer / group review | `coding_workspace_engine` review builders | Reviewer | 必带 | 直连/经 gateway 均到 adapter |
| coding InternalReviewer | `internal_pr_review.rs` | Reviewer | 必带 | 直连 |
| coding Coder | coding builders | Executor | 禁带(None) | 直连/gateway |
| Handoff | 无真实 builder | Handoff | 禁带(None) | — |
| 聚合初始化 3 provider turns | `coordinator_provider_turn.inc.rs`(gateway validated) | Executor | 禁带(None;需写配置文件,正确) | 经 gateway |
| **例外格:codex×gateway-mediated** | — | — | 不适用(REQ-ENV-05 路由阻断,预期拒绝) | gateway 阻断维持 |
| coding Coder 直连 | coding builders | Executor | 禁带(None) | legacy 直连例外(流式栈) |
| coding CodeReviewer/InternalReviewer/group review 直连 | coding review builders | Reviewer | 必带 | legacy 直连例外(流式栈) |
| 同步 AdapterInput(WorkItemSplitEngine) | — | WorkItemSplitter | 不适用(逻辑仓 fail-closed;非逻辑无生产调用方,重激活前置=策略绑定) | 不属例外,维持 REQ-ENV-01 原文 |

**gateway-mediated codex 边界(R2 C-R2-1 定案)**:gateway 路由阻断 codex(REQ-ENV-05)维持现状——gateway 上的 codex 启动预期被路由拒绝,不进入 read-only adapter 分支;codex 三联动仅适用于直连 adapter 启动(今天的作者/评审主链即直连)。解除 gateway 阻断=REQ-ENV-05 演进,独立 change。

### D3: adapter 双向守卫

pi/claude/codex adapter:`role ∈ {Orchestrator, WorkItemSplitter, Reviewer}` 且策略缺失/空/非支持意图 → spawn 前拒绝;`role ∈ {Executor, Handoff}` 携带策略 → 拒绝。**role 可信来源=engine builder 工厂唯一设置**(非用户/模型输入;威胁模型=编程错误非对抗):配套约束=新增角色入口必须走 builder 工厂,禁止绕过工厂裸构造 `StreamingProviderInput` 直启 provider(测试断言工厂全集+guard 全枚举覆盖)。

### D4: 物理 enforcement

- pi:`build_args` 追加 `--exclude-tools edit,write`(`--session-id` 位置逻辑不变)。
- claude:`build_args` 追加 `--disallowedTools Edit,Write,NotebookEdit`。
- codex:策略会话(作者/评审)thread/start 与 thread/resume 两路 `sandbox="read-only"`+`approvalPolicy="on-request"`(由 ToolPolicy 触发,不看 permission_mode);Coder 维持 `danger-full-access`+permission_mode 映射。

### D5: codex 审批分类(全角色统一,adapter 本地即时决策,bridge 不动)

parse 层识别三类+未知:
- `mcp_tool_call`(经 `_meta.codex_approval_kind`,携带 serverName/tool_params)→ **所有会话**(策略会话与 Coder)accept+审计——MCP 属自发现配置的**受信任逃逸面**(可能具备写能力;自动批准≠写安全保证,由 REQ-ENV-06 例外条款的用户裁决信任背书),全角色一致。
- `commandExecution` → 策略会话 decline+审计;Coder 会话维持现状路径(commandExecution→bridge 既有链)。
- `fileChange` → 策略会话 decline+审计;**Coder 会话同样走 bridge 既有上抛链**(R3 I-R3-2 定案:danger-full-access+never 下预期不到达;若到达,由既有审批链决策,不一律拒,消除与 Coder 全工具矛盾;task 0.2 补真实写路径验证)。
- **未知形态确定性应答表(R3 I-R3-3 冻结)**:①elicitation 形态(method=`mcpServer/elicitation/request` 但 `_meta.codex_approval_kind` 缺失/未知)→JSON-RPC error(code=`-32601`,data={"codex_approval_kind":"<原文>","reason":"unsupported_approval_kind"});②item 审批形态(item/*/requestApproval 且不在已知集)→`{"decision":"decline"}`;③同会话连续 ≥3 次未知形态→终止会话(reason_code=`unknown_approval_storm`);每类未知 method 配 fixture 单测。
- **未知形态 fail-closed 应答规范(R2 I-R2-2 定案)**:elicitation 形态→按其协议回 JSON-RPC error(合法协议动作,不会挂死)+上报事件;approval item 形态→decline 应答;无法确定应答 schema→JSON-RPC error+上报;连续未知形态异常→终止会话(防挂死)。每类未知 method 配 fixture 单测。
- 决策即时应答无 pending 生命周期,不经 `bridge.request_tool`;**request id 隔离机制(冻结)**:client 侧出站 request id 统一字符串前缀 `aria-<seq>`(typed namespace),server→client 入站 id 保持原生数字;fixture 覆盖 server id `0` 与 client id `aria-0` 共存不冲突。
- **行为改进声明(R2 I-R2-1 定案,R6-01 修订)**:策略会话与全角色的 MCP/fileChange/未知审批从现状「静默不应答(挂死风险)」改为「协议合法应答+上报」,处置逐类与 proposal/spec 逐字一致——MCP=accept+审计(所有会话含 Coder);fileChange=策略会话 decline+审计、**Coder 走 bridge 既有上抛链**(不一律拒);未知=拒绝应答+`protocol_warning`;commandExecution 不属本改进面(策略会话 decline+审计为新增决策,Coder 维持 bridge 既有链)——显式接受的非零变化,回归测试锁定新行为。

### D6: 冻结记录与 resume 比对(R2 C-R2-2 定案)

启动审计记录(D7)持久化字段含:`workspace_session_id`、`provider_session_id`、`tool_policy_canonical_digest`、`provider_version`、`adapter_dialect`、provider 名、role、最终片段。**version/dialect 来源(R3 I-R3-7 冻结)**:version=每次策略会话启动时 adapter 执行 CLI `--version` 探测(进程内缓存,有界超时;不可得→策略会话启动 fail-closed);dialect=adapter 代码常量(如 `codex-app-server-rpc`/`claude-stream-json`/`pi-rpc`);比较=精确字符串相等。resume(携带 resume_provider_session_id 的策略会话)在 spawn 前:在该 workspace 会话审计分区内按 provider_session_id 检索最近 `provider_start`,比对三元组;任一不一致或记录缺失 → 拒绝 resume、标记 superseded、新建会话(REQ-ENV-04 语义;F3 部署前存量会话=缺失→supersede+新建);version/dialect 漂移各配 fixture 单测。gateway envelope 的 aggregate policy digest/capability snapshot 维度属 gateway resume 机制,不在直连路径,维持现状不变;`tool_policy_canonical_digest` 为独立字段,不与 aggregate policy digest 混用。

### D7: durable 启动/审批审计(R2 C-R2-4/I-R2-3 定案)

- **owner=LifecycleStore**;新增分区 `tool-policy-run-audit/`;**文件粒度 key=(workspace_session_id, role_run_seq)**;**行粒度**:文件内 JSONL 逐行 append,行 seq 单调递增,`provider_start` 恰为首行且每文件仅一条(重复追加 provider_start 到同文件=错误传播),`approval_decision` 任意多条跟随;schema_version=1;append-only。
- **canonical 事件四类**(durable_tool_policy_audit):`provider_start`(恰为首行一条;D6 字段+最终 argv/沙箱+审批参数原文)、`approval_decision`(category/server_name/tool_name/request_id/decision/reason_code/policy_digest)、`protocol_warning`(未知审批形态/协议异常;读取端坏行产生的告警不入本分区,仅作读取结果日志)、`session_terminated`(reason_code 如 unknown_approval_storm)。术语:策略会话=durable_tool_policy_audit;Coder/非策略/kimi=execution_event_audit(既有通道)。
- 时序:provider 启动成功后追加 `provider_start`;**追加失败 ⇒ adapter 返回错误,engine 沿既有 provider task kill 链终止会话并将 run 判失败**(不吞、不只记日志);不声称 pre-spawn 审计。崩溃恢复/坏行:沿 role_run_event JSONL 既有语义(坏行错误传播,读取端跳过带告警)。
- **实现接点契约(R3 C-R3-3 冻结)**:①sink 注入=engine 构造 `ToolPolicyAuditSink`(trait:`append(workspace_session_id, role_run_seq, event)->Result`,内部互斥串行化),经 `StreamingProviderInput.audit_sink: Option<Arc<dyn ToolPolicyAuditSink>>` 传入(与 tool_policy 同源同在,非策略会话=None);②`role_run_seq`=engine 按 provider run 分配,随 run 记录持久化(lifecycle 既有 run 计数,跨进程单调);③`provider_session_id`=adapter 在 `start` 返回前完成握手确认(pi=预生成/传入,claude=resume 已知·fresh 等首个 init 事件有界超时,codex=thread/start 应答;确认失败→启动 fail-closed),`provider_start` 于 start 返回前写入;④kill owner=provider_start 写失败→adapter 在返回前终止子进程并返回错误(start 前);approval_decision 等后续事件写失败→engine provider task 既有 kill 链(直连/gateway 一致);⑤幂等=每次启动新 role_run_seq 新文件,provider_start 每文件恰一条,无跨文件去重需求;⑥resume 检索=当前 workspace 会话分区内顺序扫描(单写者,无并发);⑦坏行语义统一=写入失败错误传播/读取坏行跳过+protocol_warning。
- pi/claude/codex 三 adapter 接同一审计协议;kimi 不接(现状断言);Coder/非策略会话审批决策走既有 execution event 通道(非 durable 分区,R3 I-R3-6 定案);GatewayRunAudit(内存)不动。

### D8: 测试策略(TDD)

①D2 全表逐格断言(builder×role×策略×route 可测部分)+guard 全 AdapterRole 枚举;②三 adapter argv/沙箱单测(策略形态、resume 组合、空 session id、缺失/非法 fail-closed、codex start/resume 两路);③审批分类单测(三类 fixture+每类未知 method fixture+Coder/策略会话分流+Auto/Supervised 零变化回归+连续未知终止);④digest 向量单测+漂移/缺失 resume 单测(含 version/dialect 变化);⑤审计失败注入→会话终止+run 失败;⑥非策略路径零变化+pi→Auto 断言+kimi 现状断言(Orchestrator/WorkItemSplitter/Reviewer/Executor 表);⑦正向可用性断言:denylist 仅含写工具,extension/MCP/ask_user 不在排除面;⑧禁止裸构造直启的工厂全集断言。

## Risks / Trade-offs

- [coding 评审链收写面] 评审范围 A 触及刚达成完整案例的 coding 链(CodeReviewer/InternalReviewer/group review);评审者按 durable 证据从未写文件,预期零回退;矩阵 coding 格重点观察。
- [codex on-request 审批等待] 只读命令免审批;到达审批=写/MCP/逃逸请求,策略会话拒绝后模型自行改道;矩阵 codex 格观察首稿质量。
- [claude 名单/大小写] 冻结+task 0.1 实测门禁,不符→change 停止。
- [kimi Trust 门控] 用户侧一次性动作,进 4.2 checklist;未信任→MCP 静默缺失(既有行为)。
- [F3 部署切面] 存量策略会话 resume 全部 supersede+新建(短时会话,影响可忽略)。
- [bash 保留写侧逃逸] 软约束边界声明,真只读 defer。

## Migration Plan

1. task 0 三项实测门禁(0.1 claude denylist 复核/0.2 codex read-only+on-request+MCP 审批全链 preflight/0.3 pi `--exclude-tools`×`--session-id` 组合复核),各自独立记录版本/命令/结果,任一不符→change 停止上报。
2. 全量门禁四条标准命令绿→部署 4317(md5 级,PID/exe/md5 三对账)。
3. pi×轻 3 连跑+审批分类真实验证;claude/codex 随 3.6 矩阵对应格首跑。
4. 回滚:revert 单 commit 系列重建部署;无数据迁移。

## Open Questions

(无)
