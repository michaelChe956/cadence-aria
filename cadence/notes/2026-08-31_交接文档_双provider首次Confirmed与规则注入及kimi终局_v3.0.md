# 交接:双 provider 首次 Confirmed + 项目规则内容式注入落地 + kimi 终局待跑(2026-08-31 v3.0)

> 一句话:阶段 2 迎来历史性突破——**codex 与 pi 首次全链 Confirmed**(r26),项目规则内容式注入落地并实跑验证(codex 中文 plan,r28);kimi 经三轮教学修复后**终局验证轮未跑完**(r28c-rep2 被叫停),新会话第一件事就是跑它。所有上下文、证据、决策、操作手册都在本文件。

## 0. 新会话入口(照此执行,不丢步)

1. 读本文件 → 读 `.superpowers/sdd/2026-08-27_计划文档_WorkItem阶段2单候选C′MVP_v1.0/progress.md`(台账,末尾有本会话记录)→ 需要时读上一份交接 `cadence/notes/2026-08-30_交接文档_scope修复v1落地与r23二实例现场_v2.0.md`
2. 确认 worktree git 干净、HEAD `d192755e`;确认服务器状态(见 §1,当前运行的已是最新二进制,kimi 终局轮可直接跑,**无需重建重启**)
3. **首要任务:跑 kimi 终局验证轮**(§5 手册)→ 过=6.2 三 provider 全达标;不过=按用户裁决记录 2/3(B 方案,用户已预批「若仍不过,接受 2/3+合法终态记录」)
4. 之后:6.3 golden 核对 → 6.4 验收报告+全分支终审+OpenSpec 勾选/sync/archive → push → 收官(§6)

## 1. 仓库与工作区状态

- 仓库 `/home/michaelche/workspace/github/cadence-aria`,worktree `.worktrees/feat-b-0808-add-monorepo`(分支同名)
- HEAD `d192755e`(nonce-first reviewer output and atomic capability teaching),git 干净,**全部本地未 push**(D4 裁决:终审后统一推)
- 本会话 12 个实现/修复 commit + 1 个 docs commit(§3 清单)
- 服务器:**正在运行** PID 3241606,端口 4317,`--work-item-plan-single-candidate`,二进制=`d192755e` 构建(worktree 绝对路径),日志 `/tmp/aria-phase2-server-r28c.log`
- 🔴 **启动铁律(不变)**:nohup 必须用 worktree 绝对路径二进制(bash 默认 cwd 是主仓,相对路径会启动主仓旧二进制→HTTP 500)。命令:
```
nohup /home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/target/debug/aria web --workspace /home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo --host 127.0.0.1 --port 4317 --work-item-plan-single-candidate >/tmp/aria-phase2-server-r28c.log 2>&1 </dev/null &
```
(单条执行;然后单条 sleep 75,单条 curl health;`ls -l /proc/<pid>/exe` 核验)
- 若代码有新 commit 才需重建:`kill <pid>` → worktree 内 `cargo build --locked` → 按铁律重启

## 2. 本会话(2026-08-30 午 ~ 2026-08-31)完成事项总览

1. 读完 v2.0 交接 → 三轮 scout 钉死 scope 第二实例根因(内存/durable 失步+ensure 内存短路)→ oracle 推荐 A′+B → 用户批准 → ter-worker TDD → **`f8cbd522`** → r23 三 provider 实证零 scope 违规
2. r23 暴露新层(能力覆盖缺口)→ oracle 裁 B1+B2(B3) → B1 author 教学 **`e78094a9`** + OpenSpec delta **`d87d0d7a`** + B2 reviewer 投影(Plan 由 sol-worker 写,含两次批准修正:helper 迁移/双点预算)**`0f47060d`**(B2 含 Important 补强:Verification 复评断言)
3. r24 暴露 unconsumed_required_handoff + catalog 残留 → scout 全量规则清单(60+ 类)+ oracle 裁 P2+P4 → **`6d74242a`**(P4 独立)+**`4e031552`**(P2)
4. r25 两案暴露教学引发的两轮泛化(删字段→清空 Outputs)→ oracle 反证修正(`[]` 本就合法,legacy prompt 有约定但 SC 重写丢失)→ **`22fa7eb1`**(终端 `[]` 教学,又引 Outputs 空)→ oracle 审措辞 → **`f5fa46e5`**(去终端框架+Outputs 守卫)→ **r26 突破:codex+pi 首次 Confirmed**
5. 用户提出「按阶段获取项目规则」平台设计 → researcher 事实调研+sol-worker 设计草案+oracle 评审(5 Critical)→ **用户裁决:平台方案太复杂暂缓,只做简化三步**(SC 链路规则内容式注入)→ **`67aa456f`**(language 全文+优先规则句+code-usage/code-reading 摘要+fail-closed+预算 19,000)
6. r28 验证:codex 首跑环境方差(900s 流式卡 6.7KB)复跑即过 → **中文 plan 验收通过+Confirmed(627s)**
7. kimi 修复链:r28-rep2 全角冒号(语言规则泛化到标点)→ **`61876332`**(半角分隔符教学);r28b 路由回执污染 nonce 输出(kimi 顺指针读 AGENTS.md 把「首段回执」写进结构化输出)+ 能力合并字符串假性 missing(kimi reviewer 自己诊断出的)→ **`d192755e`**(nonce 首行教学+SC 确定性前言修剪+原子拆分教学)
8. interactive 人工门测试(用户要求):driver 扩展 **`74b8e512`**(ARIA_HUMAN_SCRIPT/stdin/审计)+ needs_human 门误判修复 → r27 实测**人工门打通**(request-change 带中文反馈成功)→ 暴露 SC manual revision 缺口(走 legacy 中文-heading artifact 约束)→ **用户裁决 defer 阶段 3**
9. kimi 终局轮:r28c-rep1 hard_timeout(kimi 环境层慢)→ rep2(1200s)被用户叫停 → **待新会话跑**

## 3. 本会话 commit 清单(全部已过 reviewer/亲验门禁,未 push)

| commit | 内容 | reviewer |
|---|---|---|
| `f8cbd522` | scope v2:ensure durable-first 单一物化+删返修评估 Verification 升级 | Approved |
| `e78094a9` | B1:SC author 能力覆盖教学+SC 专属预算 16,200 | Approved(4 Minor defer) |
| `d87d0d7a` | OpenSpec delta:REQ-WSC-06 增补(能力覆盖投影) | 用户批准 |
| `0f47060d` | B2:reviewer 能力覆盖投影(同源复用 validator)+64KiB 双点预算 | Approved+补强 |
| `6d74242a` | P4:SC 路径退役 trusted catalog 残留(8-29 裁决收尾)+P2+P4 契约/Plan | Approved |
| `4e031552` | P2:handoff 消费闭环 author 教学+reviewer 全图投影扩展 | Approved(3 Low defer) |
| `22fa7eb1` | 8.2-fix:终端 handoff `[]` 教学(后证仍致 Outputs 空) | (8.2-fix 补丁直收) |
| `f5fa46e5` | 8.2-v3:去终端框架+[] 作用域钉死+Outputs 永不为空守卫 | 同上 |
| `67aa456f` | 项目规则内容式注入:language 全文+优先规则句+双摘要+fail-closed+预算 19,000;含契约 8.3-fix+平台设计文档(defer 标注) | Approved(4 Low 2 Info defer) |
| `74b8e512` | campaign driver 人工门模拟(脚本/stdin/审计)+needs_human 门误判修复 | Approved(2 Low 1 Info defer) |
| `61876332` | 半角分隔符教学(全角冒号泛化修复) | 直收(一行教学,门禁亲验) |
| `d192755e` | nonce 首行教学+SC 前言确定性修剪(preamble_trimmed)+能力原子拆分教学 | 直收(门禁亲验全绿) |
| `e4343834` | (上一会话)交接 v2.0 | — |

**OpenSpec 契约累积修改**(均在 `openspec/changes/rearch-workitem-plan-pipeline/`):REQ-WSC-06 多轮增补(能力覆盖投影→handoff 闭环+终端 `[]` scenario→内容式注入 8.3-fix);tasks.md §7/§8+8.2-fix+8.3-fix+defer 登记;design.md P2+P4 裁决/终端空数组裁决/规则注入裁决/≤12min 张力登记。

## 4. Campaign 实跑结果记录(判据:confirmed_count==1、Confirmed、时长≤超时、cycle≤1、旧决策空、ledger 无重复)

| 轮 | provider | 结果 | 时长 | 备注 |
|---|---|---|---|---|
| r23-postV2 | codex/pi/kimi | 三案零 scope 违规;codex R2 pass(死点突破);暴露能力覆盖缺口 | — | scope v2 验证通过 |
| r26 | **codex** | ✅ **Confirmed**(R1 直接过,零返修) | 142.96s | **campaign 历史首例**;issue_0075/session_0080 |
| r26 | **pi** | ✅ **Confirmed**(R1 直接过) | 938s | issue_0077/session_0085;首跑 ws_closed 复跑即过;>12min(退役门张力已登记) |
| r26 | kimi | ❌ stopped_needs_human(4 轮 revise 全能力覆盖缺口) | 431s | reviewer 拦截正确;B2 投影在工作 |
| r28 | **codex** | ✅ **Confirmed**+**中文 plan**(规则注入验收) | 627s | 首跑环境方差(900s 流式卡 6.7KB)复跑即过;issue_0084;plan 见 `/tmp/r28_codex_plan.md`(内容中文+结构英文) |
| r28b/c | kimi | ❌ 全角冒号(r28-rep2)→ws_closed(r28-rep1)→路由回执污染 nonce(r28b)→hard_timeout(r28c-rep1)→**rep2 被叫停** | — | §5 |
| r27-interactive | kimi | 人工门打通(request-change 成功);SC manual revision 缺口暴露 | — | §7 |

**durable 证据**(勿删):issue_0075/0077(r26 双 Confirmed)、issue_0084(r28 codex)、issue_0063/session_0068(scope 修复素材,旧)、r27-interactive 各 issue(人工门链路)。产物目录 `/tmp/aria-phase2-results/`(r23-postV2 起各轮全留)。

## 5. kimi 问题全景与终局轮手册(新会话第一件事)

**kimi 失败轨迹**(每一环都已修或已定性):
| 轮 | 死因 | 处置 |
|---|---|---|
| r26 | 4 轮 revise 全「能力覆盖缺口」——真根因=author 把多条能力合并为单一字符串,require_all 精确匹配下假性 missing(kimi reviewer 自己诊断出) | ✅ `d192755e` 原子拆分教学 |
| r28-rep1 | ws_closed(环境层瞬断) | 复跑纪律 |
| r28-rep2 | 全角冒号(`- key： value`)——语言规则泛化到标点 | ✅ `61876332` 半角强制 |
| r28b | 结构化输出前写「工作流路由:…」回执(kimi 顺 [cadence_project_rules] 指针读 AGENTS.md 应用路由规则)→nonce 解析失败→needs_human+findings 全丢 | ✅ `d192755e` nonce 首行教学+SC 前言确定性修剪 |
| r28c-rep1 | hard_timeout 900s(kimi 环境层慢,生成都没完成) | 加时限重跑 |
| r28c-rep2 | **被用户叫停(command aborted)** | ← 新会话从这里继续 |

**终局轮手册**:
1. 服务器已是 `d192755e` 二进制(PID 3241606),health 通过即可直接跑;若已死按 §1 铁律重启
2. 注意 `/tmp/aria-phase2-results/r28c/kimi_code-rep2/` 有被中止的残缺产物(driver 不覆盖旧目录会 EEXIST)——**用全新目录名**(如 r29/kimi_code):
```
cd <worktree> && ARIA_DATA_ROOT="$PWD/.aria" ARIA_BASE_URL=http://127.0.0.1:4317 ARIA_EXPECTED_FLOW_KIND=single_candidate ARIA_RUN_POLICY=auto_if_valid ARIA_WORKITEM_HARD_TIMEOUT_MS=1200000 node cadence/reports/workitem-coding-campaign/workitem_run_campaign.mjs kimi_code 1 /tmp/aria-phase2-results/r29/kimi_code
```
3. 时限用 **1200s**(kimi 生成慢是实态:678-900s 区间)
4. ws_closed/流式卡顿→复跑一次定性(环境层,kimi/pi 本会话各 2/1 次,codex 0 次但有卡流 1 次;全部重试即恢复)
5. **判读**:Confirmed → 6.2 三 provider 全达标闭环;再 stopped_needs_human/其他产品失败 → 看失败类:若是**新教学未覆盖的形态**,先报用户裁决(用户已预批:若仍不过,接受 2/3+合法终态记录=6.2 判据修订为「codex+pi Confirmed;kimi 合法终态且拦截机制正确」;记录会写明是教学迭代成本非系统缺陷);若是环境层→再复跑
6. 期间顺带核 plan 是否中文+半角冒号(验证 `61876332`/`67aa456f` 对 kimi 生效)

## 6. 阶段 2 剩余路径(kimi 之后)

1. kimi 终局(§5)→ 6.2 达标口径落定
2. **6.3**:核对阶段 1 的 14 条 classifier golden(9 reviewer finding 映射+2 Advisory+3 class_hint 变体)+ compiler diagnostic golden(仅 grammar/lowering 类)+ usage/token 基线对比(含注入内容占比)
3. **6.4**:验收报告落盘(`cadence/reports/`)+ 全分支终审(清 §8 defer 账)+ OpenSpec 勾选/sync/archive(`rearch-workitem-plan-pipeline` 整个 change 归档)
4. push(用户 D4:终审后)+ 移交阶段 3 立项(对话流人工门与 advance 接口,连同登记议题:发布后计划变更×已完成 coding 管理、95% 测量方案 b、SC manual revision)

## 7. interactive 人工门测试结论(用户问题 1 的完整答案)

- **已打通**(r27 实测):R1 needs_human → 人工门 → driver 代真人发 `human_confirm request-change`(带中文反馈文本)→ 服务端接收进 revision;审计完整(`humanGateActions`/ws 事件)
- **断点**:SC 的 manual repair revision 走 legacy 链,被旧中文-heading artifact 约束(`artifact_constraints.rs:189-215`,「计划范围/任务拆分…」)拒绝 SC 格式修订 → SC 无自己的 manual revision 路径(auto 22 轮从未走过)
- **用户裁决:defer 阶段 3**(对话式人工门重做时一并实现);已登记 openspec tasks.md
- driver 能力(`74b8e512`):`ARIA_HUMAN_SCRIPT='request-change:<文本>;confirm'`(按门次序消费,耗尽默认 confirm)+`ARIA_HUMAN_MODE=stdin`(TTY 真人输入)+仅 interactive 生效,auto 零影响

## 8. 终审 defer 账(6.4 前必须清/裁)

1. **SC author 预算 19,000 实测 18,934,余量仅 66B——下次任何教学必先上调常量**(reviewer 建议:整百级+margin 惯例注释;consolidate 时一并做)
2. reviewer v1 4 Minor(节点轮换集成测试/phase_machine 真红标注/Err 兜底命名/旧 key 孤儿)
3. B1 4 Minor(自造词 output_capabilities 措辞/require_any 空集偏严/5 字节余量(已被 19,000 吸收)/空 design list 测试)
4. B2 2 Minor(复评四组投影字段断言(Important 已补 Verification 腿,字段组仍未)/只读投影病态重复不去重)
5. P2+P4 3 Low(legacy outline catalog/budget finding 排序偏移/Verification 复评断言完备性/同前)
6. 规则注入 4 Low 2 Info(Logical strip 静默丢弃风险/language vs 聚合政策优先级未定义/description 示例词/语言规则文件无大小上限仅 65,536 硬兜底)
7. driver 2 Low 1 Info(parse throw 分支无单测/void 火忘式调用/legacy interactive 行为扩展)
8. **pi 时长 >12min vs 退役门「≤12 分钟」张力**(design.md 已登记,6.4 用户裁决)
9. **95% 成功率测量(方案 b)**:全流程(6.2→6.3→6.4→终审→push)完成后专项轮(测量形态终审裁量)
10. Case A/B 提醒(项目规则):SC plan prompt 非 Work Item Draft prompt 家族,campaign 实跑即实测;若用户认定需按基线重验需另授权
11. 平台级规则设计文档 `cadence/designs/2026-08-31_设计文档_按阶段项目规则获取_v1.0.md`(v1.0+oracle 5 Critical 评审意见)——defer,阶段 2/3/4 完成后统一实施时参考
12. SC manual revision 缺口(defer 阶段 3)

## 9. 环境运维要点(继承+本会话新增)

- codex:0 次 ws_closed;1 次流式卡顿(12min 6.7KB,复跑即过);历史 CLI 间歇崩(rmcp→SIGKILL)重试即恢复
- kimi:2 次 ws_closed+2 次生成慢超时(678-900s 未完成)——**环境层最不稳定,终局轮用 1200s+复跑纪律**;无登录问题
- pi:1 次 ws_closed(复跑即过);全程需 1800s 上限(D2 裁决)
- 已知 flaky 测试家族(单跑即过,不改产品):kimi terminal、SC recovery failpoint 时序(finalizer/publication checkpoint matrix)、task_3_4 crash boundaries、generate_endpoints 初始化;并行全量偶发 1-3 个失败属正常,复跑定性纪律不变
- 验证纪律:worker 门禁自报常把 lib 数标错(报「400」实为 it_web 段)——controller 必须亲跑全量取数;grep -c ok=0 意味着 flaky 撞了 fail-fast,复跑再判,勿在无全绿证据时 commit
- 构建规范:`cargo fmt --check`/`clippy --all-targets --all-features --locked -- -D warnings`/`check --locked`/`test --locked`;🔴 禁 -j;定向 `--lib`
- 1200 行 large_file_guard:prompt_contract.rs 1058、prompts.rs(split_engine)约 1104、review.rs 1158——后续加测试注意余量
- 预算常量现状:SC author **19,000**(实 18,934);legacy draft 15,600;reviewer 64KiB(实 ~10K)
- sol-worker 传输故障史(2 次 HTTP/2 失败+2 次 25-40min 超时):派工必须「增量写盘」;reviewer(glm-5.3)偶发工具挂起→steer 提示跳过长命令
- 验收期常设授权(2026-08-29)仍有效:真实 provider 运行默认同意,逐次记录消耗与结果

## 10. subagent 协作规则(用户指令,继续遵守)

1. 检索代码优先 scout[user];2. 决策优先 oracle[user] 只给推荐(opus-5 持续 400,用 my-openai/gpt-5.6-sol),最终决定权在用户;3. 调研 researcher;4. 编写 plan 优先 sol-worker[user];5. 任务默认 ter-worker[user] 执行、reviewer[user] 审核;6. 尽量并行;7. 代码勘察一律派 scout,controller 只做证据整合与决策编排;8. 决策先 oracle 再用户拍板;9. 测试优先 ter-worker,controller 尽量不自测(门禁亲验除外——那是对 worker 证据的核验)

## 11. 待用户决策事项(新会话可能遇到)

1. kimi 终局轮结果裁决(若不过:接受 2/3 已预批,只需确认执行)
2. pi ≤12min 退役门张力(6.4)
3. 95% 测量形态(终审)
4. 阶段 3 立项议程(含 SC manual revision、平台级规则设计 defer 项、发布后变更管理)

## 12. 关键文件/产物索引

| 内容 | 路径 |
|---|---|
| 上一份交接(v2.0) | `cadence/notes/2026-08-30_交接文档_scope修复v1落地与r23二实例现场_v2.0.md` |
| SDD 台账 | `.superpowers/sdd/2026-08-27_计划文档_WorkItem阶段2单候选C′MVP_v1.0/progress.md` |
| Plan(28 任务) | `cadence/plans/2026-08-27_计划文档_WorkItem阶段2单候选C′MVP_v1.0.md` |
| B2 Plan | `cadence/plans/2026-08-30_计划文档_B2_reviewer能力覆盖投影_v1.0.md` |
| P2+P4 Plan | `cadence/plans/2026-08-30_计划文档_P2P4_handoff闭环与catalog残留清理_v1.0.md` |
| 平台级规则设计(defer) | `cadence/designs/2026-08-31_设计文档_按阶段项目规则获取_v1.0.md` |
| OpenSpec change(归档对象) | `openspec/changes/rearch-workitem-plan-pipeline/`(REQ-WSC-06 多轮增补+tasks §7/§8+8.2/8.3-fix+design 三裁决) |
| r26 双 Confirmed durable | `.aria/projects/project_0001/issues/issue_0075|0077/` |
| r28 codex 中文 plan | `/tmp/r28_codex_plan.md`(durable:issue_0084) |
| 全部 campaign 产物 | `/tmp/aria-phase2-results/`(r23-postV2→r28c 各轮+run logs) |
| 服务器日志 | `/tmp/aria-phase2-server-r28c.log`(当前)及各轮历史 |
| campaign driver | `cadence/reports/workitem-coding-campaign/workitem_run_campaign.mjs`(provider: claude_code\|kimi_code\|pi\|codex;已含人工门模拟) |
