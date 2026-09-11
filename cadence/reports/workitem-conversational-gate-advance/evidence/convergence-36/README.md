# 3.6 矩阵汇总表（convergence-36 总览）——截至 2026-09-11 晨

> 状态：**7/9 闭环 + kimi×重 进行中**。模型基线=glm-5.3-flash（用户裁决，四 provider 统一）；判据 C1/C2/C3 见 `cadence/plans/2026-09-04_计划文档_3.6全量收敛轮_v1.0.md`；SOP v15=levels+双脚本+60min workitem 预算+90min coding 预算。
> 细节权威=台账 `.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/progress.md`；逐格证据=本目录各子目录 README。

## 1. 矩阵终盘

| 格 | 状态 | 收官序列 | 证据 |
|---|---|---|---|
| pi×轻 | ✅ 3/3 | rep1m2/rep2m/rep3m（issue 0142-0144） | pi-light/ |
| pi×重 | ✅ 3/3 | v5 rep1c/2c/3c（0163-0165，双脚本） | pi-heavy/ |
| codex×轻 | ✅ 3/3 | rep2c/3c/4c（0155-0157） | codex-light/ |
| kimi×轻 | ✅ 3/3 | rep1v9/rep2v10/rep3v10（0175/0182/0183） | kimi-light/ |
| claude×轻 | ✅ 3/3 | rep4/5/6v12（0190-0192，claude 首秀首通） | claude-light/ |
| claude×重 | ✅ 3/3 | v15 rep5/rep6/rep7c（0203/0204/0207；coding 加分 2/3 全链） | claude-heavy/ |
| **kimi×重** | ⏳ **进行中** | v15 战绩 5过/5悬（rep1b/2/4/6/8 过；0199×3+语法×1+空输出×2 悬），最大连胜 2；通道 09-11 坏（用户测试中） | （未入库，恢复后续磨） |
| codex×重 | 📋 限制归档 | 0/10+适配器逐字节等价实锤=模型行为差异 | codex-heavy-limitation/ |
| F6 amendment | 📋 3 跑预算耗尽 | 判据 B 未达：语料强化生效（缺陷浮出 2/2）但 InvalidRepairTarget×1+路由分歧×1 拦在 plan_repair 前 | f6-amendment/ |

## 2. 3.6 修复环 commit 链（全部 TDD+k3 过审+部署）

v13 前 22 环见交接文档 §4。本轮（09-10/11 新会话）新增：

| commit | 内容 | 部署 |
|---|---|---|
| 23cdeb55 | **SC 误燃顶杀根修**（oracle 候选6：followups 循环 SC 委托守卫；红锚「reviewer 启动 2→1」） | v15（92c6ee9b，PID 183996） |
| 6c236d59 | F6 defect 语料强化（REQ-003 三重栅栏，digest 重钉 083c9e30） | —（driver 侧） |
| e7f737e6/a2f777ee | claude×重/F6 证据入库 | — |

部署谱系：…→v12(be08e161 双spawn)→v13(de12b197 kind 打点；机器重启丢失)→**v14**(0cf3aad0 重建，源码等价)→**v15**(92c6ee9b 误燃根修)。v14/v15 三对账均过（PID/exe/md5/health）。

## 3. 95-baseline 缺陷族状态更新（对照 95-baseline-v1.md §3）

| 基线族 | 3.6 后状态 |
|---|---|
| 1 零输出空交付 | 未根治：glm/kimi 上游波动族，09-11 kimi 通道整段坏（CLI 直测 90s 零回复）；SOP=重跑不计缺陷 |
| 2 前导语污染交付 | claude 首秀暴露（轻 rep1v12/重 rep4）：教学重驱恰一次后仍概率犯，重跑收敛；anti-preamble 教学预算 21000 余 18B，再教先提额 |
| 3 首稿结构缺陷 | **大幅收窄**：结构化硬化三件套（契约 8 变体逐字枚举+coder/reviewer 教学重驱）后，枚举/verdict 缺失近绝迹；残余=levels 契约网打地鼠（缺口游走+contract 跨修订重编号，0199 形态，四 provider 重格均现，靠多轮修订+人工门收敛） |
| 4 修订循环不收敛 | **已治理**：契约前移修订循环（e929aa93 机械 findings 回灌+指纹闸）+reviewer 预算 64→96KiB+author 能力覆盖教学（21000）后，pi×重/claude×重 3/3 收敛 |
| 5-7 产品已修族 | 零回归；误燃顶杀（新族，v11 双 spawn 补集泄漏）已根修 23cdeb55 |
| 8 恢复族 | 部分：resume 补发+断连不杀 run（v11+）落地；compile 拒后 active_node 对齐仍在 4 号修法包 |

**3.6 新增族（进测量报告 v2）**：①InvalidRepairTarget/route×target 语义（7 例三 provider 全中，F6 判据 B 与重格 coding 加分项共同拦路虎，教学需授权）②SC 误燃顶杀（已修）③兜底 commit provider 相关（kimi 不自行 commit→runner 兜底扛住）④kimi terminal cwd 绝对路径（已修）⑤claude 三层修复链（信任+断连+双 spawn drain）实战有效。

## 4. 待办（收官剩余）

1. kimi×重 3 连跑（通道恢复后）
2. codex×重 处置（用户 a/b/c/d）+ F6 处置（用户三选）
3. 测量报告 v2（弱模型基线发现全录）+ 全部证据核对
4. 统一 squash → 3.7 UI 立项
