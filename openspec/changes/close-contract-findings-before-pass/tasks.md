# Tasks

> 精确文件、命令、测试与提交步骤由 `superpowers:writing-plans` 写入 `cadence/plans/`。本文件只列高层可验收工作包，每个工作包映射到一条 requirement。

## 1. 有效必关闭谓词与作用域承载

- [ ] 1.1 建立**唯一** effective-closure 谓词的服务端实现（可复用平台 disposition 硬化结果，不新增 wire 字段），并为其编写单元测试：契约类 finding 在 `severity=suggestion` 时仍判为必关闭、非契约类 finding 不受影响、判定**不改写** finding 的 `severity` 与 `message`。映射 requirement：MODIFIED REQ-WSC-17「保留原 severity 与内容身份」。
- [ ] 1.2 为谓词引入**作用域参数**并证明作用域只能由可信 session / 解析入口传入：WorkItemPlan 契约类作用域生效，通用 Story/Design 评审的既有门槛逐字不变；增加用例证明模型输出的任何标签都不能授予或扩大作用域。映射 requirement：MODIFIED REQ-WSC-17「例外作用域不扩到通用评审」。
- [ ] 1.3 确认判定顺序固定为「解析平台硬化 → durable owner 排除 → 有效必关闭 / 路由计算」，并增加用例证明 owner 已排除的契约类 finding 不会被再次平台硬化复活、排除的 key 与幂等语义不变。映射 requirement：MODIFIED REQ-WSC-17「owner 显式排除优先且排除项不得复活」。

## 2. 四个消费面同口径改造

- [ ] 2.1 将 **verdict 判定**改为消费有效必关闭谓词（`pass` 遇有效必关闭的契约类 finding MUST 重开为返修结论），并补用例：模型 `severity=suggestion` + `class_hint=advisory` + `verdict=pass` 的契约类 finding MUST NOT 保持 `pass`。映射 requirement：MODIFIED REQ-WSC-17「模型标签不得使契约类 finding 自动放行」。
- [ ] 2.2 将**必关闭清单**改为消费同一谓词，清单标题与说明不再声称仅 blocking/must_fix，保留原 severity 展示值并说明平台契约闭合义务；`NeedsHuman` 结论下未解决的契约项 MUST NOT 因 gate 不是 `RequiresRevision` 而从清单消失。映射 requirement：MODIFIED REQ-WSC-17「四面同口径」「strict/lenient/historical 三入口同一规则」。
- [ ] 2.3 将 **owner 处置再归一**改为使用同一谓词重算 verdict / gate / action，保持排除项、幂等与纯读恢复语义不变，且非本作用域的 finding 不被偷改。映射 requirement：MODIFIED REQ-WSC-17「owner 显式排除优先且排除项不得复活」。
- [ ] 2.4 将 **policy 分类**改为消费解析侧的有效必关闭事实：`class_hint=advisory` 与 raw `pass` 兜底 MUST NOT 把有效必关闭的契约类 finding 判为 `Advisory`；谓词只作为**事实源**，policy 侧不另立公式。映射 requirement：ADDED REQ-TOP-07「advisory 提示不得盖过有效必关闭」「raw pass 不得把契约类 finding 判为建议项」。
- [ ] 2.5 增加**四面同口径**的一致性用例：同一条契约类 finding 在 verdict 判定、必关闭清单、owner 再归一、policy 分类四处得出同一「是否必关闭」结论，任一处不得因只读 `severity` / `class_hint` / `verdict` 字面值而得出相反结果。映射 requirement：MODIFIED REQ-WSC-17「四面同口径」。
- [ ] 2.6 统一 **strict / lenient / historical 三条入口**：三条入口使用同一谓词与同一作用域传递方式；历史入口只重新归一展示，MUST NOT 自动启动 provider，MUST NOT 回滚已确认或已发布的历史产物。映射 requirement：MODIFIED REQ-WSC-17「严格、宽松与历史入口同一规则」。

## 3. policy 侧的许可与人工分流

- [ ] 3.1 在 policy 侧落实「有效必关闭 ≠ 可自动返修」：有可验证定位且存在既有合法机械修复证据时沿既有许可走可自动返修路径；缺可验证定位或缺既有修复许可时落 `HumanRequired` 进入既有人工处理通道。映射 requirement：ADDED REQ-TOP-07「无定位转人工而非可自动返修」。
- [ ] 3.2 增加用例证明 policy **不得**仅凭 `category` 或 `severity` 新造可自动修复许可，也**不得**因此触发自动返修或新增自动重写次数。映射 requirement：ADDED REQ-TOP-07「不得凭类别或严重度自造修复许可」。
- [ ] 3.3 验证本组改动**未提高** A 池 `max_repairs` 与 B 池 `max_manual_repairs` 余额、**未改变** Verification scope 新 finding 的既有 `VerificationNewFindings` 停止规则（既有回归保持绿）。映射 requirement：ADDED REQ-TOP-07「不提高余额且不改既有停止规则」。

## 4. 范围外 advisory 与既有回归

- [ ] 4.1 增加用例证明非契约类 advisory finding（范围外事实 / 完备度意见）继续保持 advisory：不进必关闭清单、不计非 advisory 指纹、不消耗返修预算、不阻断。映射 requirement：MODIFIED REQ-WSC-17「范围外 advisory 不被放大」。
- [ ] 4.2 复跑既有回归并确认全绿：finding disposition 判定、owner 排除匹配、policy 分类、历史压缩必关闭清单渲染、重复指纹与「同一指纹复评停止」相关既有测试；既有真实 golden findings 不得改写成新证据。映射 requirement：MODIFIED REQ-WSC-17「矩阵覆盖与基线一致」。

## 5. 基线 fixture 与验收矩阵

- [ ] 5.1 建立**五条真实 finding 的基线 fixture**并断言：恰一条契约类 finding 进入必关闭集合、整轮结论 MUST NOT 为自动通过、该 finding 的原 `severity` 与 finding key 逐字不变、无 `contract_field` 的那条 MUST NOT 自动调用 author、其余四条仍不扣预算且不阻断。映射 requirement：MODIFIED REQ-WSC-17「矩阵覆盖与基线一致」。
- [ ] 5.2 建立**覆盖矩阵**用例：category（`contract_gap` / `self_contradiction`）× severity（`suggestion` / `must_fix` / `blocking`）× disposition（`actionable` / `advisory` / 缺省）× hint（三类）× verdict（`pass` / `revise` / `needs_human`），并补齐 owner 排除优先、历史 rebuild 同 key、同一指纹复评停止、全部 advisory 保持原行为四类专项用例。映射 requirement：MODIFIED REQ-WSC-17「矩阵覆盖与基线一致」「owner 显式排除优先且排除项不得复活」「严格、宽松与历史入口同一规则」「范围外 advisory 不被放大」。

## 6. 共享面串行与集成验收

- [ ] 6.1 与 `fix-author-rewrite-quota-pool` 串行：在其预算域改动落地后，再叠加本 change 的判定口径改造；MUST NOT 在同一修改中同时动「A/B 余额」与「什么算 repairable」两处语义，并保持该 change 目录不被本 change 修改。映射 requirement：ADDED REQ-TOP-07「不提高余额且不改既有停止规则」。
- [ ] 6.2 与 `mechanicalize-plan-compile-repair` 串行：在其现场 #3 重复 finding 归因结论稳定后，再只改非 advisory 指纹集合的**成员资格**（不改编集合比较方式），并保持该 change 目录不被本 change 修改。映射 requirement：MODIFIED REQ-WSC-17「四面同口径」。
- [ ] 6.3 集成验收：全量回归通过；确认契约类 finding 在四面口径一致、缺定位时转人工而非自动 author、范围外 advisory 不被放大、owner 排除与历史 key 语义不变、自动重写次数未增加、A/B 余额未提高。映射 requirement：MODIFIED REQ-WSC-17 全部场景；ADDED REQ-TOP-07 全部场景。

## Workflow follow-up

- 本 change 的实施 Plan 写入 `cadence/plans/`，由 `superpowers:writing-plans` 产出并经确认。
- 实施完成并通过验收后，由 `openspec-archive-change` 归档；归档时把 `specs/` 下的两条 delta 同步进主 specs。
