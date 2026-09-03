# 专项测量轮报告(2026-09-03 v1,路线图 3.5)

> 一句话:**对话式人工门主路径已由 codex/pi 真实跑通(双 Confirmed,8.2「8 次真实跑 0 次到门」闭环);coding 段判据 A 与 amendment 完整链仍 defer(真实 coding provider 未启动)**;「工作项太重」疑点修正为「provider 输出合规性为主变量、语料重量为放大器」;本轮另修 3 个产品缺陷、挖出 2 个 defer 缺口、产出 3.7 UI 设计输入。(终审 I-2 修正:原文「核心目标达成/完整走通」过度宣称,2026-09-03)

## 1. 目标达成度(v6.0 §2.2 判据对照)

| 判据 | 结果 | 证据 |
|---|---|---|
| 走到人工门回合数>0 | ✅ 预试验双 provider 各 1 turn(open/completed 成对);矩阵 codex 曾 2 turn | real-run-*-pretrial2/matrix-rep1 |
| 预算逐 turn 恰扣一次 | ✅ remaining_budget 序列+durable budget_reserved=1 三源一致 | 同上+durable turn 文件 |
| advance 后 Ready 且 ledger 零新增 | ✅ amendment 阶段 1(轻语料):advance record ready,provider 零启动 | real-run-codex-amendment-rep1 |
| 全程同 attempt | ✅ 全部有效跑单 session/单 attempt | 各 evidence |
| 判据 A(coding 段首测,方案 A 必达) | ⏸️ **defer**:卡在 codex 连续空交付(族 1),3a 修复点未到达(零回归信号亦零验证) | real-run-codex-final-rep1/2 |
| 判据 B(amendment 链) | ⏸️ 不可达(provider 未启动),记 amendment_not_triggered | 同上 |
| kimi 终局(阶段 2 遗留) | ✅ B 方案收官:codex+pi Confirmed+kimi 合法终态;第四形态(前导语替代交付)登记 | real-run-kimi-final-* |
| 条件项 4(coding_run 未测区) | ⚠️ 结论更新:场景不冗余不退役;真实首测 defer(接缝修后 3a/3b 连锁,编码链主战场=阶段 4) | coding-first v1/v2 |
| 条件项 5(环境清理) | ✅ 进程/临时检出/远程分支全清(38GB+) | 台账 §1 |

## 2. 全部真实跑台账(23 次 run:15 有效 plan 级+4 无效判废+4 coding 链另账;run 数非 episode 数,终审 I-3 修正)

| 阶段 | provider×rep | 结果 | failureClass | elapsed | 归因 |
|---|---|---|---|---|---|
| 预试验 v1(判废,Ruling T3-A) | codex/pi×2 | 全 fail | workspace_error | 80-590s | 驱动 prepare options 污染(已修) |
| **预试验 v2** | codex rep1 | **pass(Confirmed)** | — | 1507.6s | — |
| | pi rep1 | **pass(Confirmed)** | — | 563.1s | — |
| 矩阵 r1(levels) | codex rep1 | fail | ws_closed | 810.8s | **第 5 死路(产品,已修 dc4d0d0d)**;判定①②④首次全过 |
| | codex rep2 | fail | workspace_error | 538.1s | 首稿 27 编译错(provider) |
| | pi rep1 | fail | hard_timeout | 1800s | 修订循环×5 不收敛 |
| | pi rep2 | fail | workspace_error | 627.7s | 中文 section 标题 253 错 |
| kimi 终局 | rep1/rep2 | fail | workspace_error | 1027.9/151.1s | 前导语替代交付/零输出空交付 |
| amendment 阶段 1(defect) | rep1/rep2 | fail/**pass(advance Ready)** | workspace_error/— | —/293.8s | provider/— |
| amendment 阶段 2(coding 另账) | v1 2 次+v2 2 次=4 次 run | 全 fail | 400×2→protocol_error+hard_timeout | 0.06s→30min | **接缝(已修 1e7a7247,3/3 验证)→3a(已修 17c7eccc)→3b(defer)** |
| 矩阵 r2(levels,修复后) | codex×2/pi×1 | fail | workspace_error×2/hard_timeout | 253-313s/1800s | invalid_id/duplicate_id/循环不收敛(provider);死路修复零复发但未到 confirm(零行使) |
| 终局(defect) | codex rep1/rep2 | fail | workspace_error | 54.8/57.2s | **零输出空交付×2(codex 新子形态,CLI 漂移嫌疑证据不足)** |

## 3. 「太重」疑点终局口径(oracle 纠偏)

轻 3/6 vs 重(interactive)0/7 —— 但阶段 2 同 levels 语料+auto_if_valid 曾 codex 2/2、pi 1/1 Confirmed(r26/r28)。**结论:主变量=provider 输出合规性(随策略/prompt/会话状态漂移),语料重量是放大器而非根因。** 阶段 4 provider 策略不得据此错杀 levels 语料;真正要治的是缺陷族 1-4(见 95-baseline-v1.md §3)。

## 4. 修复账(本轮产出,全部 k3 过审+controller 亲验)

| 修复 | commit | 验证 |
|---|---|---|
| 第 5 死路:修订后缺 Evaluate route | dc4d0d0d(+81665984 测试) | 4012/0;真实行使待未来到门跑 |
| 接缝:sc_advance journal 收养 400 | 1e7a7247(+aef9ed06) | 4008/0+真实 3/3 收养 |
| 3a:worktree 未物化回落 | 17c7eccc(+e01a7f77) | 4013/0;终局跑未到达(零回归信号) |
| 诊断:fixture 套+prepare options+amendment driver(liveness×2) | 4e0cc8f6..56bfc3bb | 84/84 |

**specs 补句**:REQ-CG-02 增修订成功后预算重置语义(amendment 门分叉挂 defer)。

**过程事故与教训(登记防再犯)**:①部署假验证×2(health ok≠新进程;验证必须 PID/exe/md5 级)②我读错字段(result_artifact_ref)误导死路归因一轮(scout 纠正)。

## 5. defer/观察账(新增,并入验收产物 defer-ledger 体系)

3b 半启动恢复/判据 A 补测(随 3.7 或阶段 4)/门预算语义统一(kimi 第四形态教学)/95s 断连语义/confirm 吞 findings 可观测性/terminal flaky——详见 95-baseline-v1.md §3 族 8-13。

## 6. 3.7 UI/UX 喂数(路线图排序依据兑现)

- **交互时长**:门 turn 全程 55-300s(codex 慢/pi 快 3 倍);confirm 后编译秒级;建议 UI 呈现 turn 进行态+时长预期
- **输出质量形态**:首稿结构合规是主失败面(重复/非法 ID/缺字段/中文标题);修订 v1→v2 diff 通常只增 2-4 行 capabilities——**diff 视图比全文视图更适合人审**
- **失败呈现**:空交付/前导语污染需要「零输出」专门呈现;死锁类需要 durable 状态直读(reservation/phase)而非 WS 单句错误(observability 缺口佐证)
- **预算显示**:门预算重置语义(REQ-CG-02)对 UI 预算条的含义:显示「本门已用/剩余」需按快照读,不可累计
- **amendment 链**(3.7 吸收 DEF-3):三 WS 线时序图见 evidence/amendment-wire-notes.md,可直接作 UI 状态机蓝本

## 7. 分支合并拍板输入(用户保留)

- 分支领先 main ~700+ 提交:3 个已归档 change+本轮测量产物+3 产品修复(含 TDD 红绿)+specs 补句+证据库
- oracle 形态建议:**merge --no-ff 保历史**(过程证据=审计资产);次优按阶段 3-4 个 squash;不推荐一刀切单 squash
- 合并前待办:终局全分支审查(k3)通过+push

## 8. 声明

全程零伪造、零 dry-run 冒充;所有判定 durable 回读;两处 controller 误判(部署/字段名)均由子代理取证纠正并记档;Rulings 全录台账(`.superpowers/sdd/.../progress.md`)。
