# Spec Delta: harden-coder-write-scope-boundary

## ADDED Requirements

### Requirement: 写入范围数据与门禁同源且只在一处承载（REQ-PROMPT-04）

系统 SHALL 呈现「允许修改范围」与「禁止修改范围」。其数据 MUST 与机器门禁（Work Item 收口校验 / group `write_policy`）读取**同一份已确认 Work Item 记录**，逐字取值，MUST NOT 做仓名前缀、`./` 或聚合根变换。

范围信息 MUST 只在**一处**承载（编译后 Work Item 上下文）；coder、reviewer 与评估上下文三方同源；同一份 Prompt 内 MUST NOT 出现第二份范围列表。

#### Scenario: coder 与 reviewer 看到同一份范围

- **WHEN** 某 Work Item 已确认 `exclusive_write_scopes` 与 `forbidden_write_scopes`
- **THEN** coder Prompt、reviewer Prompt 与评估上下文 SHALL 包含同一份范围段；该段在每份 Prompt 内 SHALL 恰好出现一次

#### Scenario: group 路径的范围与机器门禁同源

- **WHEN** group 成员 Work Item 的写入范围来自 canonical 契约的 `write_policy`
- **THEN** 其 Prompt 内的范围段 SHALL 取自同一 `write_policy`，与该成员 Work Item 的收口门禁取值逐字一致

#### Scenario: group 主路径的 Prompt 同样带范围规范段

- **WHEN** attempt 属 group（schema v2）主路径，其 coder / reviewer Prompt 由投影渲染产出而非由编译后 Work Item 上下文拼装
- **THEN** 该 Prompt SHALL 仍携带与 legacy 路径**逐字同源**的范围规范段（判定基准 / 匹配语义 / 空列表语义 / 默认拒绝句），数据取自同一 `write_policy`
- **AND** 同一份 Prompt 内范围信息 SHALL 只有一份

#### Scenario: delta 轮不重复携带范围段

- **WHEN** group 或单仓走 delta-only 轮（无新的编译后上下文）
- **THEN** delta Prompt SHALL NOT 重复携带范围段（首轮 full 已带），MUST NOT 出现第二份范围

### Requirement: 写入范围的判定语义必须在 Prompt 中写明（REQ-PROMPT-05）

范围段 SHALL 写明以下判定语义，MUST NOT 与机器门禁脱节：

1. **判定基准** = 工作树根相对路径，即 `git status --porcelain` 输出的路径
2. **匹配语义** = 前缀匹配，**不是 shell glob**：含 `*` 的项取首个 `*` 之前的前缀做 starts_with；不含 `*` 且不以 `/` 结尾的项按整路径相等；单项 `*` 等于不限；首个 `*` 之前为空时机器判定等于不限制
3. **空列表语义不对称**：exclusive 为空 = 不限制可写范围；forbidden 为空 = 无额外禁止；命中任一 forbidden 项即违规，即使同时命中 exclusive
4. **默认拒绝**：范围之外的仓库文件一律不得修改（含 `.gitignore`、构建配置等全局文件）；确有必要不得自行改动，须在交付说明中标注由平台裁决

#### Scenario: agent 不得按 glob 语义误解范围

- **WHEN** 范围含 `src/**/*.ts` 或 `**/x.rs` 这类含 `*` 的项
- **THEN** Prompt SHALL 写明按前缀匹配处理，并写明首个 `*` 之前为空时机器判定等于不限制

#### Scenario: 空列表不被读成全禁

- **WHEN** exclusive 与 forbidden 之一为空
- **THEN** Prompt SHALL 分别写明「为空 = 不限制可写范围」与「为空 = 无额外禁止」

### Requirement: 范围记录缺失时 fail-closed（REQ-PROMPT-06）

当无法读取已确认 Work Item 记录、或其上下文不含范围段时，系统 SHALL 在 coder Prompt 中显式声明「范围未知」，并声明除材料明确给出的文件与验证命令外不得改动任何仓库文件；MUST NOT 渲染成「无限制」，MUST NOT 省略范围约束。

#### Scenario: 取不到记录时不得放行

- **WHEN** coder Prompt 的 Work Item 上下文不含范围段
- **THEN** Prompt SHALL 含范围未知的声明与「不得改动任何仓库文件」的禁令；MUST NOT 出现允许不受限的措辞

### Requirement: 本地产物屏蔽事实与 .gitignore 禁令在 Prompt 中显式告知（REQ-PROMPT-07）

系统 SHALL 在 coder 的**平台指令段**告知：平台已在工作树准备阶段把平台本地产物目录写入**仓库本地** ignore 文件（`info/exclude`），这些目录不会出现在 `git status`；agent **不需要也不得**为它们修改 `.gitignore`。

该告知 MUST 同时包含：目标仓 `.gitignore` 反选了这些目录时**仍不得由 agent 修改 `.gitignore`**，应在交付说明中标注由平台处理；**其他未跟踪文件属于仓库工作内容，同样不得擅自加入 `.gitignore`**。

该告知 MUST 出现在对 coder 全路径（含 delta 轮与 rework 轮）**无条件**追加的平台指令段中。

#### Scenario: agent 不得为产物噪音修改 .gitignore

- **WHEN** coder 在工作树中看到未跟踪文件或目录
- **THEN** Prompt SHALL 明确禁止其修改 `.gitignore` 以屏蔽这些内容；确有必要时 SHALL 要求在交付说明中标注由平台处理
