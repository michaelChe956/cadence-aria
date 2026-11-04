## ADDED Requirements

### Requirement: 快照预算口径为本次变更量（REQ-AUD-01）

聚合初始化的文件快照审计 SHALL 以**本次变更量**为预算口径，而 NOT 以整仓存量体量为口径。

系统 SHALL 扫描全仓**两遍**以比对变更，但每遍 SHALL 只记录**元数据**（路径、大小、修改时间）而不读取文件内容；SHALL 仅对「大小或修改时间发生变化」的文件读取内容并计算内容哈希，据此得出变更清单。

该口径 SHALL **完整保留**「recipe 写到了 allowlist 之外」这一检测能力（变更分类不依赖全量内容哈希）。系统 MUST NOT 改用事件驱动（inotify 等）作为变更判定手段。

#### Scenario: 只对变更文件读内容

- **WHEN** 快照审计扫描全仓两遍且两次之间只有一个文件被改动
- **THEN** 系统 SHALL 只对该文件读取内容并计算哈希；变更清单 SHALL 只含该文件；其余文件 MUST NOT 被读取内容

#### Scenario: 全仓两遍扫描仍然保留

- **WHEN** 快照审计执行变更判定
- **THEN** 系统 SHALL 仍然扫描全仓两遍并对比两份元数据；MUST NOT 改为只扫描被改动路径

#### Scenario: 越界写入仍可被检出

- **WHEN** recipe 在审计窗口内写到了 allowlist 之外的路径
- **THEN** 系统 SHALL 检出该变更并给出越界判定；MUST NOT 因改为元数据差分而漏检

#### Scenario: 存量体量不再压垮预检

- **WHEN** 成员仓存在大量未被忽略的构建产物，但审计窗口内没有文件被改动
- **THEN** 快照预算 SHALL 按本次变更量计（接近零），MUST NOT 因存量体量超过上限而失败

### Requirement: 快照遍历跳过整个 .git 子树（REQ-AUD-02）

快照遍历 SHALL 在遇到路径任一段为 `.git` 时**整棵子树跳过**：该子树下的条目 SHALL 不计数、字节 SHALL 不计入预算、内容 SHALL 不进入快照。

该口径 SHALL 与聚合索引的排除清单（`**/.git/`）一致，同一个仓库 MUST NOT 存在两套 `.git` 口径。

`.git` 路径的**分类**判定 SHALL 保留（用于识别「命令试图修改 `.git`」这一越界行为），仅遍历进入行为改变。

#### Scenario: .git 子树完全不入快照

- **WHEN** 成员仓存在 `.git/HEAD` 等文件
- **THEN** 该文件 SHALL 不出现在快照条目中，其字节 MUST NOT 计入预算

#### Scenario: 与索引排除口径一致

- **WHEN** 比较快照遍历与聚合索引的排除清单
- **THEN** 二者对 `.git` 的处理 SHALL 一致；MUST NOT 出现「索引排除、快照计入」或反之的分裂

#### Scenario: 对 .git 的越界写入仍被判定

- **WHEN** recipe 在审计窗口内试图修改成员仓的 `.git` 路径
- **THEN** 系统 SHALL 仍按成员 Git 面判定该变更为越界；MUST NOT 因遍历跳过而漏判

### Requirement: 文件内容流式读取并即时计费（REQ-AUD-03）

快照审计读取文件内容 SHALL 采用**流式读取**：边读边累加字节、边计算哈希，预算一旦越限 SHALL 立即停止读取。

系统 MUST NOT 在计费之前把整个文件读入内存。

#### Scenario: 超大文件越限即停

- **WHEN** 待读文件远大于快照预算上限
- **THEN** 系统 SHALL 在越限点停止读取并报预算超限；内存占用 SHALL 保持在分块大小量级；MUST NOT 出现进程内存暴涨或崩溃

#### Scenario: 预算内文件正常读取

- **WHEN** 待读文件小于快照预算上限
- **THEN** 系统 SHALL 完整读取其内容并计算出内容哈希；结果 MUST 与既有实现一致

### Requirement: 预算超限的失败面结构化且文案可行动（REQ-AUD-04）

快照预算超限 SHALL 产出**结构化**的失败信息，携带：预算上限、累计读取字节数、越线时正在处理的文件路径（仅用于定位）、**按大项分组的累计**（如 `pms/.git`、`pms/target`）与**各成员仓各自的贡献**。

失败面 SHALL 给出中文文案，明确说明：

1. **不是某个文件特别大** —— 报错里点到名的文件只是快照扫到它时刚好越线
2. 当前快照构成的大项排行与成员仓归属
3. 两条操作出路：清理不影响工作的大目录；或用 `.git/info/exclude` 排除

用户指引 SHALL 明确告知**不要改 `.gitignore`**（它是被跟踪文件，改它会撞平台的写入范围门禁）；由于平台已跳过 `.git/`，指引 SHALL NOT 再要求用户把 `/.git/` 写进排除文件。

#### Scenario: 超限错误带分组与成员仓数据

- **WHEN** 快照预算超限
- **THEN** 失败信息 SHALL 携带预算上限、累计字节、越线文件路径、大项分组累计与各成员仓贡献；MUST NOT 只有一句累计字节数

#### Scenario: 文案说明不是某个文件超标

- **WHEN** 用户看到预算超限失败面
- **THEN** 文案 SHALL 明确说明「不是某个文件特别大」，并把点名文件解释为「扫到它时刚好越线」；MUST NOT 让用户误以为该文件需要单独处理

#### Scenario: 给出两条出路

- **WHEN** 用户看到预算超限失败面
- **THEN** 文案 SHALL 同时给出「清理大目录」与「写 `.git/info/exclude` 排除」两条出路，并附大项排行与所在成员仓

#### Scenario: 明确不要改 .gitignore

- **WHEN** 用户看到预算超限失败面
- **THEN** 文案 SHALL 明确告知不要改 `.gitignore` 及其原因（被跟踪文件 + 写入范围门禁）；MUST NOT 建议修改 `.gitignore`

#### Scenario: 指引不再要求排除 .git

- **WHEN** 用户看到预算超限失败面
- **THEN** 文案 MUST NOT 要求把 `/.git/` 写进排除文件（平台已跳过 `.git/`）

#### Scenario: 英文机器行不被改写

- **WHEN** 该失败被其它消费方读取
- **THEN** 结构化错误变体的 Display 文本 SHALL 保持**英文机器行**形态，并以**稳定码前缀** `audit_snapshot_budget_exceeded` 开头（`audit_snapshot_budget_exceeded: {累计字节}/{上限字节} bytes at {越线路径}`）；MUST NOT 因新增变体而破坏既有消费方
