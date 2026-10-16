# 技术方案: Git 提交卫生与 Reviewer 构建落位修复(v1.0)

- 出具: design-task-sol (GitHygieneFix), 2026-10-16, 受测试会话委托; **设计完成未实施(DONE_WITH_CONCERNS)**
- 源码引用均经物理文件读取复核; 只读分析, 无 commit/测试/构建
- 关联: cadence/notes/2026-10-16_分析-pi与kimi执行环境差异.md 第4节后置发现(#6/#7)

## 一、结论

两个独立可分别交付的修复:

1. **代管提交: 先分类后显式暂存, 可疑新增产物可见停等, 不静默猜测**
   - Git 标准忽略的全新文件不自动提交; `.local/ target/ node_modules/ dist/` 等是**疑似运行产物信号**(非「永不交付目录」); 未忽略的新增疑似产物默认在暂存/提交/推送前阻止并列路径与理由; 已跟踪文件不因目录名或后加 ignore 被静默取消; 有意交付同名新文件走既有 write_policy **文件级明确声明**, 不新建通用 allowlist
2. **Reviewer 与 Coder 工作区纪律拆开**
   - 分支 A(pi 保持现状): Reviewer 只读原 worktree; 复跑的构建/安装/测试在 **worktree 外平台托管源码副本**执行, 平台管目录分配/复制/生命周期
   - 分支 B(pi 接入 spawn_with_boundary 后): 不改沙箱; 安全方案落地后移除分支 A 宿主构建路径, 保留历史残留治理与提交卫生

**制度性根因**: 现有公共纪律**明文要求 Reviewer 在原隔离 worktree 内复跑**(prompts.rs:408,491-497; 注入点 provider_retry.rs:487-495)——平台把「不能污染主 checkout」实现成了「所有验证写回交付 worktree」, 未区分 Coder 交付区与 Reviewer 验证区。

### 现场事实修正
- 6880023 在 **busi** worktree(非 frontend); 仅新增 .local/share/containers/storage/db.sql(114688B)
- `git check-ignore -v --no-index` 无匹配——**已跟踪且不能靠现有 ignore 规则解释**(不是再调一次 gitignore 能解决)
- 现场 Reviewer 报告自记该文件已在既有提交内(0104_output.txt:239-242); frontend git-operation.json 指向 fbac52d, 非 6880023 的 journal, 勿误用

## 二、改动点清单(摘要)

| 面 | 文件:符号 | 改动 |
|---|---|---|
| 提交 | git_workspace_service.rs:317-361,907-914 | 分类在前显式暂存在后; should_exclude 拆「既有平台产物排除」+「疑似新增分类」; 候选检查/稳定错误/结果结构 |
| 提交接线 | internal_pr_review.rs:643-761 | execute_review_request 传权威文件级例外; Before 与 CommitStarted 恢复路径均卫生复验; 阻断落可见状态 |
| **新增** | coding_workspace_engine/reviewer_scratch.rs | 平台托管副本/清理/残留隔离内聚实现 |
| 纪律 | prompts.rs:395-424,491-518 | 拆 Coder/Reviewer 纪律; 替换「必须在原 worktree 内复跑」条款; Reviewer 专用运行期注入段 |
| Reviewer 接线 | provider_retry.rs:475-554; internal_pr_review.rs:316-354; group_review_prompts.rs:81-218; group_review_orchestrator.rs:1047-1125 | 每 invocation 创建/绑定副本; group 静态纪律进 PromptSegments.fixed_protocol, 动态路径走环境字段 |
| 残留治理 | coding.rs:47-56; rework.rs:34; provider_retry.rs:328-378 | 下次 Coder 首启/返修/重试前残留检查+补偿清理(身份租约校验后) |

不改: bwrap 参数、REQ-LCG-04、provider capability 安全裁决、root-recipe 预算数值、gitignore 匹配实现。

## 三、方案细节(要点)

### 3.1 提交卫生
- 现缺口: 现流程=可读性检查→`git add -A`→取消暂存极少数产物(.aria 特定形式/__pycache__/.pyc)→commit; db.sql 未被 ignore 即被全量暂存
- 分类以 **HEAD 是否存在**为界(git add -f 也能把新产物放进索引, 不能只看索引); 模式按**路径段/目录语义**匹配(target/ 不误判 targeting.rs)
- 文件级例外: 命中疑似模式的新文件必须由权威 write_policy **明确列出该文件**; `**`/`dist/**` 宽泛范围不构成例外; 不据自然语言报告/自述/「已暂存」推断
- 暂存顺序: 只读枚举(路径 NUL 分隔)→分类→**索引变更前失败**→仅允许候选可读性检查→显式暂存→commit 前重读复验; CommitStarted 恢复同样复验(不因跳过 Before 漏检); push 前对新增提交的疑似新增路径复验
- 不因失败 reset/amend/revert; 历史污染提交不自动重写; 阻断信息=稳定原因码+路径+分类理由+解决路径(复用 blocked gate/timeline, 不只抛日志)
- 不推荐: 命中目录一律取消暂存(静默吞交付物)/仅 warning 后继续(挡不住 6880023 再发生)

### 3.2 分支 A: 平台托管副本
- 目录: `<宿主安全临时根>/cadence-aria-review/<实例>/<project>/<issue>/<attempt>/<role_run>/<invocation>/{source,home,tmp,cache}`; 每 invocation 独立; **必须物理位于 filesystem audit 扫描范围外**(canonicalize 验证不在 LC root/checkout/worktree/.git 内; scratch 无 .git 不依赖 gitignore, 不给 auditor 加全局排除入口)
- 复制=**当前受审状态**(工作区实际内容含未提交修改/删除), 非完整历史/非 clone; 排除 .git/令牌/target/node_modules/dist/.local; 同名目录内被声明源码仅复制那些文件; 禁硬链接(构建回写原 worktree); 不跟随逃逸 symlink; 构建依赖 git 元数据时传只读 revision 信息
- 环境隔离: HOME→home/, TMPDIR→tmp/, XDG/缓存→cache/; maven/npm 缓存与容器可写挂载只指向 scratch——防 target/.npm/.local 回落交付目录
- 纪律文案: prompts.rs:408 替换为「仅在证据缺失/矛盾/明确要求时复跑; 原 worktree 只读; 复跑在平台副本; 禁原 worktree 可写挂载/HOME/TMPDIR/缓存/构建输出」; Coder 纪律保留不扩权; **Reviewer 不再注入 Coder 提交责任**(现 provider_retry.rs:493-495 有「提交责任归你」段, 一并拆); 执行形态=读源 ARIA_REVIEW_SOURCE / 构建 cwd ARIA_REVIEW_SCRATCH/source/<成员>
- 注入不动 renderer/hash: 沿 prompts.rs:502-513 运行期注入先例; group 静态规则进 fixed_protocol(在 measure() 预算内), 动态绝对路径走 executor 环境字段——不在预算计算后偷偷追加
- 生命周期: 身份租约校验后分配副本→构造 prompt/input→启动 provider→**进程树退出后**清理(正常/失败/取消/超时); 崩溃遗留由下次同 issue 与下次 Coder 启动补偿清理; 不删活跃并发目录; 未知租约停等; 不靠 Drop 删目录(竞争); 清理失败可见记录

### 3.3 分支 B: pi 收紧后
不实施本次切换。接线后: 保留只读纪律; 不启动分支 A 宿主构建路径; 验证不可执行按真实限制报告, **不允许失败回退普通 spawn**; 历史治理与 Bug① 保护保留。预期消失的是「宿主 rootless podman 构建+写回 worktree」能力, 不妄称沙箱内一切编译不可行(由分支 B 验收确认)。

### 3.4 历史残留治理(两分支共用)
**时机=下次 Coder 启动前**(首启 coding.rs:47-56/返修 rework.rs:34/重试 provider_retry.rs:749-771 三入口), 早于材料收集。分级: 平台登记非活跃 scratch→清; 已确认旧 Reviewer 产物未跟踪 target/→隔离到审计外 quarantine 再删; **已跟踪 db.sql(6880023)→不自动删不 reset, 纠正提交需显式授权+审计链**; 归属不明 untracked→保留列出人工停等(禁 git clean -fdx); 文件级声明合法文件→不动; 活跃/未知租约→停等。
**不宣称 root-recipe audit 是每次 Coder 启动前置**(实读生产调用属 aggregate initialization, 无同链证据); scratch 排除靠物理路径保证。

### 3.5 与 3158bf95 的关系
既有修复已 gitignore-aware(计条目/字节前跳过命中; 20000 条/64MiB 默认; 70MiB node_modules+45MiB target 回归)。**不重复设计、不提预算**。剩余风险(未忽略 .local/.npm 仍计预算/产物占磁盘可见/ignore 不解除已跟踪/审计排除≠提交安全/scratch 若入扫描树重造风险)正是本方案转向: **从预算救火转向上下文卫生+交付卫生+副本生命周期**。

## 四、风险与兼容
新增可见停等=有意行为变化(过去能成功, 现在在污染进历史前阻断); ignore 与 tracked 语义不混用; 新文件误判有确定出口(文件级契约); 副本有 I/O 但不复制大产物; **副本不完全等同原环境**(缺 git 元数据/外部输入时报证据限制, 不伪造失败不回退原 worktree); **分支 A 是卫生约束不是安全沙箱**(零沙箱 pi 仍可违背 prompt); 在途 journal 不重写; 实施提交需同步纪律文档与 changelog。

## 五、实施切片
| 切片 | 文件面 | 规模 | 独立验收 |
|---|---|---:|---|
| T1 提交候选分类 | git_workspace_service+测试 | 2-3h | 分类矩阵/显式暂存/索引失败前不变 |
| T2 副本与残留服务 | 新 reviewer_scratch.rs+mod | 3-4h | 副本内容/路径在审计树外/清理并发隔离 |
| T3 纪律文本 | prompts.rs | 1-2h | Coder 不扩权, Reviewer 无原 worktree 构建/提交要求 |
| T4 交付接线 | internal_pr_review+范围 helper+fallback 测试 | 2-3h | Before/CommitStarted 均不能绕检查 |
| T5 CodeReviewer/Coder 补偿 | provider_retry/coding/rework/execution_context | 2-3h | fresh/resume/rework/retry 全覆盖 |
| T6 Group 接线 | group_review_prompts/orchestrator/budget | 2-3h | shard/reduction/repair 文案与真实发送字节均入预算 |

公共文件由一个集成人串行修改; 分支 B 安全接线不列入。

## 六、红→绿(要点)
提交卫生 9 例(.local 阻止/有意新源码正常/已跟踪 target 改写不静默丢/宽泛范围不能绕/ignored+add -f 不视为授权/纯产物不产生 feat 提交/CommitStarted 恢复复验/历史污染不自动重写/权限删除路径编码); 分支 A 8 例(prompt 实际字节分离/C 构建只落副本/缓存落 scratch/副本含未提交态/进程树退出后清理/并发 shard 独立/scratch 配审计树内必须失败/哈希预算不绕); 分支 B+治理(pi 真实用边界无回退/旧 target 首启前清理/db.sql 不被清扫删/未知归属停等/旧 scratch 精确清理/保留 3158bf95 回归/scratch 路径集成验收)。

> **状态更新(2026-10-16 晚)**: 本方案 Reviewer 构建落位方向(T2/T5 及 reviewer 面纪律)已被 v2.0 废止替代
> (见 2026-10-16_技术方案_Git提交卫生与Reviewer纯只读权限收敛_v2.0.md); T1/T4 提交卫生部分保留(P1)。
