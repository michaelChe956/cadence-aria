# 问题记录：审查 finding 越出 Work Item 范围导致人工回复无效、评审不收敛

日期：2026-10-30
状态：**待 owner 裁定方案取舍后实施**（owner 2026-10-30 指示「这块你需要优化」）
触发来源：owner 真机跑全流程，`workspace_session_auto_e5f6b290-f231-479a-9c5c-c7d63c93515e`

## 一、owner 的三条反馈（原文要点）

1. 「我的整个流程不应该存在这个情况……**我的文档都是 aria 产品的文档，不存在让 coder 实施的时候去帮我回写的概念**，这块你需要优化」
2. 「刚刚这样卡住了，**用户不知道如何往下继续了**」
3. 「**用户其实也操作过两次了，但是每次 author 修完之后，reviewer 还是会审核出来这个问题**」

## 二、现场事实

### 2.1 reviewer 三轮的 findings（逐字）

第 1 轮（`timeline_node_002`）5 条 suggestion，其中第 3 条：

> 补齐 Element Plus 中文 locale 后，生效规范明确要求回写自身第 4 节与第 10 节缺口表，但本计划四个工作项的写入范围都不覆盖该规范文件，任务清单里也没有这条回写动作；照此实施会留下「规范说尚未配置、代码已配置」的状态脱节（规范明令禁止）。

第 2 轮（`timeline_node_004`）3 条，其中第 3 条：

> WI-003 通过 TASK-009 交付了 zh-cn locale，但没有安排把规范文档中「中文 locale 尚未配置」的状态改回已落地，也没说明由谁回写，规范里那条「补齐后必须回写」的强制约定当前无人负责。

第 3 轮（`timeline_node_007`）3 条，**每条开头都写「重复上一轮未收敛的问题：」**，第 3 条：

> 重复上一轮未收敛的问题：前端补上中文语言包后，仓库里那份前端规范文档里「中文 locale 尚未配置」的缺口条目必须同步改掉，但当前计划没有任何一条任务负责回写，也没有任何工作项把该文档纳入改动范围，归属需要人工指定。

→ **reviewer 自己知道在重复报**（说明缺的是「收敛规则」，不是它的观察力）。

### 2.2 owner 的两次人工回复

| 时刻 | 回复 | 平台语义 | 结果 |
|---|---|---|---|
12:58:21 | 「这次不回写，文档先保持原样」 | SC manual revision（修订指令）| ✅ completed，产出 `artifact_version_003` |
**13:04:01** | 「计划里不写文档更新任务，本次只交付代码,交给流程那边统一回写」 | 同上 | ❌ `failure_class = validation_reject`（`human_gate_turn_6a66bea0…`）|

第 3 次 author_run（`kimi_code`，13:04:01→13:04:35，**33 秒**）产出 14415 字符（**模型有输出**），事件链停在 `provider` 事件的「normalized 57 structural heading lines via the fixed zh→en table before compile」→ **编译被拒** → 无 `artifact_version_004` → 自动化停在原地。

### 2.3 与 2026-10-30 交付批次的关系（已逐项排除）

- **写范围归属校验（`REQ-WSC-10`）没有拒它**：该错误码**只出现在 prompt 的教学句里**，不在任何 finding；逐 WI 核对 exclusive 与目标仓一致（WI-004 绑 `repository_d4f18e5…`＝gateway，其 exclusive 亦为 gateway 文件）
- **人工门面向 owner 的呈现（`REQ-WSC-11`）已生效**：门卡已呈现人话原因 + 「需要你决策一件事」+ 2 个带后果的选项 + findings 折叠

## 三、根因（三个现象同一病根）

### 根因 A｜reviewer 的 finding 范围**没有任何约束**

`src/product/workspace_engine/prompts/review.rs` 的 7 处输出 schema 只规定**格式**（severity/message/evidence/required_action）与 severity 分级，**没有一句**「finding MUST 落在本 Work Item 的 write scope / AC / CT 能力契约范围内」。

→ 「仓库内其它文档的当前状态」「产品侧规范/流程待办」被当成 WI 缺口报出。

### 根因 B｜findings **没有归属字段**，机器无法判别

schema 里**没有 `file_path`**（7 处全无），也没有 `target_work_item_id` / `belongs_to_work_item` 之类。

→ 平台**无法机器判别**「这条 finding 是否可以通过修订本 Work Item 收敛」。凡是「改 WI 也改不掉」的 finding（产品侧文档状态、外部系统状态），一律只能升级为 `needs_human` 逼 owner 拍板。

### 根因 C｜owner 的反馈**只有一种语义**：「修订指令」

`src/product/workspace_engine/prompts/human_gate_revision.rs::build_sc_manual_revision_prompt` 把 `feedback` 固定表述为「**本回合的 typed 修改范围**」，注入 `build_sc_manual_revision_prompt` 后交给 author 去改候选。

**平台没有「这件事不归本 Work Item」的表达位** → owner 想说「不归我管」只能写进 feedback 文本 → 被当成「修改范围」→ author 去改 plan → **文档状态没变** → 下一轮 reviewer 再报 → **死循环**（= owner 第 3 点）。

### 根因 D｜下一轮 reviewer **拿不到上一轮的处置**

单候选 reviewer 的输入只有 `plan_candidate_ir_ref` + `mechanical_report_ref`（`prompts/review_parts/single_candidate_input.inc.rs`）。**上一轮 reviewer 的 findings、owner 的回复与处置都没有回喂**。

→ 它只能自己重新发现「这些问题还在」（所以写了「重复上一轮未收敛的问题」），**却不知道 owner 已经处置过**。

### 根因 E｜门卡被 `validation_reject` 后**不说清发生了什么**

失败门记录只有 `failure_class = validation_reject`、`source_hash = ""`、`result_artifact_ref = null`，**没有任何诊断文本**；界面卡片只显示「✓ 可直接确认（**需先处理预判**）」——「预判」是什么一个字没说 → **owner 无法行动**（= owner 第 2 点）。

补充：编译诊断标签（`compile markdown source` / `validate plan candidate IR`）**在 aria home 里一条都没有**（全盘 grep）→ 现场无法复盘。

## 四、候选修法

### 修 1｜finding 范围契约化（治根因 A/B，最小改动、直接断掉死循环）

reviewer 的 7 处 prompt 增一条硬约束：

> finding MUST 落在**本 Work Item 的 write scope / 验收标准 / 能力契约**范围内。「仓库内其它文档的当前状态」「产品侧规范或流程待办」「与本 Work Item 无关的外部事实」**不属于 finding**；若确需提示，只能作为 `suggestion` 且 `required_action` **不得要求作者执行**。

**配套**：契约 delta（`work-item-plan-single-candidate` 或 review 契约能力）加一条 requirement + scenario。

**预期效果**：owner 第 1 点与第 3 点同时解决 —— 「前端规范文档的缺口」不再被报成 WI 缺口 → owner 不必再回复 → 循环断掉。

### 修 2｜findings 带归属，让机器能判别（治根因 B）

给 findings 增加归属信息（`target_work_item_id` / `belongs_to_work_item` 或 `file_path`），使平台能机器判别「这条能否通过改 WI 收敛」：

- 能收敛 → 进返修流
- **不能收敛（产品侧/外部事实）→ 降级为 advisory，不要求作者执行，不阻断 gate**

### 修 3｜owner 反馈的语义补全 + 处置回喂（治根因 C/D）

1. 门卡上给「**排除此项**」的表达位（每条 finding 一键排除，或反馈里带结构化标记）
2. 排除/处置**落 durable**（「已处置事实清单」）
3. **下一轮 reviewer 的 prompt 带上这份清单** → 明确「以下事实 owner 已处置，不得重复报」

### 修 4｜收敛性规则（治根因 D 的另一半）

reviewer 若要重复报某 finding，MUST 说明「上轮处置为何没有解决它」；若上轮已处置且事实未变 → **不得**重复报为 finding（可作 advisory）。

### 修 5｜门卡可操作性（治根因 E，owner 第 2 点）

门卡在 `validation_reject` 后必须显示：**你的哪一条反馈被拒 / 拒绝原因（可读） / 下一步怎么走**；并把编译诊断**落盘**（现在全丢）。

## 五、需要 owner 裁定的两处语义取舍

### 取舍 1｜「产品侧/仓库侧文档状态」这类问题怎么处理

| 选项 | 含义 | 代价 / 风险 |
|---|---|---|
**A** | **不报**（完全排除）| 最干净；但可能把真问题一起藏掉（若某文档确实该由本 WI 改）|
**B** | **报但降级为 advisory**，`required_action` 不得要求作者执行、不阻断 gate | 保留可见性（owner 能看到），不制造死循环；UI 需区分「要你处理」与「仅供参考」|
**C** | **报并进另一条流**（产品待办清单），与 WI 分离 | 最完整；但要新建载体与界面，改动最大 |

**倾向 B**：既保留可见性，又不制造「owner 必须回复才能继续」的死结。

### 取舍 2｜owner 的「这不归本 WI」怎么表达

| 选项 | 含义 |
|---|---|
**A** | 门卡上每条 finding 加「**排除此项**」按钮 → 结构化、可机器消费 |
**B** | 只靠反馈自由文本，由平台用 LLM/关键词判断「这是排除语义」 |
**C** | 两者都要（按钮为主 + 文本兜底）|

**倾向 A**（结构化才可靠；自由文本判断会引入新的不确定性）。

## 六、与既有待办的关系

- 本条**独立于** `cadence/notes/2026-10-30_问题记录_WS出站队列容量1导致角色运行假失败_v1.0.md`（那是 WS 出站队列，与本条无关）
- 本条**独立于** 2026-10-30 已交付的两个 change（`REQ-WSC-10` 写范围归属、`REQ-WSC-11` 人工门面向 owner）；但 owner 第 2 点（「用户不知道怎么继续」）与 `REQ-WSC-11` **同族**（人工门的可操作性），建议同一批处理
- 本条与 `fix-cascade-deletion`（级联删除不完整）无关

## 七、复核命令

```bash
A=/tmp/aria-deploy/.aria
S=workspace_session_auto_e5f6b290-f231-479a-9c5c-c7d63c93515e
# 三轮 reviewer findings
python3 - <<'EOF'
import json,glob,re
T='/tmp/aria-deploy/.aria/projects/project_0002/issues/issue_0001/workspace-timelines/workspace_session_auto_e5f6b290-f231-479a-9c5c-c7d63c93515e/timeline_node_details'
for f in sorted(glob.glob(T+'/timeline_node_00*.json')):
    d=json.load(open(f))
    if d.get('node_type')!='reviewer_run': continue
    sc=str(d.get('streaming_content') or '')
    m=re.search(r'"findings"\s*:\s*\[(.*?)\]\s*\}', sc, re.S)
    if m:
        for fm in re.finditer(r'"severity"\s*:\s*"([^"]+)".*?"message"\s*:\s*"(.*?)"', m.group(1), re.S):
            print(d.get('node_id'), fm.group(1), fm.group(2)[:150])
EOF
# owner 两次回复与其结果
for f in $A/projects/project_0002/issues/issue_0001/workspace-sessions/$S/human-gate-turns/*.json; do
  python3 -c "import json;d=json.load(open('$f'));print(d.get('created_at','')[11:19],d.get('status'),d.get('failure_class'),str(d.get('feedback_text'))[:60])"
done
```
