# 提案：把 SC generate 段教学重驱收敛为单一耐久授权窗口（通道 C）

## Why

SC 候选生成链目前有两套彼此独立的教学重驱布尔槽（compile/IR 槽与结构槽），最坏可造成原调用之外**两次**额外 provider 驱动，违反 REQ-WSC-09「每 candidate 至多一次额外驱动」；而这两套槽只是进程内布尔量，既没有耐久授权窗口，也没有真实启动账本——重连、恢复或同请求重放即可重新获得机会，既可能超发也可能凭空多发。同时 REQ-SG-05 要求「实际 start MUST 有 ledger」「上限/授权无法证明则零额外调用」，现状两头都不满足。

本次把 compile / typed lowering / typed IR / structure 的教学重驱收敛为**同一个耐久授权窗口**，原调用在内最多两次，取消独立结构第三次；并给该窗口一个独立的、不由 A/B 池余额兜底的预算归属与可审计流水。

## What Changes

- **取消独立 structure 槽**：四类可教学失败（parse 语法、事实完整的 typed lowering、事实完整的 typed IR 校验、确定性结构拒收）共享同一个 RepairOnce 窗口；跨类别不新开机会，结构不再获得独立的第三次。
- **新增耐久授权窗口**：教学重驱绑定原生成 run/reservation/phase，以原生成授权身份为锚；新 source revision、新 native 会话、重连、恢复与同请求重放都不重置窗口；原调用在内最多两次，无批准 RepairOnce 时最多一次。**RepairOnce 批准由窗口建立即授予**（owner 裁决 2026-11-09）：初轮新授权在建立窗口的同一事务内一次性核验「原授权可证明 + provider 路径合法 + 剩余机会 ≥1 + 失败事实完整」四项并冻结批准规则版本；**不新增签发方或授权载体**，批准结论不因部署升级、重连、恢复、显式 fresh 或改档而改变，MUST NOT 事后补造。
- **预算归属独立有界**：通道 C 不消耗评审机械池 `max_repairs`，不消耗 author 重写/反馈池 `max_manual_repairs`，不增加 `repairs_used` / `manual_repairs_used`，不伪造用户 turn/hold/receipt；A/B 池的触发域、余额与计费语义不变，且不得由 A/B 池余额为 C 的第三次调用兜底。
- **claim 与真实启动分账**：每次机会在任何外部启动前原子 claim，耐久绑定授权 kind、原 generation 身份、请求与材料摘要、失败 source 引用与 hash、context digest、允许修改闭包、指令与 prompt digest、attempt key、物理上限（max ∈ {1,2}，由 RepairOnce 批准状态决定）与冻结执行策略摘要字段；该 attempt 的实际启动写入既有 provider-start ledger（以 launch 身份为键并携带 attempt 键关联，不为同一次启动另插第二条记录），与 claim 分账。
- **失败处理分流**：授权/材料/上限/持久 claim 或 ledger 不可证明时零额外调用并显式系统故障；流水写失败只记日志不阻断；两者不得互相冒充。
- **流水补阶段与 attempt 关联**：REQ-WSC-25 的追加流水补 generate/parse/lowering/IR/structure 阶段与 provider attempt 关联，并保持其「记录写失败不阻断生成」义务逐字不变。
- **停止语义**：第二次交付仍失败或出现修复范围越权时，停止该生成自动链、保留 last-valid refs 与失败历史，不提升坏 candidate、不发起 review/confirm、不旁路到另一自动重写入口。

## Capabilities

### New Capabilities

- 无：本 change 不新建 capability，只修改既有 capability 的 requirement。

### Modified Capabilities

- `work-item-plan-single-candidate`：ADDED REQ-WSC-28 / REQ-WSC-28A / REQ-WSC-28B / REQ-WSC-28C / REQ-WSC-28D / REQ-WSC-28E（generate 段教学重驱的统一授权窗口、通道 C 的独立有界预算归属、教学机会的原子 claim 与耐久绑定、claim 与真实启动分账、流水按 attempt 关联、第二次失败与越权停止）；MODIFIED REQ-WSC-25（流水补 generate/parse/lowering/IR/structure 阶段与 provider attempt 关联，其 non-blocking 义务逐字保留）。
- `workspace-structural-governance`：MODIFIED REQ-SG-05（SC 初轮行绑定 GenerationRun 授权窗口与既有 provider-start ledger，接通 claim/actual start 分账与 replay/Unknown 场景；SC 人工修订的计费与上限不变）。

**关于条款形态**：决策报告 §1.3 以**单条** requirement 给出通道 C 的条款全文。OpenSpec 对 ADDED requirement 的描述文本有 500 字符硬上限（`MAX_REQUIREMENT_TEXT_LENGTH`，`openspec validate --strict` 不过即失败），而 MODIFIED 豁免该上限。为保留 §1.3 的全部规范性语义且通过校验，本 change 把 §1.3 按自然段拆为 REQ-WSC-28 / 28A / 28B / 28C / 28D / 28E 六条，逐句保留、无删减、无放宽；逐段映射见 `design.md`「§1.3 条款全文 → delta 条款映射」。

## Impact

- **契约层**：上述两个 capability 的 delta；不改动 `openspec/specs/**` 主规格文件。
- **跨 change**：资格清单的 typed IR 与 structure 扩充归 `mechanicalize-plan-compile-repair` 的 MODIFIED REQ-WSC-09（由集成 owner 执行，本 change 不改该目录），该扩充完成前本 change 不得归档（归档硬门禁，见 design）；本 change 的 A/B 池边界不写入 `fix-author-rewrite-quota-pool`。
- **实现面（由后续 Plan 展开）**：SC 生成会话的耐久授权窗口与 store CAS、provider-start ledger 的受控观察接口、generate 段教学重驱的单槽收口、尝试流水的阶段与 attempt 字段。
- **数据兼容**：新增耐久字段缺省即「未知、零额外调用」，旧记录不得倒填授权或实际启动证据；已写新格式的数据只能由兼容版本读取。
- **不涉及**：本 change 不复开已归档的结构治理 change，不把通道 C 塞进 author 重写/反馈共池 change。
