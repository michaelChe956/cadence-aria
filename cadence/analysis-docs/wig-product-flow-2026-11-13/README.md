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
6. **design seed（prepare 必填前置）**：`--complete-design <既有seed out根>
   <binary> [端口]` 在**同一已 confirmed story**上调用
   `POST .../design-specs:generate`（`{title, story_spec_ids:[story],
   author/reviewer_provider}`；产品侧先 `validate_confirmed_story_specs`，只
   create_design_spec + create_workspace_session，**不启动 provider**），再用
   **同一份** WS 驱动函数（`drive_entity_to_confirmed(kind="design")`，不复制
   第二份 WS 循环）到 `author_confirm` → HTTP confirm → durable 验证
   `design-specs/<id>.json` confirmed + `versions/<design_id>/version_%04d.json`
   的 markdown digest + design session `provider_start_ledger` 非空。
   完成后只向 `seed-manifest.json` 追加 `design_*` 冻结字段（原子写），并断言
   story 正文 digest 与补 design 前**逐字节一致**（不重建 story）。

plan 阶段沿用 2026-11-09 装置已验证语义：`automation-target` → `prepare`
（`run_policy=auto_if_valid`，`story_spec_ids:[story]` +
`design_spec_ids:[design]`；空 design 会被产品 500 `design_spec_required`）→
等 `automation-enrollment.json` 的 `terminated_at`（fresh seed 克隆无 enrollment
时直接过）→ `enable`（带 `command_id`，`plan_approval=auto`，`source.stories` /
`source.designs` 分别引用 seed 冻结的 `{id, version}`）→ 等
`workspace_session_auto_*` 终态。

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

### issue 正文必须显式给（无占位）

装置**不再内置 description 占位**（旧版写死 `"删除公告"`）。走

```bash
--issue-input /tmp/wig-measurement-setup/issue_input.md
# 或 WIG_ISSUE_INPUT_FILE=<同一文件>
```

文件被**整篇**读出并作为 issue `description` 发送（不是摘要、不是标题）；同时把
`issue_input_path` / `issue_input_sha256` / `issue_input_bytes` 写进
`seed-manifest.json` 与每个 `run-manifest.json`。文件缺失 / 为空 / 非 UTF-8 /
不含「公告删除 + REQ-001..007 + AC-001..006」锚点 ⇒ **非 0 退出，不开臂**。

重建输入 = `/tmp/wig-measurement-setup/issue_input.md`（本目录外的 controller
输入）：它只固定 6 条 AC 的保留语义（基线第 392-397 行逐字）、13 项编号清单
（第 312 行）与「恰好 7 REQ / 6 AC」的覆盖约束。**原 REQ-001..REQ-007 的散文
正文已不可恢复，未伪造**；REQ 的具体切分由 Story 阶段决定。案例内
`docs/issue-003-公告管理升级.md` 与 `story_spec_0015`（13 REQ / 14 AC）是另一个
更大的故事，**不得混入**（锚点校验会拒收）。

### 两个不同的根：state root 与 case root

| 名称 | 谁提供 | 含义 |
|---|---|---|
| **state root**（`--seed` 的第 2 个位置参数） | controller 的 `WIG_STATE_ROOT` | `.aria` 所在目录；整棵克隆成 `<out>/seed-workspace` |
| **fixed case root** | `--case-root` 或 `WIG_CASE_ROOT`（**唯一权威**） | 3 个成员仓所在目录；只用于校验成员 `canonical_path` 与冻结 authority |

装置**绝不从 state root 推断 case root**——两者不同是常态。失败一律显式非 0：

- 两处都没给 ⇒ `缺少固定案例根：显式给 --case-root <…> 或环境变量 WIG_CASE_ROOT…`；
- `--case-root` 与 src 相同 ⇒ 直接拒绝（混用会误判成员身份）；
- 成员 `canonical_path` 不在该根下 ⇒ `成员 busi 的 canonical_path=… 不在固定案例根 … 之下`；
- 快照缺 `aggregate_root` ⇒ fail closed（不猜）；有值但与 `--case-root` 不等 ⇒ 同样拒绝。
- scratch HOME 必须是 `/tmp` 下的**专属子目录**：`/tmp` 本身与真实 HOME 都拒绝。

上述三种失败都会先写 `<out>/seed-manifest.json`（`valid=false` +
`invalid_reason`）并保留已复制的 `seed-workspace` 作为证据；脚本永不删除、永不
覆盖已有 seed 目录。

### seed 与克隆

- seed 只生成**一次**（`--seed`，一次真实 provider story run，binary 记入
  `seed-manifest.json`）；8 臂全部字节级克隆同一 seed 快照。装置**不会**自动
  触发额外 baseline/初始化 run：8 臂只由 controller 显式执行。
- seed 结束时冻结成员快照 `member_snapshot`（+ 派生 `member_snapshot_sha256`），
  并在 seed 收尾再取一次快照断言完全一致；不一致 ⇒ seed 判 `valid=false` 且
  非 0 退出。
- 每臂克隆后、开 daemon 前，就把克隆体的快照与 seed 冻结快照逐字段比对；漂移
  直接判 invalid（不再白跑一小时才发现污染）。
- **design seed 与 story 同权**：`--complete-design` 在既有 seed 的现 confirmed
  story 上只补 ONE design（不新建 issue/story/session），并把 `design_spec_id` /
  `design_version` / `design_version_ref` / `design_markdown_sha256` /
  `design_session_id` / `design_provider_start_ledger` 冻结进同一
  `seed-manifest.json`；已冻结/已存在 design spec 时拒绝第二次。
- **batch preflight（缺 design 即失败，零副作用）**：8 臂模式在读 manifest 后、
  **建任何 `run-*` 目录或发任何 HTTP 调用之前**校验 design 冻结字段非空、
  `design_confirmation_status=confirmed`，并在 seed 工作区只读复核 story/design
  的 confirmed 状态与 markdown digest；缺一项 ⇒ rc=1 且不产生 run 目录。

### 输入身份：只认 durable 事实

1. **story 输入 = confirmed 版本正文**。判等用 `versions/<story_spec_id>/
   version_%04d.json` 的 `markdown` 字段 UTF-8 bytes 的 sha256
   （`story_markdown_sha256`），并记录 `story_version_ref`（`<id>@version_0001`）、
   `story_version_file_sha256`。story spec **记录**文件自带可变时间戳，其
   sha256 只作为 `story_record_sha256` 参考，**不参与判等**。
2. **成员身份 = manifest → members → checkouts 解析链**（与
   `/tmp/wig-measurement-setup/member_snapshot.py` 同构，装置内自带实现）。
   逐成员比 `alias` / `canonical_path` / `checkout_path_hash` /
   `git_dir_identity` / `canonical_git_dir` / `HEAD` / `tree` / `branch` /
   **`git status --porcelain`（dirty）**；聚合根非成员树另有内容 digest——
   只排除固定案例里**真实成员 canonical 目录的精确路径**（不按目录名剪枝，
   深层同名目录如 `nested/frontend` 的内容计入），`.codegraph` 仍按名字跳过。
   **HEAD 没动但 dirty 内容变了 ⇒ 判不稳定**，该臂 invalid。
3. 固定案例 = 精确 3 成员 `busi/frontend/gateway`，且每个成员
   `canonical_path` 必须仍在固定案例根之下（见下节「状态根重指向」）。
4. **design 输入 = confirmed design 版本正文**，判等口径与 story 相同
   （`versions/<design_spec_id>/version_%04d.json` 的 `markdown` UTF-8 bytes
   sha256 → `design_markdown_sha256`，附 `design_version_ref`）；开臂前后各复核
   一次，不一致 ⇒ invalid（design 被重生成/被改）。同时复核 story digest 未变，
   即两个输入 digest 都要与 seed 冻结值一致。

### 状态根重指向（只改该改的）

`repoint_state_root_refs()` 的纪律：

- 只处理 `.aria` 下 **JSON/JSONL 的值**，且 key 必须在白名单
  `state_root / workspace_root / aria_root / product_state_root / root_dir`；
- `members/`、`checkouts/`、`versions/` 整目录跳过；`canonical_path`、
  `source_identity`、`*_hash` 等字段不在白名单 ⇒ 永不改写（固定案例路径、
  成员 authority、正文 digest 一律保持字节不变）；
- 没有任何命中 ⇒ **不写任何文件**（保留原固定外部案例，多臂共享同一物理成员仓
  是预期语义），日志明确打印「保留原固定外部案例」；
- 重指向前后各取一次成员快照，断言完全一致，否则拒绝继续（该臂 invalid）。

### 环境来源（不猜默认档）

`seed-manifest.json` 与每臂 `run-manifest.json` 写 `observed`：`provider`
（来源 = start_generation payload）、`model` / `effort` / `reasoning_effort`
——只有 session 记录或 payload 里**真观测到**才填值并注明来源；没有观测到一律
`{"value": null, "source": "not_observed"}`，**不写 inherit、不猜 xhigh**。
`scratch_home` 记录 controller 提供的隔离 HOME 路径（不含任何 secret）。

### 权限/信任门纪律

- `permission_modes` 取 auto。`permission_request` / `choice_request` 等未知门
  ⇒ **明确报阻塞、该臂失败**，绝不自动批准。
- codex workspace trust 缺项单独处理：装置只读观测
  `$CODEX_HOME/config.toml`（缺省 `$HOME/.codex/config.toml`）中
  `[projects."<root>"]` 的 `trust_level`，**不打印 config 内容**。
  - 默认（未授权）：显式失败，并给出 controller 需追加的完整 TOML 片段与路径；
  - 仅当 controller 置 `WIG_TRUST_AUTHORIZED=1` **且** scratch HOME 是 `/tmp` 下的
    专属子目录时，装置才登记缺失条（0600 追加，绝不覆盖既有 `untrusted` 条目），
    然后**显式重试一次** start_generation；登记来源只有冻结快照里的 4 条
    （1 个聚合根 `provider_context_root` + 3 个成员 `canonical_path`），
    **不解析 error 文本里的路径**；
  - 若 expected roots 现已全部 trusted，则后续 `trust_missing_guidance` 指导帧或
    同一条重复拒绝视为**已消费（consumed）**：不退出、也不再发 start，继续等
    已发 retry 的真实结果；只有**签名不同的新拒绝**才落阻塞；
  - 装置不复制任何凭据/secret、不写真实 HOME；隔离环境一律由 controller 的
    `prepare_env.sh` 提供（`source <setup>/env.sh` 后再跑装置）。
- 真实可用性只由 durable `provider_start_ledger` 非空证明（seed 与每臂 plan
  会话都检查，为空即 invalid）；不因 `~/.codex/auth.json` 存在而推定可用。


## 臂序与绑定

- 8 臂 = 4 before + 4 after，**ABBA 交替**（每对内 after,before,before,after），
  严格串行，唯一变量 = 二进制。两件二进制内嵌 UI 资源同为 `index-CD_NhB8M.js`；
  即便如此，性能结论**仍只按后端 binary 差异解释**（前端不参与驱动）。
- 每 run 写 `run-manifest.json`：tag/arm/port、binary 路径 + sha256、
  seed-manifest sha256、issue/story/version + `story_version_ref`、design 冻结
  身份（`design_spec_id` / `design_version` / `design_version_ref`）、
  `issue_input_sha256` / `story_markdown_sha256` / `design_markdown_sha256`
  （输入来源 digest）、`scratch_home`、`observed`（provider/model/effort 及其
  来源）、plan_session_id、终态、provider_start_ledger、输入身份复核
  （`checks.member_snapshot_*` / `checks.story_markdown_identical` /
  `checks.design_markdown_identical`）、valid/invalid_reason。
- 产物纪律：已存在的 run/seed 目录**拒绝覆盖**（失败证据不删除）；任一臂
  超时/异常/未终态/正文 digest 不符/成员快照漂移 ⇒ 该臂 invalid 且进程退出码
  非 0；收割器显式列排除清单。error/timeout/inconclusive 一律不是有效样本。
- 无效判定不看「HEAD 是否变了」：`HEAD` 相同但 `git status` 变化（dirty 内容被
  改）同样判 invalid。

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
6. seed 与 issue 输入都是**重建**：seed 不是原 `story_spec_0009` 的逐字重现，
   issue 输入不含原 REQ 散文正文（仅编号 + 6 条 AC 保留语义）。任何结论都不得
   表述为「与原验收 seed/issue 逐字相同」，也不得把 `issue-003` /
   `story_spec_0015` 的更大故事当成本输入。

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
SETUP=/tmp/wig-measurement-setup

# 0) 隔离环境（controller 一次性准备；scratch HOME 在 /tmp，凭据只由它物化）
source $SETUP/env.sh     # 导出 HOME=<scratch>、PATH 垫片（只放行 codex）、WIG_CASE_ROOT
# 缺 codex trust 且已获授权时，装置才会登记缺失条并显式重试 start：
# export WIG_TRUST_AUTHORIZED=1

# 1) 真实 smoke + 一次性固定 seed（controller 验收入口）
#    第 2 个位置参数 = state root（.aria 所在、会被克隆）；
#    case root 只认 --case-root 或 WIG_CASE_ROOT（不从位置参数推断）
#    issue 正文必须显式给（无内置占位）：--issue-input 或 WIG_ISSUE_INPUT_FILE
python3 $D/run_arms.py --seed /tmp/aria-wig-pinned/after-07a0df79 \
  "$WIG_STATE_ROOT" /tmp/aria-wig/out-seed 4709 \
  --case-root "$WIG_CASE_ROOT" \
  --issue-input $SETUP/issue_input.md

# 1b) 在既有 seed 的现 confirmed story 上只补 ONE design（不重建 story）
#     同一命令完成：design prepare → WS 真实启动 → author_confirm → HTTP confirm
#     → durable 验证 → design_* 冻结进同一 seed-manifest.json（原子写）。
#     已冻结 design / 已存在 design spec ⇒ 拒绝第二次。
#     多仓 Design 确认门要求 involved 非空（REQ-PLN-04），involved>1 还需
#     change_order（REQ-PLN-05）；若 provider 没有自决，controller 显式给
#     --design-involved a,b / --design-change-order a,b（逗号或 JSON 字符串数组
#     均可；保序、不静默去重，id 必须在当前 selection 上界内，不猜业务范围）。
python3 $D/run_arms.py --complete-design /tmp/aria-wig/out-seed \
  /tmp/aria-wig-pinned/after-07a0df79 4710 \
  --design-involved <repo_id[,repo_id...]|'["repo_id",...]'> \
  --design-change-order <repo_id[,repo_id...]|'["repo_id",...]'>

# 2) 8 臂 ABBA（唯一变量 = 二进制；任一臂失败 ⇒ 退出码非 0）
#    装置只按参数跑这 8 臂，不会自行追加 baseline/初始化 run。
#    必须用**新的 batch out 根**：旧 out 的 run-* 行是失败证据，不可覆盖/复用；
#    batch preflight 缺 design 时在建任何 run 目录之前 rc=1。
python3 $D/run_arms.py \
  /tmp/aria-wig-pinned/before-a953327d \
  /tmp/aria-wig-pinned/after-07a0df79 \
  /tmp/aria-wig/out-seed/seed-workspace /tmp/aria-wig/out-batch 2 4701

# 3) 收割 + 精确置换（只出方向性对照；排除清单与两个输入 digest 一并打印）
python3 $D/harvest_by_role.py /tmp/aria-wig/out-batch /tmp/aria-wig/out-batch/sequence.log
```

全部测试产物只写 `/tmp`（装置不写仓库、不写 HOME、不动原案例目录）。
`--issue-input` 可放在任意位置；臂阶段也接受它，但仅用于**核对**是否与 seed
记录的 digest 一致（不一致即拒绝开臂）。

### 已有 out 根（失败证据）不要删、不要复用

`--seed` 拒绝覆盖已存在的 `<out>/seed-workspace`；失败的 seed 目录里会留
`seed-manifest.json`（`valid=false` + `invalid_reason`）与已复制的
`seed-workspace`，**原样保留作为证据**。

- **首选 `--resume`（同一 prepared session，不新建 issue/spec/session）**：
  只读既有 `<out>/seed-manifest.json` 与其记录的 issue/story/session，核对
  案例根 / issue 输入 digest / 成员快照未变后附着同一 session；若 story 已
  confirmed 且有 provider 账目 ⇒ **零模型调用**，直接补同一 manifest；
  否则仅在 `provider_start_ledger` 为空（证明从未启动）时才发一次 start，成功
  后仍写同一 manifest。
  ```bash
  python3 $D/run_arms.py --resume /tmp/aria-wig/out-seed \
    /tmp/aria-wig-pinned/after-07a0df79 4709 \
    --case-root "$WIG_CASE_ROOT" --issue-input $SETUP/issue_input.md
  ```
- 若既有 session 已不可用，则换新 out 根完整重跑 seed（脚本无半程重跑语义）：

```bash
# 例：/tmp/aria-wig 里已有失败的 seed-workspace ⇒ 用 out-seed2 重跑
python3 $D/run_arms.py --seed /tmp/aria-wig-pinned/after-07a0df79 \
  "$WIG_STATE_ROOT" /tmp/aria-wig/out-seed2 4709 \
  --case-root "$WIG_CASE_ROOT" --issue-input $SETUP/issue_input.md

# 新 seed 同样必须先补 ONE design（否则 batch preflight rc=1，不建 run 目录）
python3 $D/run_arms.py --complete-design /tmp/aria-wig/out-seed2 \
  /tmp/aria-wig-pinned/after-07a0df79 4710

# 8 臂指向新 seed 快照 + **另一个新的 batch out 根**
python3 $D/run_arms.py /tmp/aria-wig-pinned/before-a953327d \
  /tmp/aria-wig-pinned/after-07a0df79 \
  /tmp/aria-wig/out-seed2/seed-workspace /tmp/aria-wig/out-batch2 2 4701
```

## 本次修复的验证状态（如实）

- 已在本机（无真实 provider、无 daemon 真启动）做定向冒烟：
  ① issue 输入缺失/空/错案例、HOME 非 `/tmp` ⇒ 非 0 且不建产物目录；
  ② 合成 3 成员 `.aria` 夹具上 `member_snapshot` 解析出精确 3 成员
     alias/head/tree/dirty 与固定案例路径；`repoint_state_root_refs` 只改
     `state_root`，`provider_context_root` / `*_hash` 保持原值，二次调用 0 文件；
  ③ 成员 `HEAD` 不变、仅新增未跟踪文件 ⇒ `snapshot_diff` 命中
     `<member>.status`（判不稳定）；
  ④ trust 门：缺项未授权只报片段；授权后登记 2 条并 0600、幂等；既有
     `untrusted` 条目拒绝覆盖；授权但 HOME 非 `/tmp` 拒绝写；
  ⑤ `entity_version_digest`（story/design 同形）在 version 记录时间戳变化时
     markdown 字节 digest 不变；缺版本文件非 0（story 夹具沿用上一版观测；
     design 走同一实现，未单独跑夹具）；
  ⑥ harvester 在合成 manifests 上打印 issue/story/**design**/seed digest、
     排除 timeout 臂、token 缺值呈 `n/a`；
  ⑦ 真机 8 臂首跑失败根因 = prepare 缺 design（产品返回 500
     `design_spec_required`），已由「`--complete-design` + batch design
     preflight」闭合；该结论来自既有 run-manifest/sequence.log 证据，**新路径
     本身未经真机执行**。
- **未验证（notrun，必须由 controller 执行时观察）**：真实 provider 的 seed
  生成、`--complete-design` 的真实 design 生成/确认、缺 design 的 batch
  preflight 实际拒绝行为、开臂前后 design digest 复核、choice 门显式审批路径、
  8 臂真机执行、trust 登记在真实 codex 会话中的实际生效、任何耗时/token 数值
  结论。本目录脚本不会自行启动 provider 或追加 run。

### 真机 seed 首次失败的根因（已修）

controller 的 `bg_555` 首跑失败：`成员 busi 的 canonical_path=<case>/busi 不在
固定案例根 <state-seed> 之下`。根因 = 旧版把 `--seed` 的**位置参数（state root）**
当成 fixed case root。现改为 case root **只**取自 `--case-root` / `WIG_CASE_ROOT`，
缺失即显式 error，绝不由 src 推断（见「两个不同的根」）。已补验证：
⑦ case root 取自 `WIG_CASE_ROOT` 时能走过旧失败点并按 state root 克隆；
`--case-root` 与 src 相同、给无关目录、以及两处都缺，三种情形分别得到明确
error 且前两种先落 `valid=false` 的 seed-manifest 并保留 `seed-workspace` 证据。


## 窄冒烟（controller 用，不启动真实 provider）

```bash
D=/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-wig-optimize-11-08/cadence/analysis-docs/wig-product-flow-2026-11-13
SETUP=/tmp/wig-measurement-setup
# scratch HOME 必须是 /tmp 下的**专属子目录**（/tmp 本身与真实 HOME 都被拒绝）
H=/tmp/wig-probe-home
mkdir -p "$H"

# 0) 语法（不是验收，只挡低级错误）
python3 -m py_compile $D/run_arms.py $D/harvest_by_role.py && echo COMPILE_OK

# 1) 缺 issue 输入 ⇒ rc=1，且不创建产物目录
HOME="$H" python3 $D/run_arms.py --seed /bin/true /tmp/no-case /tmp/wig-probe-out; echo "rc=$?"

# 2) 空输入 / 错案例输入 ⇒ rc=1
: > /tmp/wig-probe-empty.md
HOME="$H" python3 $D/run_arms.py --seed /bin/true /tmp/no-case /tmp/wig-probe-out \
  --issue-input /tmp/wig-probe-empty.md; echo "rc=$?"

# 2b) 缺 case root（既无 --case-root 也无 WIG_CASE_ROOT）⇒ rc=1「缺少固定案例根」
env -u WIG_CASE_ROOT HOME="$H" python3 $D/run_arms.py --seed /bin/true /tmp/no-case \
  /tmp/wig-probe-out --issue-input $SETUP/issue_input.md; echo "rc=$?"

# 3) 臂模式：伪造 seed-manifest（valid=false）⇒ rc=1「拒绝开臂」，不开 daemon
mkdir -p /tmp/wig-probe
printf '{"schema":"wig-seed-manifest/1","valid":false,"invalid_reason":"probe"}\n' \
  > /tmp/wig-probe/seed-manifest.json
HOME="$H" python3 $D/run_arms.py /bin/true /bin/true /tmp/wig-probe/seed-workspace /tmp/wig-probe 1
echo "rc=$?"

# 4) 臂模式：seed valid 但**缺 design 冻结字段** ⇒ rc=1「缺 confirmed design
#    seed」，且不创建任何 run-* 目录（preflight 先于建目录/发调用）
mkdir -p /tmp/wig-probe2
printf '{"schema":"wig-seed-manifest/1","valid":true,"issue_id":"issue_0001",'\
'"story_spec_id":"story_spec_0001","story_version":1,'\
'"story_version_ref":"story_spec_0001@version_0001",'\
'"story_markdown_sha256":"%s","provider_start_ledger":1,'\
'"member_snapshot":{"members":{}},"fixed_case_root":"/tmp"}\n' \
  "$(printf 'a%.0s' {1..64})" > /tmp/wig-probe2/seed-manifest.json
env -u WIG_CASE_ROOT -u WIG_ISSUE_INPUT_FILE HOME="$H" \
  python3 $D/run_arms.py /bin/true /bin/true /tmp/wig-probe2/seed-workspace \
  /tmp/wig-probe2-out 1; echo "rc=$?"
ls -d /tmp/wig-probe2-out/run-* 2>/dev/null || echo "no run dirs（符合预期）"
```

以上门禁都不接触 provider、不写 HOME、只在 `/tmp` 下留探针文件。
