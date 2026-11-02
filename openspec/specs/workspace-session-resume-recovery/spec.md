# workspace-session-resume-recovery Specification

## Purpose
为逻辑代码库规划会话在真实环境漂移或冻结材料不可验证时提供耐久、可解释、用户可达的恢复合同。旧 native 会话不可续接时等待明确 fresh；只更换 provider native 会话，保留原 workspace、产物、反馈、评审和业务 run/turn 身份，并让 driver、observer、刷新与重连得到一致事实。

## Requirements

### Requirement: 持久 superseded 等待事实

系统 SHALL 在 LC 规划旧 native 指纹不匹配、冻结材料不可用或 planning snapshot stale 时保存稳定 recovery 身份、原因、旧身份已知部分、原运行上下文与旧 native tombstone；终结旧 provider 节点并清该运行活跃标记。同一 revision driver SHALL NOT 继续启动 provider；waiting SHALL NOT 被重连/普通生成/中断重试/自动接力隐式消费。

#### Scenario: 漂移零启动并终结节点
- **WHEN** revision 判定旧 full 指纹与当前事实不一致
- **THEN** 系统 SHALL 保存 waiting 与不可恢复旧 native 身份，将旧 provider 节点置 Failed；同一 revision driver 不继续 start_streaming，adapter session-start 与 session child/native 创建均为零

#### Scenario: 等待期间旁路动作拒收
- **WHEN** recovery 为 waiting/reserved/running 且用户或自动接力尝试普通 generation、RequestRevision、中断重试或旧 driver 直接续跑
- **THEN** 后端 SHALL typed 拒收且保留等待与上下文，不因前端未接线或 allowed_actions 为空而存在可启动旁路

#### Scenario: durable 保存失败
- **WHEN** recovery/tombstone/指纹保存或旧 provider 节点收口失败
- **THEN** 系统 SHALL fail-closed、零新的 provider session-start，报告真实存储错误；不伪称等待全部落盘或发布可启动动作，历史产物/反馈/audit SHALL 保留

#### Scenario: SC 等待不是业务失败
- **WHEN** SC author 或已受理人工 turn 在 native spawn 前发生指纹拒绝
- **THEN** 系统 SHALL 保留原 phase、gate、turn_id、attempt_no、budget_reserved 与 reservation，不能借 generic Failed/rearm 新建门或再次扣预算

### Requirement: 原生冻结记录耐久绑定

系统 SHALL 持久化完整 LC projection 基准的 fingerprint 与 provider/role/native/run 身份绑定；spawn 前 prepared 记录、原生握手或同 run 审计后的 active 绑定 SHALL 可跨连接/引擎/进程重建。旧指纹不得由当前事实覆写，reviewer 不得覆盖 author；缺可信 full map SHALL 等待显式 fresh，不静默新会话。

#### Scenario: map 跨连接仍同 native
- **WHEN** 完整冻结记录存在且当前 full 指纹/snapshot 与其一致，即使 Author conversation 引用缺失
- **THEN** 系统 SHALL 仅以同 provider/role/run 的真实 audit 还原 native ID 并合法恢复该 ID，不把 audit 当旧 fingerprint 的替代

#### Scenario: 存量或丢 map 等待
- **WHEN** 旧 native/audit/author 历史存在但完整耐久 map 缺失、basis 不同或材料不完整
- **THEN** 系统 SHALL 进入 resume_fingerprint_unavailable waiting；升级首次继续亦如此，不回填当前指纹、不静默 fresh

#### Scenario: 吊销 audit 不复活
- **WHEN** 旧 native 已在 tombstone 或其旧 run superseded 审计中失去资格，而 conversation/audit 仍在
- **THEN** 系统 SHALL 永久排除该 ID，历史证据不删除、不捞更旧已吊销代；后继修订只候选新 active native

#### Scenario: 候选读取错误不是出生 fresh
- **WHEN** native/audit/map 读取发生 IO/JSON 错误或旧运行存在而 native ID 不可验证
- **THEN** 系统 SHALL typed 拒收或等待，不把错误折叠为空候选后按出生生成启动

### Requirement: 明确 fresh 只换 native 会话

系统 SHALL 仅接受有权 driver 携当前 recovery_id、intent=fresh 与非空 operation_id 的明确动作，重新完整 fresh admission 并使新输入旧 resume ID 为空。该动作 SHALL 保留 workspace/entity、原产物/反馈/可信评审/有效历史/规划事实，按原 run-context 产完整新 artifact 并回原合法门，不增加自动重试或替代动作授权。

#### Scenario: 真 fresh 保留修订上下文
- **WHEN** 有权用户明确选择启动新会话继续当前修订且完整 admission 通过
- **THEN** 系统 SHALL 只创建一个不同的新 native ID，fresh 输入不含旧 resume ID，完整 prompt 包含原 artifact、feedback、trusted review 与必要上下文；原 workspace/entity 不变并产完整新 artifact

#### Scenario: admission 不通过仍等待
- **WHEN** 明确 fresh 后 policy/trust/capability/材料/版本等门不满足
- **THEN** 系统 SHALL 零 provider session-start，保存该 operation 的真实失败/未启动结果并回 waiting，不自动 fresh、不自动登记 trust

#### Scenario: 恢复不换配置或身份
- **WHEN** fresh 消息更换 provider 配置、跨 workspace/scope、recovery 身份过期或原业务 source/hash 已变
- **THEN** 系统 SHALL 拒绝，不使用前端身份/root/provider覆盖服务端事实，不降为普通 author 或新 workspace

### Requirement: Plan 原运行上下文与 SC turn 保留

系统 SHALL 耐久保存 legacy author/outline revision/rebuild/Draft/Batch/PlanRevision、SC author/manual 的明确运行种类、反馈、候选/source引用与业务身份，并按同种类恢复。SC manual SHALL 保持同 turn/command/source hash、gate/预算/reservation；未知 tag/非法 flow SHALL 拒绝，不 fallback plan_author。

#### Scenario: legacy outline feedback 全量 fresh
- **WHEN** 有反馈的 legacy outline revision mismatch 后进入 waiting，用户显式 fresh
- **THEN** 新输入 SHALL 不含旧 ID且自足包含旧 outline 全文、原反馈/评审与当前 story/design/业务材料；完成仍走原 outline 合法门，不把 revision 当无反馈出生 full

#### Scenario: Draft Batch PlanRevision 身份不重建
- **WHEN** 原运行是 Draft、Batch 或 PlanRevision 并被显式 fresh 恢复
- **THEN** 系统 SHALL 保留原 generation/outline/batch/draft IDs、retained/redo/反馈与候选引用，按原完成器返回原门，不重建业务代或重置修复预算

#### Scenario: SC author 从材料到 IR 保持成员
- **WHEN** SC author 或 reviewer 返修轮显式 native fresh
- **THEN** 系统 SHALL 保留原 logical reservation/phase，消费原成员材料并把有效成员 physical ID写入IR，原 compiler/source/selector/review门正常推进，不重新业务reserve/rearm或降legacy

#### Scenario: SC 人工同 turn fresh
- **WHEN** 已受理 SC manual turn mismatch→waiting 后用户明确 fresh
- **THEN** 系统 SHALL 重读同 turn/source hash与feedback，fresh ID为空并按同 turn 完成，原 attempt/budget/gate 不因 native 更换重臂；重连不能先自动增 attempt 或 spawn

#### Scenario: 无合同运行不兜底
- **WHEN** 原 run tag 不认识、非法flow，或同步split没有 native resume 合同
- **THEN** 系统 SHALL 明确拒绝或保持原 Unknown/零恢复启动，不 fallback streaming fresh、普通Author或OutlineRebuild

### Requirement: 最新耐久事实的纯投影

driver、observer、live 广播、attach 与重连 SHALL 使用同一最新 durable recovery事实；引擎锁忙时不得消费 manager 出生旧 record。纯 fallback SHALL 只读、无恢复写入或 spawn；真实恢复入口才幂等收口旧节点。record 读失败 SHALL typed拒绝、不发布旧 snapshot/空恢复/可启动假态。

#### Scenario: 同存活 manager 持锁即时一致
- **WHEN** 同一存活 manager 的引擎锁仍被 provider run 持有，durable recovery更新后立即广播或新连接attach
- **THEN** driver/observer/fallback SHALL 得到同一 recovery_id、reason、allowed actions与非busy投影，不等待锁释放后才恢复或重用出生record

#### Scenario: fallback 零恢复写入
- **WHEN** observer/attach/degraded基线仅请求 durable snapshot
- **THEN** 读取前后 durable session/timeline/turn/journal恢复事实 SHALL 不变，无provider/run登记或spawn；需要reconcile只在真实manager恢复/live mismatch入口执行

#### Scenario: 读取失败不发布假态
- **WHEN** 最新record NotFound/IO/JSON/identity校验失败
- **THEN** 系统 SHALL 返回 workspace_durable_projection_unavailable typed错误，不发送旧record、默认Open/null清恢复、fresh权限或虚构ownership；不能用旧可启动snapshot补关键帧基线

### Requirement: 耐久 operation Replay 与 typed 结果

明确 fresh SHALL 先鉴权/lease/scope，再核 recovery 身份并查询已受理 operation；同 ID SHALL 回放原结果，只有新 ID才核waiting/活run/配置并claim。响应 SHALL typed携recovery_id/operation_id/accepted或replayed/当前耐久operation_state及必要reason；running持锁回放不得等待引擎锁，且任何Replay均零新spawn。

#### Scenario: running 同 ID 回放
- **WHEN** fresh_running且引擎锁被持有，有权同scope用户重传已受理 operation_id
- **THEN** 系统 SHALL 从durable事实立即返回replayed及running/实际operation状态，session-start/native计数不增加；普通stage白名单不得先误拒Replay

#### Scenario: resolved 同 ID 回放
- **WHEN** recovery已resolved且同已受理operation被重传
- **THEN** 系统 SHALL 返回typed回放完成结果，不能重新claim、spawn或复活恢复卡；终态新操作仍拒绝

#### Scenario: prepare 失败回 waiting 同 ID 回放
- **WHEN** 某operation因prepare失败回waiting后同ID再次到达
- **THEN** 系统 SHALL 回放原failed/not_started结果而非再claim；再次显式尝试须新ID，不删除已消费operation记录

#### Scenario: 并发新 ID 唯一胜者
- **WHEN** 多连接/不同operation对同recovery竞争，或同operation重复到达
- **THEN** 系统 SHALL 至多一个durableclaim/一个run/一个spawn；reserved/running的新ID为Busy，observer/stale/crossscope先拒绝，不以Replay绕鉴权

#### Scenario: 未知结果不自动重播
- **WHEN** 接受后进程中断、客户端超时或claim/run/握手结果未知
- **THEN** 系统 SHALL 保存unknown及原操作身份，先核真实audit/run状态，不自动重发新fresh或并发启动第二次

### Requirement: typed 恢复动作与 resolved 收口

前端 SHALL 仅从authoritative recovery字段投影waiting/reserved/running恢复项，提供明确新native会话动作，并保持连接/provider/driver lease/终态守卫。resolved SHALL 消卡并恢复后续合法动作；错误字符串、advance/C1/interrupted retry/批量确认 SHALL NOT 产生fresh授权。

#### Scenario: live 与重连动作可达
- **WHEN** waiting经live、刷新、observer/driver切换或重连到达
- **THEN** 卡 SHALL 保留同recovery身份并显示“启动新会话继续修订”及只换native/保留workspace说明；observer仅展示接管提示，driver一次点击带明确fresh参数

#### Scenario: resolved 后续修订正常
- **WHEN** fresh产物完成并recovery resolved
- **THEN** 卡 SHALL 消失、generic合法动作不再永久隐藏；下一轮修订候选为新native/fullmap，不从历史错误复活旧卡

#### Scenario: 双击与未知回执守卫
- **WHEN** 用户连续点击或发送后未知/断连
- **THEN** UI SHALL 复用同动作operation ID并在inflight禁重复，不乐观伪成功、不自动新ID重发；typed结果与SessionState operation字段共同对账
