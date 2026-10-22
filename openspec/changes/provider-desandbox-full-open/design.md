# 设计：四 Provider 全开契约迁移

## Context

动机与范围见 proposal.md。owner 的最终产品决策以 `cadence/notes/2026-10-17_总览-全流程测试决策与排期.md:18-35,60-67` 为准；去沙箱设计的全角色与 Codex v2 尾修覆盖旧的规划隔离、网络参数化与受限档正文。本 change 只形成契约工件，不修改实现、不启动 apply、不归档其它在途 change。

已读取并定点复核：

| 来源 | 已观察事实 | 契约影响 |
|---|---|---|
| `src/product/logical_codebase/provider_gateway.rs:1312-1327`，`validate_inner` | fresh 先 require_supported，再 require_write_boundary，再执行 Codex 路由硬门 | 只删 bwrap 或放开 adapter 不能打开 LC；准入必须同步换证据语义 |
| 同文件 `:2301-2326`，spawn 复验 | Normal fresh/resume 均再查 write_boundary，resume 追加自身能力；RootRecipe 有单独凭据分支 | 新合同必须覆盖 validate 与 spawn 两处，不误删原生恢复或 root-recipe 凭据复验 |
| 原 `lc-gateway-multi-provider` change 的网关 delta | 十四项已勾任务具有源码/既有真实矩阵证据；strict 格式问题由 controller 修复，存在下述 DEV-01 有意迁移桥偏差 | controller 先归档产生真实主规范，本 change 网关只用 MODIFIED，不保留双模式 ADDED |
| `openspec/specs/session-policy-envelope/spec.md:9-70,109-140` | ENV-01/02 将 Reviewer 绑定 deny，ENV-03 用只读命名，ENV-05 拒 Codex 全开，ENV-09 连同 Direct 与 Kimi 冻结旧限制 | 不只改 ENV-09，所有直接冲突的启动/审批场景一并更新 |
| `openspec/specs/logical-codebase-aggregate-planning/spec.md:8-17,50-55` | PLN-01 的场景也承诺只读，PLN-06 明确要求只读证明 | 两条一起替换，不留下相反验收条件 |
| `openspec/specs/kimi-acp-client-services/spec.md:37-150` | 三条沙箱合同实际在此，另有独立 120 秒生命周期 | 正确 capability 真实改四条；集成面另作三条接线配套 |
| `openspec/specs/kimi-code-provider-integration/spec.md:30-83` | ACP 会话、默认权限及逐工具审批三条是宿主能力的集成入口合同 | 明确统一执行语义而保留 ACP、审批、提问与取消所有权 |
| `openspec/specs/coding-provider-transport-retry/spec.md:70-82` | Reviewer 技术重试仍要求新只读会话 | 改为先收口再开新 B 档窗口，预算不变 |

设计文档由另一工作面修订，物理锚点仍按本次读取记录；本 change 不写 `cadence/designs/` 或 `cadence/notes/`。角色×profile 权威矩阵与 B 档状态转移归现行设计，而不是在 spec 重复一套。

## Goals / Non-Goals

**目标**：

- 五处契约变更可逐项验收，附带 HostCoding CHECK、B 档三要素、无条件鉴权三条不可遗漏的交付红线。
- 保留现有唯一 authority、native identity、canonical cwd、完整 fingerprint、真实能力证据、审批及失败关闭结构，替换旧权限目标而非删除所有保护。
- 让后续 Plan 明确区分“宿主能力全开”“作者岗位限制”“Reviewer 报告是否可采信”三件事，避免把恢复成功或工具禁用当成审查安全结论。

**非目标**：

- 不重裁 owner 决定，不设计独立 UID、cgroup、应用层 bwrap 替代品，不恢复已取消的 Group Reviewer Provider。
- 不在 spec 固化角色×profile 表、状态枚举、目录格式、令牌载体与重试实现步骤；这些由修订后的既有设计与后续实施 Plan 展开。
- 不把二期容器或触发清单变为自动升级机制，不新增 CHECK canonical schema、通用命令 runner 或备用 provider。

## Decisions

### 1. 复用原合同边界，不创建平行权限体系

沿用 `session-policy-envelope` 持有执行/岗位/窗口需求，`lc-gateway-multi-provider` 持有真实能力与两阶段复验，`kimi-acp-client-services` 持有实际 terminal/fs 行为；平台鉴权形成长期 `platform-api-authentication` capability。原网关基线必须先由 controller 归档到主规范；本 change 对真实主规范使用 MODIFIED，不以 ADDED 加覆盖注释绕开依赖。

替代方案：把内容塞入新 provider capability 会留下旧合同并存；在基线未建立时保留 ADDED 并寄望未来归档顺序会重复创建，均不采用。仅改少数主条款会漏 ENV-01/02/05、PLN-01 与重试路径，也不采用。

### 2. write_boundary 保留证据门，显式迁移语义

选择：保留现有 action 矩阵与 Confirmed/Denied/Unknown，给执行模型/证据语义明确版本；write_boundary 的当前 Confirmed 改为“已真实验证宿主全开执行及岗位/窗口合同”，不再证明 root/其它成员/.git/.aria 的物理拒写。validate 与 spawn 两处都消费同一新语义，旧隔离证据不自动升级；历史记录保留为历史，不改写其过去含义。

直接删 require_write_boundary 会丢失执行模型证据与 spawn 复验；复用旧 Confirmed 或保留 Unknown+旧 supported_actions 放行则绕过新证据，均不采用。新合同显式废止迁移默认 allow，仅当前语义 Confirmed 放行，legacy 必须重探针铸证。DEV-01 的拆桥属于后续实施，不倒逼契约前置期修改源码；具体字段命名由 Plan 展开。

RootRecipe 在所读代码中有独立能力凭据分支；保留该分支的 Running 凭据复验，不把普通 action 矩阵误套根初始化，不扩大 provider 功能支持矩阵。

### 3. 岗位工具与原生 sandbox/approval 解耦

作者岗位规则共同目标仍是不编辑代码文件；Claude/Pi 使用工具过滤，Kimi 使用 client-service policy。**Codex danger 下没有预防能力保证**，采用经 controller/oracle 确认的检测型纪律：fileChange 事件→中断 turn→本轮 fail-closed→durable 审计，不写“Decline 强制”或“工具物理不可达”。LC 全相位 danger 为明示终裁；Direct 同步全开是“诉求 1：四家全角色统一”的推论，不冒充锚点逐字说过 Direct。approval never/on-request 独立，Reviewer 不继承旧合并 deny。

后续实施必须固定真实 CLI 版本，对 danger+apply_patch 分别在 on-request/never 下观察审批出现/未出现、fileChange、实际变化、中断及审计；审批未出现不能宣称被拒成功。若事件不可观测或中断不可靠，检测型岗位机制不通过。该检测不回滚已发生写入、不阻断其它宿主通道，不能借能力缺口退 read-only 或清空岗位纪律。

### 4. B 档只固化可采信条件，不把状态机塞进规范

采用 owner 决定的快照恢复、平台通道锁、对账违规整轮作废；没有独立 UID，不承诺防同 UID 恶意篡改。spec 要求锁/停止 writer 早于基线与材料、恢复前违规持久化、三域分开判定、恢复成功不洗白、违规轮只能新轮重审；具体状态与人工操作沿现行 B 档设计。

平台 Git 操作统一禁止仓库 hooks/fsmonitor 桥，不能只改一个入口；资源登记先于执行，取消/崩溃须先静止再清理。清理仅限运行前创建且登记的私有目录，不能按 HOME/XDG 递归删除。替代的“只禁 reviewer edit/write”无法覆盖 terminal 自提交，不采用。同批删除 ProcessManager 的 `env_clear` 后，`git_finalize` 的进程环境契约同步改为宿主继承加 `LC_ALL`/`HOME`/`SSH_AUTH_SOCK` 显式覆盖层（`repository-initialization-progress` delta），不再声称其余变量隔离。

### 5. Kimi 三条合同以真实载体为准，集成面配套三条

去沙箱设计的三条是 `kimi-acp-client-services` 的 OS 隔离、封闭 grammar、权限与 fs；原题保留作 MODIFIED 锚但正文明确取代旧语义。另改生命周期解除独立 120 秒硬上限，活动 terminal 同时进入 ACP stall 与产品 idle 判定。`kimi-code-provider-integration` 的 ACP 流式会话、权限默认、逐工具审批三条则补主会话与子执行一致性，保留会话 ID、用户拒绝、AskUserQuestion 与取消/失败语义。不能只写集成声明而不改实际沙箱合同。

### 6. 鉴权先行，CHECK 是 Coder 主路径

API 身份认证、Origin/Host、WS 与敏感 GET 覆盖是启用全开前的公共门，不等风险触发或二期。令牌不能进入 provider 环境、URL 或明文日志；同 UID 读取可读凭据后的取证调用不属于该边界承诺。Coder 在宿主 HostCoding 环境自行 CHECK；A′ 只补外部/人工/临时缺项，不新增 server runner，也不将环境声明变成权限授权。

### 7. 同 capability 冲突面与归档顺序

原始 `lc-gateway-multi-provider` 先由 controller 归档建立真实主规范，本 change 逐题 MODIFIED。未实施的 `fix-planning-session-anchor-recovery` 不作为本 change 的已完成基线；它排在去沙箱③之后，必须先按依赖门修订旧权限前提，再由自身工作流实施，不能用覆盖注释替代实际修订。

| 基线/在途条款 | 本 change 处理与依赖 | 禁止复活的旧语义 |
|---|---|---|
| 原网关显式映射（REQ-LCG-01） | 对真实主规范同题 MODIFIED，版本化证据并废止 DEV-01 桥 | Unknown+旧 allow 默认放行 |
| 原网关 admission（REQ-LCG-02） | 同题 MODIFIED，保留原 cwd/member target 与双阶段复验；不提前加入未完成 anchor 新身份规则 | 逻辑 roots 等于物理 target-only |
| 原网关 projection（REQ-LCG-03） | 同题 MODIFIED，改全开执行证据与岗位纪律 | 宿主写成功必判能力失败 |
| 原网关 Codex（REQ-LCG-04） | 同题 MODIFIED 为 LC 全相位全开，Direct 标诉求1推论 | 永久禁 danger、read-only/workspace-write 基线 |
| 原网关 resume（REQ-LCG-05） | 保留真实主规范原生同ID、指纹，加入新执行语义 | 旧隔离指纹与全开混比 |
| 原网关角色守卫（REQ-LCG-06） | 同题 MODIFIED 为 author 岗位（Codex 检测型）与 Reviewer 全开窗口 | Reviewer 必带 deny，Kimi 恒拒 |
| 原网关 E2E（REQ-LCG-07） | 同题 MODIFIED，真实全开探针与 B 档证据 | 普通宿主可写等于能力失败 |
| anchor-recovery 网关 admission（其 spec :7-25） | 最小修订 ReadOnly 名称不代表物理限制；保留其 native 聚合身份新需求供后续自身实施 | 新 target 能恢复旧受限档 |
| anchor-recovery projection（:27-41） | 先改 :29/:35-37 的 readonly/target-only Confirmed 前提；保留完整候选/基准/审计同源目标 | 旧负探针晚归档覆盖新合同 |
| anchor-recovery resume（:43-61） | 后续完整 fingerprint 包含新执行与 Codex 检测纪律，不复制为本 change 已交付要求 | 旧 full map 回填当前模型 |
| anchor-recovery 版本观测（:65-83） | 保留在其 change 内，依其工作流实施验收，本 change 不冒领 TTL 交付 | 提前同步尚未完成的合同 |
| anchor-recovery aggregate-planning header | 它改 PLN-03/05/07、加 PLN-08；本 change 的 planning delta 改 PLN-01/06 并新增 PLN-11/12/13，**无同名 header 重叠**；评审范围边界已在其 delta 内按最小修订补齐（PLN-03/05/07/08） | 把 ReadOnly 名称当物理边界 |

tasks 5.4 是本 change 的归档硬门：先交付 anchor 的依赖/权限最小修订与审阅，核对 Codex 投影与测试预期；5.5 是失效措辞门。评审范围与多成员对账域批次已按最小修订补齐 anchor 的 aggregate-planning 侧（PLN-03/05/07/08）与 `lc-root-initialization` 的 session-policy-envelope 侧（ENV-03/04/10/11），其网关侧的 Codex/角色/写边界冲突仍按 5.4 门在本 change 归档前完成，不能后补注释。逻辑排期为原网关基线归档→本契约及去沙箱③实施→anchor 按修订依赖实施。

### 8. plan 评审范围对齐 Issue 选集与多成员对账域（owner 已决，2026-10-22）

技术方案见 `cadence/designs/2026-10-22_技术方案_评审范围对齐Issue选集与多成员对账域_v1.0.md`，第十节 F 表是本次契约修订的逐条清单。本节只固定已拍板边界，不重开产品决策。

**范围与冻结**：plan 评审范围＝Issue 选集的有效成员全集，权威源是 `IssueCodebaseSelection` → 唯一 resolver → effective member ids，不从 Story/Design/WorkItem 的 involved/focus/change_order/target 收窄。范围在评审启动时冻结为 scope 快照（selection/set digest、membership revision、policy digest、有序成员映射），冻结早于任何材料采集与 provider 启动。空集、成员失效、checkout 不可解析、非 Git 工作树、身份解析不明与选集内 canonical common-dir 重复均以稳定错误码整轮 fail-closed，不回落 primary、不跳过成员。冻结后 selection/membership/checkout/root/policy 漂移即拒绝本轮采信并等待显式新轮。

**窗口承载**：采用路线 (ii)——每个选中成员复用一个单树评审窗口，新增 durable 集合屏障：集合账本先于首个成员窗口落盘，全窗先开且基线完整后才开始评审，整轮持续持锁、成员依次评审、末次统一收口对账与恢复。common-dir 互异是本路线的前提；重复 common-dir 以专用码整轮阻断。任一成员开窗失败零 reviewer 启动并回收已开窗口；任一成员失败或违规即整轮 fail-closed，PARTIAL 仅留诊断。

**变更政策分档**：窗口携带显式 `mutation_policy`（`RecoverableVerification` / `ZeroWrite`），冻结后不可变并进入 manifest 指纹。ZeroWrite 仅用于 plan reviewer，基线后任何 worktree 差异（含 ignored 产物）均生成违规事实，且先落证据再恢复、恢复成功不洗白；coding reviewer 与 internal reviewer 保持既有 B 档临时验证豁免，不得声称全部 Reviewer 已统一零写。ZeroWrite 是采信政策，不是物理 no-exec 承诺。

**双范围**：可读面＝LC 根全树（所有角色 cwd＝manifest `provider_context_root`，`readable_roots` 覆盖根全树，根级规则/工具/索引上下文继续可读可加载）；对账面＝Issue 选中的成员仓。LC 根级非成员条目不纳入基线、比对与恢复，并在报告逐项披露“根域未对账”；成员仓内部同名条目仍属该成员完整基线。对账域收窄不等于可读面收窄，不做根 guard。

**报告与判定**：一份报告按成员分节（每成员恰一节，含状态、成员 verdict、finding 归属、raw 引用、window/baseline 标识），外加一个由服务端事实计算的确定性总判定；模型不得伪造 accepted，completeness 为 partial 时 acceptance 不得为 accepted。成员分段只引用 finding ID，不复制内容分叉。

**标识与接管**：集合基 ID 取 session+plan run，成员子窗 ID 追加成员 UUID（不用 alias 或顺序号）；终态旧窗口归档后可接管并延续原基线与 journal，非终态窗口仍拒绝接管。`fabf3e95` 已落地该前置，不重复修复。

**启用门**：多成员评审入口只在范围冻结、集合屏障、ZeroWrite 对账、成员分段报告、总判定与恢复闭环全部落地并验收后启用；不得以单仓锚定、仅分段报告或漏检的过渡模式交付。

**契约落点**：本 change 的 session-policy-envelope delta 新增 REQ-ENV-13/14/15/16/17 并改写 REQ-ENV-REVIEW-B；logical-codebase-aggregate-planning delta 新增 REQ-PLN-11/12/13 并在 PLN-06 补 plan ZeroWrite 全集对账。主规格同步 PLN-02/04 与 primary 语义、TGT-01/03、MTG-01/02；在途 delta 最小修订 `lc-root-initialization`（ENV-03/04/10/11）与 `fix-planning-session-anchor-recovery`（PLN-03/05/07/08）。主规格 session-policy-envelope 的 ENV-01..09 全部被在途 delta 占用，其 B 段与 ZeroWrite 只经活 delta 新增，不在主规格另发冲突修订。

| 需求 | 承载条款 |
|---|---|
| 范围同源与冻结 | REQ-PLN-11、REQ-ENV-14、REQ-PLN-03（在途 delta）、REQ-ENV-04（在途 delta） |
| 集合屏障与整轮 fail-closed | REQ-ENV-15、REQ-ENV-REVIEW-B、REQ-PLN-13 |
| ZeroWrite 分档 | REQ-ENV-13、REQ-ENV-03、REQ-PLN-06 |
| 可读面与对账面分离 | REQ-ENV-16、REQ-ENV-10/11（在途 delta） |
| 成员分段报告与总判定 | REQ-PLN-12 |
| 窗口标识与终态接管 | REQ-ENV-17 |

### 已知偏差台账

| 编号 | 文件/事实 | 合同矛盾 | 处置归属与验收 |
|---|---|---|---|
| DEV-01 | `src/product/logical_codebase/production_policy_resolvers.rs:344-362`，require_write_boundary 的 Unknown+旧 supported_actions 臂返回 Ok；gateway `:1322/:2316` 两阶段消费，controller 另核 `:2556` | trait `:893-898` 与原 spec 要求仅 Confirmed，旧运维报告称无迁移默认 allow；函数注释表明是有意迁移桥、不铸 Confirmed，非本任务认定的新实现缺陷 | controller 裁定允许原 change 附偏差归档；本 delta 显式废止桥，tasks 2.5 实施删臂并改生产消费测试；legacy 未重探针必须 Unknown 拒绝，不能只测 mock |

原 change 十四项实装核验为“通过附偏差清单”：源码/测试入口与既有52格真实矩阵在位（每家13格：7 confirmed、4 carried、2 denied），有 argv/native ID/事实/事件/前后快照。四家 plan/split_sync/fresh 无 manifest.sha256 是工件限制（原 spec 未要求此特定文件）；历史成功格 provider_spawn_count=0 不证明当前零会话启动。本任务未重跑 Cargo 或真实 CLI，不把既有报告称为本次 fresh 验收。

### 五点交付映射

| 红队阻断 4 要求 | delta / requirement |
|---|---|
| REQ-LCG-04 永久拒绝→四家全开含 Codex | `lc-gateway-multi-provider` / Codex 仅受限安全模式（保留原标题，正文为全开） |
| REQ-ENV-09 作者/评审 deny 与不得放宽→author-only 岗位 | `session-policy-envelope` / REQ-ENV-09；联动 ENV-01/02/03/05 与新增 HOST/B/CHECK |
| REQ-PLN-06 规划只读→Kimi 规划配套放开 | `logical-codebase-aggregate-planning` / REQ-PLN-06，联动 PLN-01 |
| write_boundary/Confirmed 与 spawn 再校验 | `lc-gateway-multi-provider` / 映射、admission、projection、E2E；保持原门两次消费但迁移语义 |
| Kimi 三条合同 | `kimi-acp-client-services` / OS 隔离、grammar、权限与 fs（原题 MODIFIED）；另 lifecycle 配套；`kimi-code-provider-integration` / ACP 会话、Auto/Supervised、逐工具审批三条 |

## Risks / Trade-offs

- [同 UID 可读凭据、可写宿主] → owner 已接受；子进程继承宿主环境、安全由用户自管，审计只检测不物理防写，不作超额保证。
- [恶意依赖与外部副作用] → 明示 postinstall/mvn 插件、网络/容器副作用不可回滚，二期由 owner 安排。
- [恢复抹去违规证据] → 违规事实在恢复前持久化；恢复状态和报告可采信独立。
- [旧归档复活受限档] → 新语义、仅 Confirmed 与拆桥任务、真实主 spec MODIFIED、anchor 依赖修订硬门，不把旧记录强改 Confirmed。
- [Codex danger 无作者代码写预防] → 明示检测型 fileChange 纪律与真机双审批档探针；事件后中断+失败关闭+durable 审计，不夸大为预防。
- [假静止与误删 owner 目录] → 未登记 writer 如实报告；先确认静止再清，清理仅运行前登记私有目录。

## Migration Plan

1. 本 change 先通过 strict，作为契约审阅输入；批准后由后续 writing-plans 展开高层工作包，不在本次执行实现。
2. 平台鉴权公共前置先交付；受信装配、宿主继承环境、资源与 Git 护栏统一接线，再迁移 action 能力语义与 Direct/LC 两阶段门。共享文件单一集成人。
3. 四家全部角色采用统一宿主执行，author 工具机制与 Reviewer B 档窗口分别验收；Kimi 删除旧全角色沙箱/grammar/120 秒路径，不留双路径 fallback。
4. 旧 capability/audit/native session 保留历史，失配进入明确核验或 fresh 等待；不沿用旧隔离指纹，不洗白未关闭 Reviewer 窗口。
5. 真实验收覆盖四家×角色及 Direct/LC、fresh/resume/retry，Kimi Maven/npm/Podman、长静默 CHECK、Reviewer 自提交作废与崩溃恢复、鉴权拒绝及正常 Coder 交付。缺工具或登录态按真实前提缺失记录，不 skip 后宣称全通。
6. 若部署回退，先停止新会话准入与活动 writer，收口/人工处理未闭 B 档窗口，再恢复与所部署版本匹配的旧契约/能力材料；不得仅回退代码却复用新证据，或把污染现场作为旧基线。
7. plan 评审范围与多成员对账域按「范围/集合基础 → 集合屏障与 ZeroWrite 对账 → 报告与总判定 → 集成验收」推进；各批次可独立提交验收，但在范围冻结、集合屏障、ZeroWrite 对账、成员分段报告与恢复闭环全部落地并通过验收前，多成员评审入口保持关闭，不得以单仓锚定、仅分段报告或漏检的过渡模式对外交付。

未决产品决策：无。原网关归档由 controller 执行，anchor 最小冲突修订是明确归档门、未声称本次完成。strict 只证明工件与 delta 合法，不证明实现或真实行为已通过。

## 失效措辞扫描清单

扫描对象为最终 `openspec/specs/` 与待归档的 anchor-recovery 相关 delta；归档前检查目标新正文及尚待迁移的主规范命中，归档后检查真实主规范。必须人工区分“历史来源/明确废止”引用与仍在生效的规范性结论，不能仅靠零字面命中宣称正确。

| 检索词族 | 必查条款 | 通过条件 |
|---|---|---|
| 永久禁止/永久拒绝 + Codex/danger-full-access；codex_danger_full_access_unsupported | 网关 Codex（REQ-LCG-04）、ENV-05、ENV-09；anchor 的网关投影 | 只能作为被取代的历史说明；不能仍要求有效 danger projection 在 spawn 前被拒绝 |
| target-only/只读 boundary/保护路径写成功/E2E 越界 | 网关映射、admission、projection、E2E | 新语义仅 Confirmed 放行，宿主普通写成功不再判能力失败；逻辑交付范围与 B 档违规仍独立有效 |
| Unknown + supported_actions/旧 allow/迁移默认 allow | 新网关映射/admission 及 DEV-01 实施证据 | 旧记录必须重探针；生产 capability source 不得存在 Unknown 放行臂；不能只改文案 |
| Reviewer + DenyFileWriteBuiltins/带策略/只读会话/Terminal 恒拒 | 网关“按角色守卫 tool policy 并同步 preflight”、ENV-01/02/09、重试与 Kimi 三条 | Reviewer 全工具面并接 B 档窗口；作者岗位规则不扩到 Reviewer |
| Codex author + 不可达/强制 Decline/物理预防 | ENV-09 及规划场景 | 只承诺 fileChange 检测、中断 turn、失败关闭和 durable 审计；必须有 danger+apply_patch 双审批档真机观察 |
| 规划只读/production_verified_readonly/best_effort_configured | PLN-01/06 与 Purpose；anchor PLN-03/05/07/08 | 本 change 改 PLN-01/06，anchor 改 PLN-03/05/07 与加 PLN-08，标题不重叠；ReadOnly action 名不被误解为物理限制，聚合身份需求仍归 anchor 后续实施 |
| bwrap 不可用拒绝/封闭 grammar/TERMINAL_COMMAND_TIMEOUT_SECS=120 | Kimi ACP 三条与生命周期 | 全角色 Host 单路径；保留 fs no-follow、并发/输出/总 deadline，不复活旧沙箱或独立 120 秒硬截断 |
| ZeroWrite + 物理不可写/no-exec/全部 Reviewer 统一零写 | REQ-ENV-03/13、REQ-ENV-REVIEW-B、REQ-PLN-06 | 只承诺观察差异判定、先取证后恢复与恢复后仍作废；不得宣称物理隔离，不得声称 coding/internal reviewer 已统一零写 |
| 首仓锚定/非主成员未对账但继续/primary 决定评审集合 | REQ-PLN-11、主规格 PLN-02 与 primary 语义 | 评审集合恒为有效选集全集；旧首仓兼容路径与附注文案须删除 |
| 根域已零写/根域未对账/对账域等同可读面 | REQ-ENV-16、REQ-ENV-10/11（在途 delta） | 根级条目逐项披露“根域未对账”且未采基线/未比对/不恢复；对账域收窄不得收窄 readable_roots，成员内部同名条目仍完整对账 |

归档前未通过任一项时保持本 change 归档门关闭；归档后由 controller 对主规范执行同清单与 strict 验收，不以提案 strict 代替实现或同步后的真实主规范验收。
