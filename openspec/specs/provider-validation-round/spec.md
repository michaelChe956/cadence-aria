# provider-validation-round Specification

## Purpose
provider 批（change ①）验证轮与归档收尾契约：kimi coding 验证轮以真实 CLI 跑真实链并产出结论；结论只 gate kimi 自身状态流转（转正 or 受限登记），不作为任何其他 change 的门禁；claude headless 修复验证与 kimi 验证轮共享 campaign 基建；两 active change（add-kimi-code-provider/add-pi-provider）以「勾选与代码事实对齐+遗留显式登记」的纪律归档收尾。

## Requirements

### Requirement: 验证轮真实链与基建共享（REQ-PVR-01）

kimi coding 验证轮 SHALL 以真实 Kimi Code CLI（非 fake/fixture 替身）覆盖 Coding Workspace 三角色：Coder、Code Reviewer、Internal Reviewer，每角色至少 1 案例到达真实终态；claude headless 修复验证（真实 CLI smoke）SHALL 与 kimi 验证轮共享 campaign 基建（`cadence/reports/workitem-coding-campaign` 驱动器族，显式非 dry-run）。验证轮暴露的真实缺陷 SHALL 按既有契约修复（TDD）并回归；未在真实轮覆盖的面（如普通 Workspace 角色、image-create）SHALL 如实记为未覆盖，MUST NOT 虚称已验证。

#### Scenario: coding 三角色真实到达终态

- **WHEN** 验证轮以真实 kimi CLI 运行 Coder、Code Reviewer、Internal Reviewer 各至少 1 案例
- **THEN** 每角色案例到达真实终态（成功或如实登记的失败），全程无 fake provider 替身参与结论路径

#### Scenario: 暴露缺陷按契约修复

- **WHEN** 真实轮暴露 kimi 链路缺陷
- **THEN** 以失败测试先行修复并回归，修复不超出既有契约语义（不新增能力、不改协议边界）

#### Scenario: 未覆盖面如实登记

- **WHEN** 验证轮结论落盘
- **THEN** 真实轮覆盖面与未覆盖面（普通 Workspace 角色、image-create 等）在证据矩阵中分栏如实记录，未覆盖面不以单测/前端回归证据冒充真实轮证据

### Requirement: ACP 方言核对前置（REQ-PVR-02）

kimi 验证轮主体 SHALL 在 ACP 方言核对之后执行：以现行 Kimi Code CLI（版本以实跑 `kimi --version` 记录于核对矩阵为准）对冻结自 0.34.0 的 ACP fixtures 做 wire shape 逐方法核对（initialize、session/new、session/prompt、session/request_permission、session/load 等）。存在方言漂移时 SHALL 先更新 fixture 并适配适配器，再进验证轮——MUST NOT 在已知方言漂移未处置的状态下出验证结论。

#### Scenario: 方言一致直接进轮

- **WHEN** 现行 CLI 的 wire shape 与冻结 fixtures 逐方法一致
- **THEN** 验证轮直接开始，核对记录留档

#### Scenario: 方言漂移先适配

- **WHEN** 现行 CLI 某方法的 wire shape 与冻结 fixture 漂移
- **THEN** fixture 更新+适配器适配先行完成并回归，验证轮在适配后执行；漂移事实与处置留档

### Requirement: 验证结论的 gate 边界（REQ-PVR-03）

kimi 验证轮结论 SHALL 只 gate kimi 自身状态流转（REQ-PVR-04），MUST NOT 作为退役（REQ-WSC-07 门）、多仓 coding（change ②）或任何其他 change 的门禁、输入前提或排序依据——REQ-WSC-07 判据原文仅含 codex 与 pi。本口径 SHALL 贯穿本 change 的全部产物与文案。

#### Scenario: 结论不影响退役门判据

- **WHEN** kimi 验证轮得出任意结论（转正或受限登记）
- **THEN** 退役门 REQ-WSC-07 的判据口径不变（仍仅 codex+pi），change ③ 门重测不因 kimi 结论增删判据

#### Scenario: 结论不影响其他 change 排序

- **WHEN** kimi 验证轮受限登记或失败
- **THEN** 本 change 其余 WP（claude 补验、flaky 根治、归档收尾）照常收口，不因 kimi 结论阻塞

### Requirement: kimi 状态流转二值（REQ-PVR-04）

kimi 在 provider 目录中的验证状态 SHALL 取二值之一：`转正`（真实轮通过——三角色到达真实终态且无未登记阻断缺陷）或 `受限登记`（实跑存在如实可登记的限制/缺陷——受限面、证据与限制说明登记入台账并反映在 spec/目录标注）。受限登记是合法终态（3.6 先例：限制如实标注可归档），MUST NOT 被表述为「验证失败挂起」；状态流转 SHALL 以证据矩阵为依据落盘。

#### Scenario: 真实轮通过转正

- **WHEN** 验证轮三角色均到达真实终态且无未登记阻断缺陷
- **THEN** kimi 状态流转为转正，证据矩阵留档

#### Scenario: 存在限制时受限登记

- **WHEN** 实跑暴露如实可登记的限制或缺陷
- **THEN** kimi 状态流转为受限登记：受限面+证据+限制说明登记入台账并反映在 spec/目录标注，change 照常收口

### Requirement: 归档核账纪律（REQ-PVR-05）

add-kimi-code-provider 与 add-pi-provider 的归档 SHALL 以双向核账为前置：tasks 勾选与代码事实逐项对齐——实做大于账面的项补勾并附证据（file:line 或提交号），勾而未落的项如实改回并登记。add-pi 遗留 2.2（aria-ask.ts 结构化提问扩展）与 2.3（版本范围检测）SHALL 显式 defer 登记（不实施、限制如实标注），MUST NOT 静默勾选或删除。fix-claude-code-ask-user-question-headless SHALL 按被吸收处置：其契约以本 change `claude-code-structured-interaction` delta 为唯一权威版本，MUST NOT 产生同名 capability 双版本。归档动作 SHALL 在确认无并行会话持有后由唯一 owner 执行。

#### Scenario: 实做大于账面补勾留证

- **WHEN** add-kimi 某 task 的工程面已落地但账面未勾选
- **THEN** 归档前补勾并附 file:line 或提交号证据；核账记录留档

#### Scenario: 遗留项显式 defer

- **WHEN** add-pi 2.2/2.3 归档处置执行
- **THEN** 两项保持未勾选并显式登记 defer（pi 结构化提问维持文本暂停信号现状、版本范围检测未做），限制如实标注，不静默勾选

#### Scenario: 吸收处置无双版本

- **WHEN** 三个 change 文件夹归档完成
- **THEN** 主 specs 中 claude-code-structured-interaction 仅存在本 change delta 落地的唯一版本

#### Scenario: 归档唯一 owner

- **WHEN** 归档动作执行前
- **THEN** 已确认无并行会话持有归档动作，确认记录留档，归档由单一 owner 完成
