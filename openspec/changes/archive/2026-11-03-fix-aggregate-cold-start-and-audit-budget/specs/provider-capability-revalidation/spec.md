## ADDED Requirements

### Requirement: 能力状态可机读与拒绝分类（REQ-PCR-01）

provider capability 的 launch 分格 SHALL 以**稳定小写 token** 表达其状态（`confirmed` / `denied` / `unknown`），错误文案与任何分类器 SHALL 共用该词表；MUST NOT 依赖 Debug 格式化输出（`{other:?}`）来推断状态。

被 capability 门拒绝时，系统 SHALL 给出**可机读的拒绝分类**：记录缺失或分格为未核验 → 未核验（Unverified）；分格为 `Denied{reason}` → 真实拒绝（Denied，携带真实原因）；capability snapshot 引用漂移 → 证据过期（EvidenceStale）。分类 SHALL 由**既有**拒绝原因文本判定，MUST NOT 新增一套独立的能力判定逻辑；判定 MUST NOT 放宽 gateway 的准入（只有 `Confirmed` 放行）。

无法归入上述三类的拒绝 SHALL **保持既有错误码与既有文案形态**（零回归），MUST NOT 被改写成三态之一。

#### Scenario: 三态各有稳定 token

- **WHEN** capability launch 分格分别为 `Confirmed` / `Denied{reason}` / `Unknown`
- **THEN** 系统 SHALL 分别产出稳定 token `confirmed` / `denied` / `unknown`；MUST NOT 产出依赖 Debug 输出的可变文本

#### Scenario: 未核验被归为 Unverified

- **WHEN** 拒绝原因为「能力记录缺失」或「某 action 的 launch 分格为未核验」
- **THEN** 分类 SHALL 为**未核验（Unverified）**；MUST NOT 与「实测不支持」混为一类

#### Scenario: 真实拒绝携带真实原因

- **WHEN** 拒绝原因为「某 action 的 launch 分格被判定为 denied」并带原因文本
- **THEN** 分类 SHALL 为**真实拒绝（Denied）**且 SHALL 携带该真实原因；MUST NOT 把原因藏进括号或 Debug 文本

#### Scenario: snapshot 漂移归为证据过期

- **WHEN** 拒绝原因为 capability snapshot 引用与记录不一致
- **THEN** 分类 SHALL 为**证据过期（EvidenceStale）**；MUST NOT 归为未核验

#### Scenario: 不匹配时零回归

- **WHEN** 拒绝原因不匹配上述任何一类
- **THEN** 系统 SHALL 产出与改造前**逐字节相同**的 `reason_code` 与 `action`；MUST NOT 改写文案

#### Scenario: 准入不放宽

- **WHEN** 任意 provider 的某 action 行不是 `Confirmed`
- **THEN** gateway SHALL 继续 fail-closed 拒绝；MUST NOT 因新增了分类能力而放行

### Requirement: 能力相关失败面按三态分流并逐家点名（REQ-PCR-02）

与 provider 能力相关的失败面 SHALL 按三态分流为三个稳定码：未核验 → `aggregate_provider_capability_unverified`，实测拒绝 → `aggregate_provider_capability_denied`，证据过期 → `aggregate_provider_capability_evidence_stale`；三路文案 SHALL 与 provider 选择阶段的结论复用**同一套文案来源**。

三类失败 SHALL **逐家点名**：候选表 SHALL 齐全列出四家候选并按 owner 顺序排列，每家 SHALL 给出命令可用性、命令原因码、能力状态与**可执行动作提示**。机器码与原始 detail SHALL 降级为次要行（复用既有 `details.stderr_summary`，MUST NOT 改变 DTO 形状）。核验指引 SHALL 只指向**单个** provider 的核验入口，MUST NOT 指向尚未提供的「一键核验全部」入口。

#### Scenario: 选择阶段的四种结论各有稳定码

- **WHEN** provider 选择阶段得出结论
- **THEN** 四种结论 SHALL 各自有稳定码：无可用命令 → `aggregate_initialization_no_provider_available`；有命令但都未核验 → `aggregate_initialization_provider_needs_verification`；有命令但都被实测拒绝 → `aggregate_initialization_provider_capability_denied`；provider health 异常 → `aggregate_initialization_provider_health_degraded`；四者的文案 SHALL 与执行期三态文案同源

#### Scenario: 未核验给出单家核验指引

- **WHEN** 失败原因是某 provider 的能力尚未核验
- **THEN** 失败码 SHALL 为 `aggregate_provider_capability_unverified`；文案 SHALL 点名该 provider 并说明「已安装但未核验」；`details.revalidate` SHALL 给出可点击的核验入口（含 `ui_path` / `button_label` / `method` / `path` / `body` / `expect`）；MUST NOT 指向一键核验全部

#### Scenario: 真实拒绝指向下一家

- **WHEN** 失败原因是某 provider 的能力被实测判定为不支持，并带真实原因
- **THEN** 失败码 SHALL 为 `aggregate_provider_capability_denied`；文案 SHALL 点名该 provider 与真实原因，并给出优先顺序中的下一家；`details.fallback_provider` 与 `details.revalidate` SHALL 指向下一家

#### Scenario: 证据过期要求重新核验

- **WHEN** 失败原因是 capability snapshot 引用漂移
- **THEN** 失败码 SHALL 为 `aggregate_provider_capability_evidence_stale`；文案 SHALL 说明证据已作废并要求重新核验该 provider

#### Scenario: 未核验与实测拒绝不共用一句文案

- **WHEN** 两种失败分别发生
- **THEN** 二者 SHALL 使用不同的稳定码与不同的主文案；MUST NOT 出现同一句文案覆盖两种语义

#### Scenario: 三类候选结果逐家点名

- **WHEN** 产生「需要核验」「实测拒绝」或「无可用」类失败
- **THEN** 失败详情 SHALL 齐全列出四家候选（按 owner 顺序），每家 SHALL 含命令可用性、命令原因码、能力状态与可执行动作提示；MUST NOT 只说「没有可用 provider」而不点名

#### Scenario: 无可用命令时给出安装线索

- **WHEN** 四家候选的命令均不可用
- **THEN** 失败码 SHALL 为 `aggregate_initialization_no_provider_available`；每家候选的动作提示 SHALL 指向安装方式；MUST NOT 归因为「未核验」

#### Scenario: 机器码降级且 DTO 形状不变

- **WHEN** 上述任一失败面被投出
- **THEN** 原始 detail SHALL 经既有 `details.stderr_summary` 字段作为次要行呈现；MUST NOT 改变 DTO 字段形状，也 MUST NOT 把机器码当主文案

#### Scenario: 不匹配既有分类的失败保持原样

- **WHEN** 某条 provider turn 失败不属于能力三态
- **THEN** 失败码 SHALL 保持既有 `aggregate_{step}_failed` 形态与文案；MUST NOT 被改写为能力三态之一
