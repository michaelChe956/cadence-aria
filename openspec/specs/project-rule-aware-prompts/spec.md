## Purpose

确保流程型 Provider prompt 读取并遵循当前项目规则，同时不依赖外部 Cadence 路径或知识库禁令。

## Requirements

### Requirement: Provider prompt 使用当前项目规则（REQ-PROMPT-01）

逻辑代码库流程 SHALL 在实际 provider 运行或索引/规划准入前预检其将消费的聚合政策、成员规则材料和 capability/gateway 条件。缺失、非法、过期或 digest 不一致时，系统 SHALL fail-closed 并在 provider 启动前生成可诊断等待项，明确缺失材料、目标和准备/重试操作；用户完成准备后回到原链继续。有效 prompt 的规则依据仍 SHALL 来自 envelope 校验过的聚合政策正文或受控 artifact 引用，最小成员指针仅用于发现/路由，不能作为政策正文或缺失策略的 fallback。

#### Scenario: 逻辑代码库 prompt 以聚合政策为权威

- **WHEN** 逻辑代码库流程构建 planning/coding prompt
- **THEN** 规则依据 SHALL 来自 envelope 校验过的聚合政策；最小指针不作为政策正文；未加载有效政策时 SHALL 阻塞

#### Scenario: 逻辑代码库规则材料缺失

- **WHEN** 逻辑代码库构建 planning/coding prompt 前发现实际 provider 将消费的规则材料缺失
- **THEN** 系统 SHALL 在 provider 启动前停等并通知用户，提供可重入准备/重试操作；准备成功后使用校验过的聚合政策继续，不在运行时才以 Failed 暴露缺口

#### Scenario: 规则 digest 或 capability 不一致

- **WHEN** policy revision/digest、成员规则引用或 provider capability 与准入快照不一致
- **THEN** 系统 SHALL 拒绝该次启动并展示核验/重新准备操作，不回落到目标根同名文件、不伪造 capability 成功、不清除既有错误事实

### Requirement: Prompt 不依赖外部 Cadence 路径或知识库禁令（REQ-PROMPT-02）

逻辑代码库 prompt 对聚合政策与成员指针的引用 SHALL 由唯一 authority resolver 产生，声明权威根并携带 policy revision/digest；provider MUST NOT 通过目标 worktree、项目级历史布局或外部 Cadence 路径猜测同名材料。若 resolver 无法唯一解析或策略不可读，流程 SHALL 停等并提供核验/修复操作。

#### Scenario: 聚合政策引用须声明根

- **WHEN** prompt 引用聚合政策或成员指针
- **THEN** 引用 SHALL 声明权威根并携带 policy revision/digest 供审计，不得猜测同名路径

#### Scenario: policy 引用无法唯一解析

- **WHEN** 同名规则文件存在于多个布局，或 authority/path 与 policy digest 不一致
- **THEN** prompt 构建 SHALL 被拒绝并返回迁移/核验等待项，不选择“最新”或任一路径继续

### Requirement: 单仓生成类 Prompt 的规则引用按需化（REQ-PROMPT-03）

系统 SHALL 在 Legacy（单仓）上下文中，对生成类 prompt（outline、draft、plan、revision 及其生成侧 review）的规则引用仅声明目标仓库规则文件位置并允许按需查阅；规则读取 MUST NOT 作为生成输出的前置条件，规则文件或读取工具不可用时 MUST NOT 阻塞生成或强制转入 blocker。Legacy 上下文中的 coding 阶段 prompt 不受本要求约束，维持既有完整读取与失败关闭行为。

#### Scenario: 单仓 outline 生成不因规则文件缺失而阻塞
- **WHEN** Legacy 上下文构建 Work Item Group outline 生成 prompt 且目标仓库缺失 AGENTS.md 或 CLAUDE.md
- **THEN** prompt SHALL 仅包含规则位置声明与按需查阅提示，Provider SHALL 可正常输出候选 outline，不得被要求先阻塞报告

#### Scenario: 单仓 draft 生成不强制完整读取规则文件
- **WHEN** Legacy 上下文构建 Work Item Draft 生成 prompt
- **THEN** prompt SHALL NOT 包含"完整读取 AGENTS.md 与 CLAUDE.md 后才允许输出 JSON"类前置门禁

#### Scenario: coding 阶段保持既有规则门禁
- **WHEN** Legacy 上下文构建 coding 阶段 prompt
- **THEN** 规则引用 SHALL 维持现状（完整读取与失败关闭），不受本变更影响

#### Scenario: 逻辑代码库政策门禁不受影响
- **WHEN** Logical 上下文构建任一生成类或 coding prompt
- **THEN** 聚合政策权威加载与"未加载即阻塞"行为 SHALL 与既有 REQ-PROMPT-01/02 完全一致

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
