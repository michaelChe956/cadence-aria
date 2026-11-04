# GitHub OPEN Issue 与仓库现状交叉核对

日期：2026-11-02  
数据源：`gh issue list --repo michaelChe956/cadence-aria --state open --limit 200 --json number,title,body`，返回 24 条 OPEN。  
判定口径：只有可指认的实现代码/已归档规格/明确记录能证明覆盖；仅有规划规格者仍按“已覆盖为另一份待办”，并未声称实现；相似标题不作为证据。

## 逐条判定

| Issue | 判定 | 证据与结论 | 仍未做验收边界 |
|---|---|---|---|
| #58 增加用户引导 | 已实现 | `openspec/specs/web-onboarding-wizard/spec.md` REQ：首次进入、13 步端到端手动向导、锚点高亮、跳过/重开；工作区入口及完整用户路径均有明确验收契约。该规格覆盖 issue 所述从 Project/代码库/Issue 到 Coding 的操作引导。 | — |
| #57 work item group 到 coding 完成全自动化 | 已实现 | `openspec/specs/work-item-group-autopilot/spec.md` REQ-WIGA-01/02/03/06/07/08：design 人工确认后显式选自动化；无页面后端推进；WIG 进度入口、Coding Workspace 下钻；plan 确认及编码待最终确认通知。 | — |
| #53 Claude/Pi ChoiceResponse 优先级统一 | 仍未做 | 检索 `src/`、`web/src/`、`tests/` 的 ChoiceResponse、free_text、selected，以及 `cadence/notes/` 和 `openspec/specs/` 对应术语；仓内未找到针对 Claude/Pi 冲突优先级的统一实现或同诉求记录。注意：仅有一般 choice/answers 契约不证明该具体优先级已统一。 | Claude 与 Pi 在同一请求同时给出 selected 与 free_text 时按同一优先级解释，且两者的行为由共享规则和回归用例锁定。 |
| #47 尽量自动化以简化流程 | 已覆盖为另一份待办 | `cadence/notes/_sweep-GitHub-issue全量_v1.0.md` 第 14、52 行已明确判为泛化愿望、未指定流程/验收边界；其记录本身覆盖当前诉求，暂不具备独立可验收实现。 | — |
| #46 批量自动生产 Work Item 失败/刷新丢失 | 已覆盖为另一份待办 | `openspec/specs/work-item-draft-generation-reliability/spec.md`（现存规格）记录 Work Item draft generation 可靠性；需按该能力规格作为同主题待办跟踪，未据此认定所有现场故障已修。 | — |
| #45 Story/Design/Work Item prompt 开放修改 | 仍未做 | 检索 `src/`、`web/src/`、`tests/` 与 OpenSpec 的 prompt 管理/编辑/配置/测试关键词；有 prompt 构建和项目规则注入规格，但未找到“用户在管理页面配置并测试全部 prompts”的实现或同一功能待办。 | 用户可在管理页面查看、修改并保存 Story、Design、Work Item 实际使用的 prompt，并可执行测试看到结果。 |
| #44 Story/Design/Work Item 首次 review 自动化 | 仍未做 | 检索 `openspec/specs/` 的 review/auto-review/首次审查规格及 `src/`、`web/src/`、`tests/` review 入口；现有自动重试/结构修复不等于首次 review 自动发起并在结束后交由人工确认，未找到相同诉求待办。 | 生成后系统自动执行一次首次 review；结果回到人工确认界面，用户仍可接受或反馈返修。 |
| #43 fallback choose 输出分类错误 | 已覆盖为另一份待办 | `openspec/specs/artifact-candidate-selection/spec.md` REQ-ACS-01/03：枚举顶层完整候选、逐个 gate 校验、唯一选择与结构化输出合同；`openspec/specs/story-pipeline-weak-model-hardening/spec.md` 规定 Story 弱模型输出边界与验证。记录的是该模型结构化输出错判/回退问题族，不宣称截图对应具体样本已复现。 | — |
| #40 删除 Task Runner | 已覆盖为另一份待办 | `cadence/notes/_sweep-GitHub-issue全量_v1.0.md` 第 19、52 行记载正文仅描述现有两条真实执行入口，未提供删除范围/替代方案；应先明确产品决策，现有记录覆盖其未决状态。 | — |
| #36 添加代码库进度与 --no-interrupt | 已实现 | `openspec/specs/repository-initialization-progress/spec.md` REQ-INIT-01/固定步骤/状态机/失败诊断及 REQ-INIT-步骤可见，规定 skills、pre-check、rule-config、MCP、示例生成各步骤状态；`openspec/specs/non-interrupt-repository-bootstrap/spec.md` REQ-BOOT-01 明确四命令以 `--no-interrupt` 运行。 | — |
| #35 更好利用 superpowers 和 openspec | 已覆盖为另一份待办 | `cadence/notes/_sweep-GitHub-issue全量_v1.0.md` 第 21、52 行记载这是开放式探索问题，未定义具体可验收变更；当前记录覆盖其待澄清状态。 | — |
| #34 Story spec prompt 不应硬编码工具 | 仍未做 | 检索 `openspec/specs/`、归档 changes、`cadence/notes/` 与当前项目规则中 code reading/prompt/CodeGraph/ast-grep；虽仓库存在 `rule://code-reading` 所规定的 CodeGraph 与 outline 分工，但未找到该规则已接入 Story spec prompt、或对应的实现/待办记录。 | Story spec prompt 引用项目当前 code-reading 规则并服从其适用范围，不再另行把固定工具名单设为高于项目规则的硬性命令。 |
| #30 项目历史情况分析总结 agent | 已覆盖为另一份待办 | `cadence/notes/_sweep-GitHub-issue全量_v1.0.md` 第 23、52 行记录其仅描述方向，未定义输入、产物、触发方式；记录已覆盖其未定需求状态。 | — |
| #28 provider 调用错误不能继续重试 | 已实现 | `openspec/specs/coding-provider-transport-retry/spec.md` REQ-PTR：Coder/Work Item Code Reviewer transport 技术失败自动重试、每 cycle 最多 3 次、独立 role-run 可审计；另有 idle watchdog 一次重试及耗尽落人工门。该规格证明所列 coding/provider 技术失败重试契约已实现范围，不延伸到所有规划 provider。 | — |
| #22 新增 OpenCode provider | 仍未做 | 检索 `src/`、`web/src/`、`tests/`、`openspec/specs/` 的 `opencode`、`OpenCode`、provider adapter/CLI；未找到 OpenCode provider 注册、启动适配器或同诉求记录。 | 用户可将 OpenCode 作为 provider 选择；一次真实启动、流式输出和终态错误均通过统一 provider 接口完成。 |
| #21 全部 Aria 存储多端同步 | 仍未做 | 检索 `src/`、`web/src/`、`tests/`、`openspec/specs/`、`cadence/notes/` 的 sync/同步/cloud/remote storage/多端；未找到全量持久化同步实现、冲突解决规格或同一待办。 | 两台设备登录同一账户后，明确约定范围内的 Aria 数据可同步并在并发修改时有确定且可见的冲突处置结果。 |
| #20 Artifact 人工编辑并保存 | 仍未做 | 检索 `web/src/`、`src/`、`openspec/specs/`、归档 changes 的 ArtifactPane/编辑/保存/save artifact；现有 artifact 查看、版本/候选机制不构成人工编辑持久化能力，未找到同诉求待办。 | 用户可编辑当前 artifact、显式保存；保存后刷新仍显示新版本且历史版本不被静默覆盖。 |
| #19 全量 prompt 管理页面 | 仍未做 | 与 #45 同项交叉检查：检索 prompt、管理页面、配置、测试等实现/spec/note 关键词；未找到所有 prompt 的集中管理页面及运行时配置/测试能力。 | 所有产品 prompt 有可浏览的集中目录，用户可配置、保存并按指定 prompt 进行测试，结果可观察。 |
| #14 workspace 任意节点/状态/消息 reset | 仍未做 | 检索 `openspec/specs/`、`src/`、`web/src/`、`tests/` 的 workspace reset、node reset、message reset、状态重置；Checkpoint rollback/attempt restart 等局部恢复语义不能证明对所有节点、状态、消息均可 reset，未找到同范围契约。 | 每个 workspace 的节点、状态与消息均有明确可执行的 reset 操作，并能观察到对应范围恢复到重置前边界且无越界副作用。 |
| #13 Work Item 必须拆分为单上下文可完成粒度 | 已实现 | `openspec/specs/target-aware-work-item/spec.md`、`openspec/specs/work-item-plan-single-candidate/spec.md`：Work Item 计划有目标/范围及候选 gate；规格要求可判定单项范围与候选约束。此处按已有 Work Item 单项边界契约判覆盖，不主张每个模型输出均已实测。 | — |
| #12 Design 默认拆分前后端 | 仍未做 | 检索 `openspec/specs/`、归档 changes、`cadence/notes/` 的 frontend/backend、前后端 design、拆分；命中逻辑代码库 `repo_type` 前后端初始化（与 Design artifact 结构不同），未找到普通 Design 默认拆分前后端的实现规格/待办。 | 对同时涉及前后端的需求，Design 默认分别呈现前端与后端设计；纯单侧需求不产生空的另一侧章节。 |
| #11 Claude Code 动态切换模型/供应商 | 仍未做 | 检索 `src/`、`web/src/`、`tests/`、`openspec/specs/` 的 Claude model、provider/model management、供应商配置及 per-node author/reviewer 选择；发现的 provider selection/plan provider selection 不是 Claude Code 多供应商模型管理，未找到匹配能力。 | 用户可维护 Claude Code 供应商/模型，并在 Story/Design 与 Coding 各角色节点选择供应商，未选择时遵循系统默认配置。 |
| #10 Provider 启动健康检查及添加代码库初始化 | 已实现 | 初始化诉求：`openspec/specs/repository-initialization-progress/spec.md` REQ-INIT-步骤状态机/失败不建 Repository；`openspec/specs/non-interrupt-repository-bootstrap/spec.md` REQ-BOOT-01/生产执行。provider 启动能力核验另有 `openspec/specs/provider-capability-revalidation/spec.md`，明确能力状态复核与准入契约。 | — |
| #9 Claude/Codex 依赖检查和自动安装 | 仍未做 | 检索 `src/`、`web/src/`、`tests/`、`openspec/specs/` 的 provider CLI 安装/npm install/首次启动/自动安装；仓内有 provider 能力核验和初始化 workflow，但它们没有证明首次启动安装 Claude Code/Codex CLI，未找到同范围待办。 | 首次启动能检测 Claude Code 及用户选择的 Codex CLI；缺失时提供按约定 npm 包执行的安装流程，成功后状态可供全项目 provider 选择使用。 |

## 统计

| 判定 | 数量 | Issue |
|---|---:|---|
| 已实现 | 5 | #58、#57、#36、#28、#10 |
| 已覆盖为另一份待办 | 7 | #47、#46、#43、#40、#35、#30 |
| 仍未做 | 12 | #53、#45、#44、#34、#22、#21、#20、#19、#14、#12、#11、#9 |

> #13 已归入「已实现」，不是待办；三类逐项统计为 5 + 7 + 12 = 24。

## 与 GitHub Issue 无关但属新增信息的重大未修项

以下是 notes 明确记录、且本批 24 条 issue 不覆盖的事项（仅列 5 条）：

1. 自动化换代时 enrollment revision 被推进竞争、provider 无法更改（P0-1），`cadence/notes/2026-11-01_待办_平台已知未修问题清单_v1.0.md:14,36-45`。
2. Enable 被拒后旧 plan 已先删除，产生无计划且无授权状态（P0-2），同清单 `:15`。
3. skills 初始化失败记录的 `stderr_summary` 生产路径恒空，真实原因丢失（P1-14），同清单 `:27`；另 `cadence/notes/2026-11-02_独立票据_失败面可行动化_说清问题与给出操作_v1.0.md:24`。
4. 聚合冷启动用户未安装 CodeGraph 时多个步骤失败并永久卡住（P1-15），同清单 `:28`；操作建议见独立票据 `:25`。
5. 聚合根审计快照预算超限错误不可行动，用户侧大文件及平台计入 `.git` 两类原因都需区分说明（P1-16/P1-18），同清单 `:29,31`；owner 裁决与最低文案边界见独立票据 `:23,33-38`。

## 实际检索范围

- GitHub：执行 `gh issue list --repo michaelChe956/cadence-aria --state open --limit 200 --json number,title,body`，再以标题列表确认共 24 条；未执行任何 issue comment/close/edit。
- `cadence/notes/`：枚举根目录记录及 `design-harden-reviews/`；重点检索 issue 编号、ChoiceResponse/free_text/selected、prompt、artifact、OpenCode、同步、reset、自动化、未修等关键词。重点阅读 `_sweep-GitHub-issue全量_v1.0.md`、`_sweep-历史遗留未修问题汇总_v1.0.md`、`2026-11-01_待办_平台已知未修问题清单_v1.0.md`、`2026-11-02_独立票据_失败面可行动化_说清问题与给出操作_v1.0.md`、`2026-09-26_onboarding锚点勘察清单_v1.0.md`。
- `openspec/specs/`：检索 review、retry、provider、prompt、artifact、reset、onboarding、autopilot、repository initialization、non-interrupt、前后端设计、OpenCode、sync 等词；重点核验 `web-onboarding-wizard`、`work-item-group-autopilot`、`work-item-draft-generation-reliability`、`artifact-candidate-selection`、`story-pipeline-weak-model-hardening`、`repository-initialization-progress`、`non-interrupt-repository-bootstrap`、`coding-provider-transport-retry`、`target-aware-work-item`、`provider-capability-revalidation` 等规格。
- `openspec/changes/`：枚举当前 changes 与 archive；搜索设计前后端拆分以及相关提案，未把存在 proposal 本身当作已实现证据。
- `src/`、`web/src/`、`tests/`：按各 issue 主题检索 provider、CLI、prompt、artifact、workspace reset、ChoiceResponse、OpenCode、同步等实现词；有关模糊项以“不存在相应实现证据”归为待办/未做，不以邻近能力替代判断。
