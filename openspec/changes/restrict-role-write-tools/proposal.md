# Proposal: restrict-role-write-tools

## Why

会话角色决定其职责边界:计划作者与评审者产出计划/findings,**不应**绕过「计划→门→编码」流程直接改文件;编码者写代码刚需全工具。但当前工具面不受角色约束(2026-09-04 摸底+scout 核实):

- **pi**:启动 argv 无任何工具限制——作者/评审拥有全工具(含 `edit`/`write`)。
- **claude code**:argv 无工具限制参数;角色默认权限模式 Auto,ApprovalBridge 全量自动放行。
- **codex**:`sandbox: "danger-full-access"` 硬编码于 thread/start 与 thread/resume——全角色全访问、无沙箱。
- **kimi**:client services 已按角色硬拒(Orchestrator FsWrite 无条件拒;Reviewer 只读;WorkItemSplitter 全拒),是目标语义的既有范本,无需改动。

durable 实证(作者仅用 `read`+`bash` 零 `edit`/`write`;coding Coder 写工具刚需;评审产出 findings)表明按角色收窄写面对现有行为零回退。3.6 收敛轮测试矩阵前统一关闭非编码角色的写越权面。

## What Changes

> 术语:SC=SingleCandidate(单候选)。

**主 spec 修订(用户裁决 2026-09-04 放本 change)**:本 change 的 spec delta 同时 MODIFIED 四条既有 requirement,正式解决存量规范冲突——REQ-ENV-01/02 增补「legacy 直连显式过渡例外」(现有 author/revision/review 直连入口保留,替代约束=builder 工厂策略+adapter 双向守卫+durable 审计;全量 gateway 迁移另列路线);REQ-ENV-06/08 增补「自发现通道豁免」(项目 `.mcp.json`/`.kimi-code`/`.codex` 等 provider 原生配置发现=用户裁决的受信任通道,不受 Aria bundle 管控;Aria 主动注入场景管控维持)。

- 新增**角色×工具黑名单**统一策略(宿主层单一策略源):`StreamingProviderInput.tool_policy: Option<ProviderToolPolicy>`(+审计 sink 同源字段),承载语义意图;**保护范围=built-in 工具面**,bash/terminal 与自发现 MCP 为显式信任逃逸面(MCP 工具可能具备写能力,经 REQ-ENV-06 例外由用户裁决信任);本期矩阵:
  - **作者家族**(SC 首轮 author/author feedback revision=Orchestrator;WorkItemPlan author 普通/fresh/resume/serial/batch=WorkItemSplitter)→ `DenyFileWriteBuiltins`
  - **评审家族**(SC reviewer/review repair、coding CodeReviewer/InternalReviewer、group review=Reviewer)→ `DenyFileWriteBuiltins`
  - **编码**(coding Coder=Executor)→ 无策略(黑名单为空)
  - Handoff 无真实入口,不适用
- 各 provider translator 把意图翻译为 canonical 物理片段并在启动时过滤:
  - pi:argv 注入 `--exclude-tools edit,write`;
  - claude code:argv 注入 `--disallowedTools Edit,Write,NotebookEdit`(名单冻结,实施首步 CLI 实测门禁复核);
  - codex:作者/评审会话 thread/start 与 thread/resume `sandbox=read-only`+`approvalPolicy=on-request` 并联动**审批分类**(MCP 工具审批自动批准;命令执行升级/文件写/未知审批拒绝并审计,fail-closed);Coder 维持现状;gateway-mediated codex 启动维持 REQ-ENV-05 路由阻断现状(见 design 边界);
  - kimi:零改动(client services 既有角色策略已是目标语义且更严;不收紧也不放宽)。
- **非写工具面不受本策略影响**(否定式约束):读取、bash/terminal 只读命令(codex read-only 下免审批直行)、各 provider **自发现**的 MCP/extension 工具、`ask_user` 提问通道。本 change 不做 MCP 注入/管理,也不对 MCP 可用性作正向保证(MCP 可用性取决于 provider 自发现的项目配置,见 design「REQ-ENV-06 边界」)。
- adapter 层 fail-closed 守卫(双向):作者/评审角色启动必带有效策略(缺失/非法/非支持意图 → spawn 前拒绝);Executor/Handoff 携带策略 → 拒绝(防误标)。
- **codex 审批分类**(全角色统一):parse 层识别 `item/commandExecution/requestApproval`/`item/fileChange/requestApproval`/`mcpServer/elicitation/request`(`_meta.codex_approval_kind="mcp_tool_call"`)三类+未知形态;分类决策:MCP=**所有会话**(含 Coder)accept+审计(受信任逃逸面,自动批准≠写安全保证);commandExecution/fileChange=策略会话 decline+审计、**Coder 维持 ApprovalBridge 既有上抛链**;未知形态=**所有会话**确定性拒绝应答(elicitation→JSON-RPC error -32601;item→decline;连续≥3 终止)+上报(由现状的静默不应答改进而来,fail-closed 优先于挂死,回归锁定)。
- **冻结 digest 与 resume 一致性**:启动审计记录持久化(tool_policy canonical digest+provider_session_id+version/dialect 等);resume 启动 spawn 前比对,不一致/缺失 → 拒绝 resume、superseded、新建(REQ-ENV-04 语义)。
- **durable 启动审计**:LifecycleStore 新增 tool-policy-run-audit JSONL 分区;provider 启动成功后追加,追加失败 ⇒ 立即终止会话并将 run 判失败;不声称 pre-spawn 审计。
- 保留(不动):permission 映射(含 Pi→Auto)/ApprovalBridge 语义/gateway 路由策略/REQ-ENV-05/GatewayRunAudit/driver choice 兜底。
- 边界声明:工具面收窄(软约束),非只读安全边界——bash/terminal 保留写侧逃逸;真只读不在本 change。

## Capabilities

### New Capabilities

(无)

### Modified Capabilities

- `session-policy-envelope`: 新增 requirement(REQ-ENV-09)——非编码角色会话必须携带经校验的文件写拒绝策略(角色×黑名单矩阵、provider 翻译、codex 审批分类、双向 fail-closed 守卫、冻结 digest resume 比对、durable 启动/审批审计、kimi 零改动与非作者路径边界)。

## Impact

- `src/cross_cutting/streaming_provider`:`StreamingProviderInput` 新增策略字段与语义类型(全部构造点受影响,非策略路径 None)。
- `src/cross_cutting/pi_provider` / `claude_code_provider`:argv enforcement+spawn 前守卫+审计 payload。
- `src/cross_cutting/codex_provider`:沙箱/审批三联动(thread/start+resume 两路)、审批分类与即时决策、request id 空间隔离、审计 payload。
- `src/product/workspace_engine`+`coding_workspace_engine`:作者/评审家族全部 builder 按矩阵注入策略。
- `src/product/lifecycle_store`:tool-policy-run-audit JSONL 分区(owner/key/schema 见 design)。
- `src/cross_cutting/kimi_code_provider`:零改动,仅回归断言。
- 测试:入口×provider×role×route 表驱动矩阵、argv/沙箱/审批分类单测、digest 漂移/记录缺失、非策略路径/kimi/正向可用性断言、审计失败注入。
- 运行验证:全量部署(md5 级)后 pi×轻 3 连跑+审批分类真实验证;claude/codex 随 3.6 矩阵对应格首跑。
- 非目标:不做逐工具审批门;不动 permission 映射/gateway/REQ-ENV-05/GatewayRunAudit;不做 MCP 注入通道;不覆盖同步 AdapterInput 死路径(defer,重激活前置=策略绑定);不修 logical WorkItemPlan gateway 硬编码 ClaudeCode(另账);不实现真只读(defer)。
