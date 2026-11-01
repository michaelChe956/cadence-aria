# 技术方案: Git 提交卫生与 Reviewer 纯只读权限收敛(v2.0)

- 出具: design-task-sol (ReviewerReadOnlyV2), 2026-10-16, 受测试会话委托; **设计完成未实施(DONE_WITH_CONCERNS)**
- **本方案替代 v1 的 Reviewer 构建落位方向; T2/T5 副本机制正式废除**; 源码事实均物理文件复核
- 关联: designs/2026-10-16_技术方案_Git提交卫生与Reviewer构建落位_v1.0.md(其 T1/T4 卫生部分保留, Reviewer 部分废止)

## 一、结论

### 修复目标重新定性
铁证(controller 核实): 6880023 提交时间与 pi CodeReviewer run 重合(18:16:16), **实际由 Reviewer 自行提交**。
根因链: ①provider_retry.rs:487-495 给 CodeReviewer 注入公共 worktree 纪律+platform_directives ②prompts.rs:494-495 要求构建/测试/Git mutation 在交付 worktree 内执行且允许只读 Reviewer 复跑 ③prompts.rs:514-517 又声明「提交责任归你」 ④pi 仅禁 edit,write 未禁 Bash(mod.rs:48-52,437-498) → Reviewer 能构建+git add -A 交付残留。
**修复不是换构建地点, 是撤销 Reviewer 的执行及提交能力。** 3158bf95 后不再把 ignored 产物撑爆 64MiB 列为待修后果; 不改预算/沙箱/bwrap/REQ-LCG-04。

### 三层组合
| 层 | 机制 | 职责 |
|---|---|---|
| **主闸** | Reviewer 专用 no-exec 工具策略+真实行为证据能力门 | 获准工具面物理不可写(不暴露 exec/写/提交/执行型 MCP 扩展) |
| **纵深检测层** | 前后状态对账+审查期关闭平台提交通道+报告绑定受审状态 | 检出主闸失效/接线错误/外部并发写, 阻污染入交付与采信 |
| **纪律层** | Coder/Reviewer 文案分离 | 去冲突指令: Reviewer 只审查不验证不修改不提交 |

**纵深层不是防写硬边界**(对账挡不住瞬时写/写后恢复; 关平台通道挡不住 Bash 自 commit)——必须先撤执行工具面。「物理不可写」仅指经行为验证的平台允许工具面, 不是新 OS 沙箱。

### 验证衔接(沿用 A')
验证执行者=Coder 或人工; 证据源=交付物/raw/artifact/人工回贴; Reviewer 只查证据是否支持结论, **不复跑任何验证**; 缺失如实 NOT_EXECUTED(verdict 仍 approve|request_changes|blocked, 不新设); 现场 v1.0 报告的 BLOCKED 是应保留的正确先例。

## 二、改动点清单(摘要)

| 改动面 | 锚点 | 改动 |
|---|---|---|
| 角色纪律 | prompts.rs:395-518 | 公共 worktree 段收回为 Coder 纪律; 新增 Reviewer 只读纪律(禁构建/安装/测试/lint/脚本/一切 Git mutation); 删复跑例外+提交责任 |
| 注入点 | provider_retry.rs:475-554; internal_pr_review.rs:297-407; group_review_{prompts,orchestrator} | 全路径(fresh/resume/retry/shard/reduction/repair/GroupFinal)统一; 不再注入 Coder 的 platform_directives_section |
| 工具策略 | streaming_provider/mod.rs:80-127,511-557 | 新增 Coding Reviewer 专用意图(建议名 ReviewerReadOnlyNoExec); 不扩大 DenyFileWriteBuiltins 语义, 不动非 Coding Reviewer |
| Provider 实现 | 四家 provider adapter/session/projection | 新策略同源消费; 工具集+扩展/MCP 配置进 profile/canonical digest/启动审计/resume 比对 |
| 主闸证据 | provider_capability_{store,probe,revalidate}.rs | Review 行增 no-exec 证明; 旧「可启动/只读边界 Confirmed」不自动视为 no-exec |
| 审查窗口(新增) | coding_workspace_engine/reviewer_read_only.rs + coding_attempt_store/reviewer_read_only.rs | 冻结基线/只读窗口/全退出收口/不可静默洗白的校验; 状态写平台数据目录不写受审 worktree |
| 提交兜底 | internal_pr_review.rs:564-761; git_operation.rs:99-146 | 提交及 journal 恢复前校验窗口; 活动/未对账/违规窗口禁 stage/commit/push/补偿(含 mixed-reset 补偿路径) |
| 证据预供 | coding_evaluation_context/ | Reviewer 分支补足可审查内容+来源绑定(现 CoderEvidencePack 只有 latest run+6000 字符摘要, 不足) |
| T1 提交卫生 | git_workspace_service.rs | 保留 v1 分类/显式暂存/恢复复验, 降为 P1(次于主闸) |

既有事实注意: DenyFileWriteBuiltins 自述非全工具 allowlist; LC ReviewReadOnly+writable_roots=[] 不能证明 pi 无 Bash; 跨仓 baseline 只监控主 checkout 不覆盖受审 worktree; 共享 AdapterRole::Reviewer 还用于非 Coding 产物审查(本次不动)。

## 三、硬闸设计(要点)

### 主闸: no-exec 能力
- 语义: 允许原生读/枚举/文本搜索/受控交互; 不允许 Bash/terminal/通用 exec/文件写/子代理执行/codemode 动态加载/未证明的 MCP 扩展; **Git 事实由平台预计算提供, 不留 raw Bash 跑「只读 git 命令」**; 不满足的 provider Reviewer 启动前 fail-closed(不降级旧 denylist/普通 spawn/legacy bridge)
- pi 首选: `--tools read,grep,find,ls` + 禁自动 extensions/MCP/skills 发现 + 只显式加载平台自有只读守卫扩展; **不能只加 --tools**(帮助明示默认可能保留 MCP; 现 argv 保留用户全局 extensions)。启动顺序: 校验指纹→精确参数启动(不发业务 prompt)→平台扩展检查 active/registered/可达工具+MCP 集合→有界只读握手取工具清单比对→不符即 kill/reap 不启动→通过后才发审查 prompt。active set≠全部可调用集(codemode/deferred exposure), 必须查全部注册及嵌套可达面。握手传输经平台自有扩展+stderr 有界读取=启动控制协议, 不是验证 runner; runtime 行为须实施中真实验证
- 各家边界: Claude 不把 allowedTools 当撤销机制, 须行为证明; Codex read-only sandbox≠不能验证执行, 须证明执行工具被撤或执行前拒绝; Kimi 复用 Terminal/FsWrite 硬拒绝+证明无 MCP 旁路
- 行为证据门: 绑定精确版本/二进制身份/adapter dialect/策略版本/argv 摘要/工具集合/扩展与 MCP 配置摘要/行为探针原始记录; 缺失/漂移/仅 --help/仅模型自述均失败关闭; fresh/resume 分开证明, 旧 session 摘要不兼容不得带旧工具面恢复。**探针只在受控 fixture 上试写, 绝不对真实 worktree 做破坏试验**(不是副本机制复活)

### 纵深选型
保留: 前后对账(违规审查失败)+审查阶段关闭平台 commit 通道+只采报告(附状态校验)。不采用: 自动恢复(可能抹 Coder 改动/历史重写)、Git hook/移除 author/PATH 包装(可绕过且误伤)。

### 状态对账: 比较「原样」不要求「干净」
Coder 合法未提交修改原样审查。窗口在材料采集前冻结: canonical 身份/HEAD/符号引用/index/staged/tracked+untracked+**ignored** 文件路径类型模式摘要/可观察元数据/symlink 本身(不跟随逃逸)。**不把 porcelain 当内容快照**(ignored 产物变化逃过它); 不套审计的 gitignore 排除——预算审计可忽略 target, 零写对账不能忽略。快照只存清单摘要不复制源码; 扫描不完整/权限不足/并发变化→显式「对账未完成」阻断。平台材料采集也须只读(新建安全 Git 观察入口, 关 optional index refresh/external diff/textconv, 不执行仓库自定义脚本——现 git_diff() 直接调 git diff 且只设 LC_ALL, 不能直接用)。成功/失败/取消/超时全收口; 崩溃留下未完成窗口下次只能复核原基线不得重新冻结。发现差异: raw 留审计但**不采信 verdict/不发通过事件/不推进交付**, 记录差异+稳定失败码+人工门; 不自动 reset/restore/clean/纠正提交。

### 平台提交通道: 按权威窗口拦截
活动 Reviewer 窗口禁平台 Git mutation(并行只读窗口可共存); 未完成对账/违规/归属未知→stage/commit/push/恢复路径失败关闭; 正常结束生成绑定基线+策略+role run+材料摘要的完整性结果; mutation 前同临界区复验(防先检查后进入竞态); 不依赖内存布尔/自然语言 caller/可伪造角色串。**取消恢复特别处理: compensate_cancelled_review_commit 的 mixed reset 不得被 Reviewer 越权错误触发**。Coder 正常构建/修改/commit 不变。

### 报告采信
Reviewer 输出不能更新 Coder completion commit/HandoffRevision/交付 SHA; 不能把审查期间新 HEAD 解释为 Coder 补交; 报告仅对绑定 diff/HEAD/index/工作区+证据包有效, 状态变化须重审; Group 任一窗口违规整组不能 approve, 不得经 reduction/repair 丢弃违规事实; GroupFinal 生产路径必须覆盖。

### 证据预供(8 维最小清单)
身份与版本/需求契约(含 manual check 分类)/当前实现(含截断标志)/验证记录(每项 check 的命令+cwd+状态+退出+原始输出; 未执行明确标记)/Coder 交付/人工证据/Group 跨仓逐 unit 关联(不能只给 attempt 最新一次 Coder 的证据)/历史与 artifact 覆盖声明(溢出显式登记不静默截断)。平台仅从既有记录预组装, 不运行验证不推测补齐。Reviewer 的 aria-evidence-query 脚本指引移除(Coder 保留); 不补 shell/HTTP 通道。**固有代价: 现场脚本查询维度(按文件历史/跨 artifact 聚合)覆盖不到时只能 NOT_EXECUTED, 审查深度可能下降——已接受, 但覆盖声明必须显性暴露不得无声退化。**

## 四、保留与砍掉
| v1 项 | v2 决定 |
|---|---|
| T1 提交分类 | 保留 P1(次于主闸) |
| T3 纪律 | 保留重写 P0 |
| T4 交付接线 | 保留卫生复验+新增窗口守卫 |
| T6 group 纪律 | 保留提前 P0 |
| **T2/T5 副本机制** | **正式废除**(已验证 worktree src 无残留引用, 纯删计划) |
| spawn_with_boundary 接入/宿主 runner/沙箱内嵌容器 | 不实施/继续否决 |
| 3158bf95 审计回归 | 原样保留 |
| 自动历史清扫/自动恢复 | 不采用 |

**6880023 存量**: 维持显式纠正提交需授权; 记录污染来源; 授权后单独显式路径纠正提交移除非源码产物再由只读 Reviewer 审; 不自动 amend/reset/revert/clean; owner 决定保留则记为存量例外。

## 五、红→绿(16 例, 实施验收要求)
R1 实际 prompt 字节全只读; **R2 pi 真白名单行为**(请求 Bash/write/git/构建→执行前不可达, 无进程文件 index HEAD 变化, 非口头不做); **R3 第三方工具旁路**(自动扩展/MCP/codemode 写哨兵→未注册不可达); R4 主闸失败关闭; **R5 pi 现场重跑**(同类审查材料→正常报告或 NOT_EXECUTED, worktree 零变化零提交); R6 生产 group; R7 legacy group 子轮; R8 dirty 基线原样过; R9 对账完整性(含只改 index/ignored target/chmod/symlink/commit 后恢复 HEAD); R10 异常收口; R11 平台 commit 通道; R12 报告采信; R13 证据衔接; R14 Coder 正常交付不变; R15 T1 回归矩阵; R16 存量与预算。
探针要求: 正向控制(read/grep 可用)+确定性负例调用(非自然语言诱导)+fresh/resume 两面+原始请求/清单/拒绝/哨兵证据/精确版本。

## 六、实施切片(13 片, 共享文件归一切片)
| 切片 | 面 | 规模 |
|---|---|---:|
| V1 新策略及审计契约 | streaming_provider+tool_policy_audit | 3-4h |
| V2 pi no-exec 实现 | pi_provider | 3-4h |
| V3/V4/V5 Claude/Codex/Kimi reviewer 策略 | 各 provider | 2-4h/2-4h/2-3h |
| V6 证明持久化与导入 | provider_capability 三件 | 3-4h |
| V7 真实 no-exec 探针 | provider_boundary_probe+it_web | 3-4h |
| V8 证据预供 | coding_evaluation_context | 2-4h |
| V9 纪律与策略注入 | prompts/provider_retry/internal_pr/group(集成人持 internal_pr_review) | 3-4h |
| V10 durable 审查窗口(新增) | coding_attempt_store/reviewer_read_only | 3-4h |
| V11 快照与退出收口(新增) | coding_workspace_engine/reviewer_read_only | 3-4h |
| V12 交付守卫与 T1 | git_workspace_service+git_operation(接口由 V9 集成人接) | 3-4h |
| V13 契约与真实验收 | 方案/废止标记/验收记录(controller 统一执行 R2-R16) | 2-4h |

编排: V1 后 V2-V5/V6/V8/V10 可并行; V7 依赖策略/证据接口; V9/V11/V12 由集成人按共享契约接线, **不得出现「先上线 prompt 仍留 raw Bash」半切换**; 最终验证 controller 一次执行。**P0 完整交付边界=主闸+纪律+对账/采信+提交守卫; T1 其后提交但不以 T1 宣称零写成立。**

## 开放假设与关闸条件
1. 未验证 pi 参数/扩展观察接口真实行为与握手时序(V2/V7 必须闭合, 失败不开放该 profile)
2. 未验证其他 provider 完整 no-exec 面(denylist/read-only sandbox/Terminal 拒绝不能替代全通道证明)
3. 状态快照是检测非期间零写证明; 零写主保证来自工具面撤权
4. 证据预供不能完全替代按需查询; 未覆盖维度显式报告, 不恢复执行通道补齐

> **状态更新(2026-10-16 深夜)**: 本方案 no-exec 主闸方向已被 owner 推翻, 由 v3.0 整体替代
> (见 2026-10-16_技术方案_Reviewer能力全开与铁律硬化_v3.0.md: 能力全开+独立 UID/窄挂载铁律+可恢复快照)。
> v2 的快照对账/窗口持久化/提交通道守卫设计被 v3 沿用升级。
