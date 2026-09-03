# 95% 成功率口径与当前基线(方案 b,2026-09-03 v1)

> 依据:路线图 v2.0 §3.5(阶段 2 遗留测量);oracle 裁决 4(2026-09-03,my-anthropic/k3):当前不加跑,只建口径+基线+缺陷族清单。
>
> 🔴 **口径警示(防误读)**:本表样本非 i.i.d.(策略/语料/prompt 版本/会话状态混杂),数值仅描述已发生事实,**不得**当作回归基线或质量结论引用;引用必须连同本节的样本构成说明。

## 1. 口径定义

- **产物级 pass/fail**:单次真实跑按阶段 2 6.2 判据,plan/draft/review 产物全链走到 durable Confirmed = pass;其余(含合法终态失败)= fail。
- **同构要求**:同一 fixture set+run policy+prompt 版本下的跑才可合并;跨口径只分列不合计。
- **95% 声称门槛**(rule of three):需约 **60 次零失败**同构跑;当前任何口径都远未达标,**本轮不声称任何成功率**。
- 无效样本:驱动侧配置污染的跑(Ruling T3-A,预试验 v1 ×4)不入统计,另行注记。

## 2. 本轮基线(2026-09-03,interactive/auto 如标注)

### 2.1 按语料×策略分列(plan 级有效 15 跑)

| 口径 | 跑次 | pass | 通过率 | 明细 |
|---|---|---|---|---|
| 轻语料(minimal/defect)+interactive | 6 | 3 | 50% | pretrial2 codex/pi ✅✅;amendment rep1❌rep2✅;final rep1❌rep2❌(空交付) |
| 重语料(levels)+interactive | 7 | 0 | **0%** | matrix1:codex 死路(产品,已修)❌/codex 首稿 27 错❌/pi 循环超时❌/pi 中文标题❌;matrix2:codex invalid_id❌/codex duplicate_id❌/pi 循环超时❌ |
| 重语料(levels)+auto_if_valid(kimi 终局) | 2 | 0 | 0% | 前导语替代交付❌/零输出空交付❌ |

### 2.2 按 provider(codex 9/pi 3/kimi 2,仅本轮)

| provider | 跑次 | pass | 主失败形态 |
|---|---|---|---|
| codex | 9 | 1(11%) | 首稿结构缺字段/重复·非法 ID/零输出空交付(新子形态)/死路(已修) |
| pi | 3 | 1(33%) | 修订循环不收敛×2 |
| kimi | 2 | 0 | 前导语替代交付/零输出空交付 |

### 2.3 历史对照(阶段 2,levels+auto_if_valid,无人工门;/tmp/aria-phase2-results,27 份 result)

| 轮 | codex | pi | kimi |
|---|---|---|---|
| r26 | ✅ Confirmed(143s) | ✅ Confirmed(938s) | ❌ stopped_needs_human |
| r28 | ✅ Confirmed(627s,中文 plan) | — | ❌(r28b/c 教学链失败) |

**关键对照(oracle 纠偏口径)**:同一 levels 重语料,阶段 2 auto_if_valid 下 codex 2/2、pi 1/1 Confirmed;本轮 interactive 0/7。**主变量=provider 输出合规性(随策略/prompt/会话状态漂移),语料重量是放大器而非根因**。阶段 4 provider 策略不得据此错杀 levels 语料。

## 3. 缺陷族清单(治理前置:以下各族修复前,95% 专项测量无意义)

| # | 族 | 归属 | 状态 |
|---|---|---|---|
| 1 | 零输出空交付(codex 新子形态+kimi 既有) | provider/CLI 层 | 未治理;codex 连续 2 次,CLI 会话性漂移嫌疑,证据不足定因 |
| 2 | 前导语/路由污染交付(kimi 第四形态) | provider 教学家族 | 登记;教学候选「工具被拒不阻塞交付」 |
| 3 | 首稿结构缺字段(requirement_refs/requirement_id/中文标题/重复·非法 ID) | provider 输出合规 | 8.2 同族延续,本轮重语料主死因 |
| 4 | 修订循环不收敛(pi,×2) | provider/时长 | 8.2 pi rep3 同族 |
| 5 | 第 5 死路(修订后缺 Evaluate route) | 产品 | ✅ 已修(dc4d0d0d,过审;真实行使待未来到门跑) |
| 6 | 接缝(journal 收养 400) | 产品 | ✅ 已修(1e7a7247,过审,3/3 验证) |
| 7 | 3a worktree 未物化 | 产品 | ✅ 已修(17c7eccc,过审;终局跑未到达=零回归信号亦零验证) |
| 8 | 3b 半启动互等无恢复 | 产品 | defer(阶段 4/恢复矩阵家族) |
| 9 | 判据 A(coding 段首测) | — | defer(卡在族 1;编码链=条件项 4 主场) |
| 10 | 门预算语义家族分叉(amendment 接续 vs 原门重置) | 契约 | defer 挂账(specs 已补句记录,REQ-CG-02) |
| 11 | 95s 空闲断连→ws_closed 污染 failureClass | 产品观察 | defer(阶段 4 定语义) |
| 12 | confirm 失败吞 strict validator findings(WS 单句) | 产品观察 | defer(可观测性) |
| 13 | flaky:kimi_code_provider/terminal 进程组超时测试 | 测试债 | 两轮各撞 1 次,定向复跑均过;阶段 4 顺手清 |

## 4. 结论

- 当前**不声称**任何成功率;n=15(本轮)/n≈29(含历史,口径异质只分列)。
- 下一有意义动作(阶段 4 或提示词教学 change):治理族 1-4 后,以**固定口径**(单 fixture set+单策略+单 prompt 版本)重开 95% 专项测量,预算 ~60 跑。
