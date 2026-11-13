# 真机装置：WIG 事实投影修复 before/after 对照（2026-11-13，修复版）

本目录是 `repair-wig-fact-projections`（归档于
`openspec/changes/archive/2026-11-13-repair-wig-fact-projections/`）的**真机对照装置**。
本版修复了初版装置的两个致命缺陷（协议猜错 + 输入不固定），并纠正了初版 README
的三处错误归因（见「对初版 README 的更正」）。

## 装置闭环的真实协议（从 src 逐点核对 + controller 真浏览器取证）

初版装置的 30 分钟空等根因：**`story-specs:generate` 只是 prepare**。它只落
draft story spec（`confirmation_status=draft`）+ open 状态的 story session，
不启动任何 provider（`src/web/handlers/lifecycle.rs generate_story_specs` 仅
create_story_spec + create_workspace_session）。真机冒烟证据
`/tmp/aria-wig-smoke/run-smoke-4701`：返回 200 后 30 分钟仍 status=open、
`current_version=null`、`provider_start_ledger=[]`、无 timeline。controller
真浏览器验收同构：打开 `/workbench/workspace/workspace_session_0001` 界面停在
「开始生成」按钮——缺的从来不是等待，是显式 start。

完整链路（`run_arms.py --seed` 自动执行）：

1. `POST /api/projects/{p}/issues`（带 `logical_codebase_id`）。
2. `POST .../story-specs:generate` → draft spec + open story session（prepare）。
3. **WS 真实启动**：`GET /api/workspace-sessions/{id}/ws`（`src/web/app.rs:670`）
   → `{"type":"hello","session_id":...,"last_seen_node_id":null}`
   → 首帧 `session_state.stage=prepare_context` 时发
   `{"type":"start_generation","provider_config":{"author":"codex",
   "reviewer":"codex","review_rounds":1,"permission_modes":{"author":"auto",
   "reviewer":"auto"}},"reviewer_enabled":false}`
   （stage 守卫：`src/web/workspace_ws_handler/protocol.rs`
   `is_message_valid_for_stage_with_flow` 在 PrepareContext 放行
   StartGeneration；引擎 `WorkspaceEngine::start_generation` 经 provider_drive
   驱动真实 provider）。
4. stage 到 `author_confirm`（story 确认门）后，**approve 走 HTTP**：
   `POST /api/workspace-sessions/{id}/confirm`
   `{"confirmed_by":"wig-harness","with_review":false}`。
   story 门上 WS `confirm` 帧不放行（protocol.rs AuthorConfirm 注释）；在途
   Running 会被 F-31 守卫 409 拒收，装置带 6×5s 小步重试只覆盖 durable 残留窗口。
5. durable 验证：story spec `confirmation_status=confirmed`、`current_version`
   非空、session `provider_start_ledger` 非空（**真实 provider 启动证据**）。

plan 阶段沿用 2026-11-09 装置已验证语义：`automation-target` → `prepare`
（`run_policy=auto_if_valid`）→ 等 `automation-enrollment.json` 的
`terminated_at`（fresh seed 克隆无 enrollment 时直接过）→ `enable`（带
`command_id`，`plan_approval=auto`，stories 引用 seed 的 `{id, version}`）→
等 `workspace_session_auto_*` 终态。

## 原案例证据现状（不可恢复项，如实列出）

原接受案例 = `/home/michaelche/workspace/test-folder/test-center-java`
（busi/frontend/gateway 三成员仓）上的「公告删除」issue；历史产物原位于
`/tmp/aria-verify/.aria/projects/project_0001/issues/issue_0001`（含
story_spec_0009 等已确认 seed）。**该目录现已不存在**（2026-11-13 核查为空），
且全盘搜索无 `story_spec_0009*` 副本。具体缺项：

| 缺项 | 说明 |
|---|---|
| 原已确认 story/design seed 内容与版本 | `/tmp/aria-verify` 整体丢失，无法按字节恢复 |
| 原 3 成员工作区 `.aria` 状态（LC 注册、成员 heads/config） | 从未入 git；`/tmp/aria-wig`（11-09/11-11 装置与模板）也已不存在 |
| 原验收 run 的原始产物 | 11-09 目录仅留存 sequence/harvest 汇总指标（已在 git），非 seed 本体 |

幸存：`test-center-java` 三成员仓内容仍在（成员 git 仓完好）；11-09/11-11 的
汇总指标仍在 `../wig-product-flow-2026-11-09/`。

因此本装置的 seed 是**在 controller 准备的 3 成员案例工作区上一次性重建的固定
输入**，不是原验收输入；结论表述必须携带该限定。**1 成员 maven 夹具
（`/home/michaelche/aria-kb-test-workspace`）不得作为性能对照案例**——成员数与
案例仓都不同，会引入第二变量（初版装置正是错用了它，且每臂各生成一个新 story，
双重污染，全部数据不可归因，已作废）。

## 固定输入（消灭第二变量）

- seed 只生成**一次**（`--seed`，一次真实 provider story run，binary 记入
  seed-manifest.json）；8 臂全部字节级克隆同一 seed 快照（confirmed story +
  成员 heads 完全一致）。
- 每臂收尾复核输入身份：story spec 文件 sha256 与 seed 一致、成员仓 HEAD 未漂移；
  任一不符 ⇒ 该臂 invalid（`member_heads`/`story_input_sha256` 记入 manifest）。
- provider/model/effort：author=reviewer=codex、effort 不传（产品默认档）、
  权限 auto。**不因 `~/.codex/auth.json` 存在而推定可用**——真实可用性只由
  durable `provider_start_ledger` 非空证明（seed 与每臂 plan 会话都检查，
  为空即 invalid）。
- 若服务端发来 permission_request / choice_request / trust 指引等未知门：
  装置**明确报阻塞**（该臂失败），不自动批准、不发 `register_trust_entry`
  （不写任何 HOME 级用户配置）。

## 臂序与绑定

- 8 臂 = 4 before + 4 after，**ABBA 交替**（每对内 after,before,before,after），
  严格串行，唯一变量 = 二进制。两件二进制内嵌 UI 资源同为 `index-CD_NhB8M.js`；
  即便如此，性能结论**仍只按后端 binary 差异解释**（前端不参与驱动）。
- 每 run 写 `run-manifest.json`：tag/arm/port、binary 路径 + sha256、
  seed-manifest sha256、issue/story/version、plan_session_id、终态、
  provider_start_ledger、输入身份复核、valid/invalid_reason。
- 产物纪律：已存在的 run/seed 目录**拒绝覆盖**（失败证据不删除）；任一臂
  超时/异常 ⇒ 进程退出码非 0；超时/异常/未终态/污染的臂不算有效样本，
  收割器显式列排除清单。

## 对初版 README 的更正（错误归因）

1. **Origin 403 的根因**：不是「urllib 丢请求头」，而是 origin 拼接双冒号——
   `scheme` 自带尾冒号（`"http:"`）再拼 `"//host"` 得 `"http:://host"` 畸形值，
   被 `forbidden_origin` 拒收。修复 = 拼接前剥尾冒号；mutation 仍必须带受信
   精确回环 Origin（`src/web/api_auth.rs`）。
2. **置换检验分辨率**：n=4/臂时**精确**枚举全部 C(8,4)=**70** 个分组，p 的
   分辨率下限是 **1/70**；初版的「20000 次重排、分辨率 1/20001」是对随机
   近似的误解（重排次数不创造统计分辨率）。本版改为精确枚举，且只作
   **方向性记录**，不作显著性判定。
3. **总耗时口径**：= 各 timeline 节点时长**之和**（sum(nodes)），不是墙钟
   起止差（节点可重叠/含等待，两者不可互换）。first-token time 无 durable
   时间戳来源（execution_events 不带时刻、streaming_content 无 token 时刻），
   本装置**不测**，不以估算冒充。

## 已知限制（必须随结论一起说）

1. 同 provider 串行是前提；并发跑 ⇒ 归因不可分（11-09 报告 §5.9 已判）。
2. 单案例单 issue：结论只对「该 3 成员案例 × 该 seed × codex」成立；
   seed 是重建输入，非原验收 seed（见上）。
3. n=4/臂 ⇒ 任何指标只看方向，不能声称统计显著；不使用多重检验校正话术。
4. 界面前端呈现层不在测量面（本轮改动是平台侧投影与 prompt 注入）。
5. 二进制固定：改代码必须重建并重新固定 sha，否则前后臂不可比。

## 复现（controller 入口）

前置：① 3 成员案例工作区（controller 准备：在 test-center-java 聚合根上注册
busi/frontend/gateway 三成员 LC——原注册态不可恢复，属重建）；② 固定二进制：
before=`/tmp/aria-wig-pinned/before-a953327d`
（sha256 `66af31fa567e947cd311ffb72a649bbb21277444e30bc58dbe8e6ceac202c60f`）、
after=`/tmp/aria-wig-pinned/after-07a0df79`
（sha256 `324274abb9bf900378ba82e5b68f731367c020b9691cdc8f24ee6e938573065a`）；
③ 端口避开共享 4317（装置默认 4701+，seed 4709）。

```bash
D=/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-wig-optimize-11-08/cadence/analysis-docs/wig-product-flow-2026-11-13

# 1) 真实 smoke + 一次性固定 seed（= controller 验收入口：
#    证据链 = server 接受 start_generation（story-ws-frames.jsonl 有
#    start_generation_sent 帧）→ provider_start_ledger 非空 → story confirmed）
python3 $D/run_arms.py --seed /tmp/aria-wig-pinned/after-07a0df79 \
  <3成员案例工作区> /tmp/aria-wig 4709

# 2) 8 臂 ABBA（唯一变量 = 二进制；任一臂失败 ⇒ 退出码非 0）
python3 $D/run_arms.py \
  /tmp/aria-wig-pinned/before-a953327d \
  /tmp/aria-wig-pinned/after-07a0df79 \
  /tmp/aria-wig/seed-workspace /tmp/aria-wig 2 4701

# 3) 收割 + 精确置换（只出方向性对照；排除清单一并打印）
python3 $D/harvest_by_role.py /tmp/aria-wig /tmp/aria-wig/sequence.log
```

全部测试产物只写 `/tmp`（装置不写仓库、不写 HOME、不动原案例目录）。
