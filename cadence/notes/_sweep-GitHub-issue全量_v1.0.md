# GitHub Issue 全量清单（排期输入）

数据源：`gh issue list --repo michaelChe956/cadence-aria --state all --limit 200 --json number,title,state,createdAt,closedAt,body,labels,url`。抓取共 **24** 条：OPEN **24**，CLOSED **0**。此结果仅包含该命令当前返回条目。

> 核心诉求引用依 issue body 摘录；覆盖核查仅针对目前可直接确认的线索，未执行逐条代码/提交历史核验的均明确标记「未核实」。每条的 closedAt 均为 null。

## ① 全量清单

| # | Issue / 标题（逐字） | 状态 / closedAt | Body 核心诉求（原文） | 具体可实施？ | 覆盖核查 / 关联记录 |
|---:|---|---|---|---|---|
| 58 | 增加用户引导 | OPEN / null | “用户首次进入需要给用户增加全流程操作的操作引导，包括：怎么添加项目，代码库（目前只支持单代码库），issue的创建，story，design，work item，coding怎么操作” | 是：新增首次使用的分步引导 | 未核实；cadence/notes、openspec 关联未核实 |
| 57 | 关于 work item group 到 coding 完成的全自动化 | OPEN / null | “design结束后，开始发work item group的任务。弹窗可以选择是否自动化”；“work item group 编写完需要给用户发通知，coding 完成也需要给用户发通知” | 是：后台自动执行、入口查看及通知联动 | 未核实；关联文件未核实 |
| 53 | 额外收获（你的"确认下拼接"要求）：发现 Claude/Pi 的 ChoiceResponse 优先级不统一（Claude free_text 优先、Pi selected 优先），你已记下要单独开任务统一——这是个有价值的副产品发现。 | OPEN / null | “发现 Claude/Pi 的 ChoiceResponse 优先级不统一（Claude free_text 优先、Pi selected 优先）” | 是：统一响应优先级规则 | 未核实；关联文件未核实 |
| 47 | 整个过程太复杂了，还是要考虑哪些可以自动化的尽量自动化 | OPEN / null | “整个过程太复杂了，还是要考虑哪些可以自动化的尽量自动化” | 否：泛化愿望，未指定自动化流程/验收行为 | 未核实；关联文件未核实 |
| 46 | work item 批量自动化生产有bug，无法正常生成 | OPEN / null | “work item 批量自动化生产有bug，无法正常生成。刷新后也无法保留之前生成的内容” | 是：修复生成失败及刷新后状态丢失 | 未核实；关联文件未核实 |
| 45 | story design work item的prompt需要开放修改 | OPEN / null | “story design work item的prompt需要开放修改” | 是：开放相应 prompt 配置 | 未核实；关联文件未核实 |
| 44 | story，design， work item 第一次review都做自动化 | OPEN / null | “如果是必须review一次，自动review就好了，没必要人工再点一次”；“自动review完，再让人来确认是不是符合要求” | 是：首次 review 自动执行并保留人工确认 | 未核实；关联文件未核实 |
| 43 | fallback choose的相关问题 | OPEN / null | “一些模型输出的有问题。比如截图的内容应该就是一个执行步骤，但是输出的内容更像选择了，导致程序解析错了” | 是：修复模型输出分类/解析错误 | 未核实；含截图需进一步复现；关联文件未核实 |
| 40 | 删除 Task Runner | OPEN / null | “Task Runner 的真实执行入口有两条：CLI：aria task run ...，会完整跑规划、执行、最终审查链路” | 是：具体提出移除 Task Runner，但说明内容像背景/现状陈述，目标及替代方案不清 | 未核实；关联文件未核实 |
| 36 | 针对添加代码库出现的问题进行优化 | OPEN / null | “添加代码库期望有进度展示，包括：Cadence-skills的下载和软连”；“使用--no-interrupt的方式” | 是：增加进度展示并调整 skill 命令调用 | 未核实；关联文件未核实 |
| 35 | superpowers和openspec如何更好的利用 | OPEN / null | “superpowers和openspec如何更好的利用” | 否：开放式探索问题，无具体可验收变更 | 未核实；关联文件未核实 |
| 34 | story spec的prompt是有问题的，不应该写死使用codegraph，ast-grep等 | OPEN / null | “这段内容是不对的，应该是强制要求按照项目中的code reading规则去做” | 是：修正 story spec prompt 的工具约束 | 未核实；关联文件未核实 |
| 30 | 项目历史情况分析总结 | OPEN / null | “项目历史情况分析总结 agent” | 否：仅描述方向，未定义输入/产物/触发方式 | 未核实；关联文件未核实 |
| 28 | 所有的provider出现模型调用出错的情况下无法继续重试 | OPEN / null | “所有的provider出现模型调用出错的情况下无法继续重试” | 是：修复 provider 调用失败后的重试能力 | 未核实；截图需核对错误边界；关联文件未核实 |
| 22 | 新增opencode provider | OPEN / null | “新增opencode provider” | 是：增加具体 provider 支持（接入范围仍需定义） | 未核实；关联文件未核实 |
| 21 | 整个aria存储内容，我期望可以多端同步 | OPEN / null | “整个aria存储内容，我期望可以多端同步” | 否：方向明确但同步范围/冲突与存储后端未定义，属产品/基础设施决策 | 未核实；关联文件未核实 |
| 20 | Artifact 期望可以人工编辑，需要有保存按钮 | OPEN / null | “Artifact 期望可以人工编辑，需要有保存按钮” | 是：编辑 Artifact 并提供保存操作 | 未核实；关联文件未核实 |
| 19 | prompt个性化 | OPEN / null | “把aria中用到的所有prompt都抽离出来，提供一个管理页面可以进行配置和测试” | 是：prompt 管理、配置和测试界面 | 未核实；关联文件未核实 |
| 14 | 针对所有的workspace添加reset的功能 | OPEN / null | “每个节点都期望可以进行reset，每个状态期望可以reset，每条消息也期望可以reset” | 是：为 workspace 的节点、状态、消息提供 reset | 未核实；关联文件未核实 |
| 13 | work item优化 | OPEN / null | “design spec生成work item的时候需要强制对work item进行拆分。要求每个work item必须是在一个上下文内可以完成的。” | 是：约束 Work Item 粒度与拆分 | 未核实；关联文件未核实 |
| 12 | design spec 默认拆分前后端 | OPEN / null | “编写design spec的时候需要默认拆分为前端design和后端design。除非是纯前端需求或者纯后端需求，否则都需要进行拆分” | 是：调整 design spec 默认结构 | 未核实；关联文件未核实 |
| 11 | claudecode可以动态切换模型 | OPEN / null | “用户在spec workspace和coding workspace使用claude code provider的时候，可以动态切换模型” | 是：支持 workspace/节点选择 Claude Code 模型及供应商 | 未核实；关联文件未核实 |
| 10 | 添加代码库需要对代码库进行初始化 | OPEN / null | “启动aria程序需要对claude code，codex等provider进行健康检查”；“添加代码库需要对代码库进行初始化” | 是：启动检查 provider、初始化 repo 与依赖 | 未核实；关联文件未核实 |
| 9 | 项目依赖 自动安装 | OPEN / null | “aria需要使用到claude code，codex，用户可能没有安装，需要帮助用户进行自动检查与自动安装” | 是：检查并自动安装 CLI 依赖 | 未核实；关联文件未核实 |

## ② 主题归类

| 主题 | Issue 编号 |
|---|---|
| UI 交互与上手/Artifact | #58、#20 |
| Work Item / Story / Design 流程自动化与质量 | #57、#47、#46、#44、#13、#12 |
| Prompt 定制与规则 | #45、#19、#34 |
| Provider、模型与网关 | #53、#43、#28、#22、#11、#9 |
| 存储、同步与 Workspace 操作 | #21、#14 |
| 代码库初始化与技能接入 | #36、#10 |
| 自动化/研发流程与工具决策 | #35、#30、#40 |

## ③ OPEN 且诉求具体可实施（排期候选）

#58、#57、#53、#46、#45、#44、#43、#36、#34、#28、#22、#20、#19、#14、#13、#12、#11、#10、#9。

排除 #47、#35、#30（目标泛化）；#21（需先作同步架构决策）；#40（仅标题与所给正文不足以判定移除范围）。

## ④ OPEN 诉求覆盖核查

本次取得的是 issue 列表数据；没有逐项检视仓库实现、提交历史或 `cadence/notes` / OpenSpec 内容。因此 **所有 24 条 OPEN issue 的覆盖状态均为「未核实」**，没有以标题或 issue 状态推定实现完成。关联记录文件名也均未核实；后续应逐条查找后再用于排期。
