# Tasks: retire-legacy-workitem-protocol

**工作包性质**：本文件只登记**高层工作包**与验收口径（映射 specs 的 requirement 与 design.md 的 D 系列决策）。精确文件、命令、测试与提交步骤由 `superpowers:writing-plans` 展开到 `cadence/plans/`；Plan 只能展开工作包，不能重定义契约。

**主契约**：三方决议 `stage4-tripartite-decision.md`（Q1/Q4/Q5(③)，决议承接对照表 = design.md 附录 B）+ 本 change 四件套。

**全局边界（每工作包均继承）**：

- **范围铁律**：仅决议四项+oracle 风险承接（preflight 修订）（门重测/legacy 删除含前端迁移/DEF-4 契约显式化/退役后 preflight 条款修订），不加不减；多仓 coding、provider 批事项属 change ①②，不越界。
- **判据零改红线（1c 钉死）**：REQ-WSC-07 判据不放宽、不改口径、不删子项；重测只出证据。重测仍超标→挂起等用户终裁（问法三选备呈报，不预执行），WP2/3/5 冻结（WP4 DEF-4 可独立先行）。
- **Q3 口径延续**：kimi/claude 结论不进入 REQ-WSC-07 判据（判据原文仅 codex+pi），全部产物与文案同口径。
- **SC 基座零变化**：REQ-WSC-01..06、REQ-CG-01..07 语义（除关门决策 typed 重承载）、compile/policy/amendment 链、StartCoding 语义不动。
- **存量红线**：历史 durable 记录只读保留、事件前缀不可变、无 legacy 复活开关。
- **RR-3 定性纪律**：重测/迁移中 flaky 定性须首败事实+定向复跑佐证+diff 无交集。

**排序总则**：WP1 门重测（解锁闸门，超标挂起）→ WP2 L0 typed 重承载 ∥ WP4 DEF-4（独立面）→ WP3 L1 前端迁移 → WP5 L2 后端删除跨端原子 → WP6 关闸。

## 1. WP1 退役门全口径重测（解锁闸门）

- [x] 1.1 超时预算前置+campaign 准备：设定单案例超时上限与总预算（pi 先例曾需 1800s 上限）并留档；复用 change ① 验证过的 campaign 基建（`workitem-coding-campaign` 驱动器族，显式非 dry-run）；重测基线 build 版本如实记录（≥v24，以执行时现行 build 为准）（REQ-RET-01）——验证：预算与 build 版本设定记录留档先于任何重测跑
- [x] 1.2 全口径证据矩阵出具：REQ-WSC-07 判据原文全部子项在现行 build 重测——codex 与 pi 各 1 案例 Confirmed（2/2）、时长 ≤12min、初评/复评总 ≤2、自动返修 ≤1（服务端持久计数）、14 条 classifier golden、grammar/lowering 才走 compiler diagnostic golden、断线重连/恢复、legacy 回归全绿、多仓 preflight 不静默回落；逐子项证据锚留档（REQ-RET-01）——验证：矩阵逐子项有证据锚且判据与 spec 原文逐字一致（零改自查）；真实 provider 参与判据路径、无替身
- [x] 1.3 退役解锁判定：矩阵全绿→WP2/3/5 解锁留档；任一关键子项（尤其 pi 时长）仍超标→本 change 挂起，问法三选（A 维持门不删 legacy/B 登记例外+修订门文本/C 限 N 次取最佳）如实呈报用户终裁，终裁前零删除零门文本修订（REQ-RET-01）——验证：解锁/挂起判定记录在案；挂起分支下 WP2/3/5 无任何已执行变更
- [x] 1.4 coding_run_campaign 未测区顺带核：逐区核对未测区，冗余区标记退役并登记（不静默删除），仍引用区登记引用面（REQ-RET-04）——验证：核对表覆盖全部未测区，处置逐区留档
- [x] 1.5 WP1 关闸——验证：1.1-1.4 证据齐备；矩阵覆盖 REQ-WSC-07 全子项无遗漏；判据零改终检

## 2. WP2 SC 门关门决策 typed 重承载（L0，双轨期）

- [x] 2.1 abandon typed 入站命令：后端新增显式 typed abandon 命令承载 SC 门关门终止决定（与 legacy `human_confirm` 通道零共用枚举），approve 保持 `Confirm` 语义；`conversational_gate.rs` 关门签名收敛为门专属 typed 决策（REQ-RET-02、design D2-L0）——验证：SC 门 approve/abandon/反馈行为与重承载前等价（幂等/预算/恢复语义测试全绿）；新命令失败测试先行
- [x] 2.2 双轨期全量回归：legacy 消息此时仍接受（双轨保留），全量门禁（lib/it_web/it_core+前端）双绿（REQ-RET-02）——验证：门禁结果留档；SC 门既有测试族零改语义通过
- [x] 2.3 WP2 关闸——验证：2.1-2.2 证据齐备；REQ-CG-02/04 修订句（delta）与新行为一致

## 3. WP3 前端 legacy 消费面迁移（L1，oracle scope 修正大头）

- [x] 3.1 发送面与动作面切 typed：useWorkspaceWs（request_change/terminate 经 `human_confirm` 的发送）、bulk-confirm-runner（批量 confirm）、useStageUI（`human_confirm` 三动作与 `review_decision` 动作）、cockpit/plan-repair `legacy:human_confirm` 路由全部切 typed（abandon 用 WP2 新命令）（REQ-RET-02）——验证：前端产码不再发送任何 legacy 决策消息（grep 断言）；动作面行为等价（按钮/快捷键冒烟）
- [x] 3.2 union 与类型收敛+legacy 页面处置：workspace.ts union 删 `human_confirm` 分支+增 abandon 命令类型；`ChatWorkspacePageLegacy` 按消费路由实测定处置（legacy 路径删除后决策面不可达）；前端测试面同步收敛（legacy 断言随行为删）（REQ-RET-02）——验证：前端全量测试绿；类型层无 legacy 决策残留（暂留后端 wire 类型除外——L2 同批归零）
- [x] 3.3 WP3 关闸——验证：3.1-3.2 证据齐备；e2e 覆盖 typed 动作面（approve/abandon/feedback 全路径）

## 4. WP4 DEF-4 契约显式化（独立面，可与 WP1 并行）

- [x] 4.1 REQ-ADV-05 契约落地核验：现行代码与测试锚定「advance 到 Ready 即止+StartCoding 唯一入口+`SC_CODING_REQUIRES_ADVANCE` 守卫」三句（`advance_ready_only.rs:72` 测试、`socket.rs:284,302,321` 守卫为既成事实基线）；spec 与代码行为一致（REQ-ADV-05、design D4）——验证：逐句锚定表（spec 句→测试/代码证据）留档；契约缺失处补测试（不扩权）
- [x] 4.2 auto 通道红线条件 defer 登记：五条红线（opt-in 持久化 run_policy/默认 off/per-attempt 单发/绝不批量/不动唯一人工门）登记入 defer 台账指向 REQ-ADV-05，触发条件=autopilot/驾驶舱真实 auto 需求（REQ-ADV-05、design D4）——验证：台账登记在案且指向 spec 条款；本 change 无任何 auto 启动路径落地
- [x] 4.3 DEF-4 销账：defer-ledger DEF-4（defer-ledger.md:23）按 REQ-ADV-05 契约显式化销账登记（REQ-ADV-05）——验证：销账记录留档；DEF-6 销账随 WP6 关闸（退役完成后）
- [x] 4.4 WP4 关闸——验证：4.1-4.3 证据齐备；WP4 不依赖 WP1 结论（Q4 独立面，挂起分支下可先行）

## 5. WP5 后端 legacy 删除（L2，跨端原子收口）

- [x] 5.1 wire 层删除：`in_.rs` 必删集变体族+DTO 删除（generation-mode 决策/逐段确认消息/review_decision 双选项/`HumanConfirm`+`HumanConfirmDecision`/`SelectRevisionPath` 族及专属 DTO）；共享变体按「SC 是否消费」逐个判定归属并留档（REQ-RET-02、design D2）——验证：归属判定表留档；已删消息类型收到返回 protocol error 零副作用（负向测试）
- [x] 5.2 引擎层删除：`workspace_ws_handler/decisions.rs` 与 `workspace_engine/decisions.rs` legacy 决策路由分支、`draft_batch`/`plan_outline` 逐段确认引擎面删除；`flow_kind` 单路径收敛（新会话一律 SingleCandidate，存量 durable 字段只读兼容）（REQ-RET-02/03）——验证：全量门禁双绿；存量记录读取路径有兼容测试
- [x] 5.3 多仓 preflight 条款修订落地：会话创建路由 legacy fallback 分支删除，preflight 失败一律收敛新路径 durable fatal/recoverable 终态（REQ-WSC-08、design D3）——验证：负向测试（preflight 失败→新路径终态+原因，无 legacy 回落）；REQ-WSC-08 scenario 逐条对应
- [x] 5.4 跨端归零+legacy 回归退役：前端残余 legacy 类型同批归零；legacy 回归测试族在 WP1 证据矩阵留档后删除（不以 ignore/永红压制）；残留归零断言（必删集符号全仓产码+测试检索为零，历史归档文档与 durable 数据除外）（REQ-RET-02、design D5）——验证：grep 断言脚本结果留档；全量门禁（后端+前端）双绿
- [x] 5.5 WP5 关闸——验证：5.1-5.4 证据齐备；中间双轨态不存在（原子收口自查）

## 6. WP6 关闸与销账

- [x] 6.1 validate strict+主 specs 预检：`openspec validate retire-legacy-workitem-protocol --strict` 过；REMOVED REQ-WSC-07 的被引点全数接正（conversational-gate/advance/provider-validation-round（C1 归档 sync 后的 REQ-PVR-03 历史性提及——语义自足非悬挂，显式点名防漏读）/其他 spec 无悬挂引用）（REQ-WSC-08、design R8）——验证：validate 输出留档；主 specs 引用扫描无悬挂
- [x] 6.2 全量门禁双绿+证据归档：lib/it_web/it_core+前端+fmt/clippy 全绿；WP1 证据矩阵/归属判定表/残留断言/核账记录归档 `cadence/`（REQ-RET-01/02）——验证：门禁结果与归档清单留档
- [x] 6.3 DEF-6 销账：defer-ledger DEF-6（defer-ledger.md:25）按退役完成销账登记（REQ-RET-02）——验证：销账记录留档
- [x] 6.4 关闸终检——验证：决议四项逐项对照（附录 B）；范围铁律自查（无越界项）；挂起分支未触发或已按用户终裁分叉留档
