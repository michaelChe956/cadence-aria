# Spec Delta

## MODIFIED Requirements

### Requirement: markdown 编译器模型（REQ-WSC-02）

系统 SHALL 以 markdown/EARS 文档作为 work item plan 的唯一可编辑源，并提供确定性编译器将其单向编译为顶层 `PlanCandidateIr { source_revision_hash, compiler_version, items: Vec<PlanCandidateItemIr> }`

多仓 AC/验证路径 SHALL 使用全量授权成员基线：明确成员前缀只查对应仓，裸仓内路径只绑定本 WI 的 `target_repository_id`，MUST NOT 因其它成员同名文件存在而通过。跨成员读引用命中 MUST NOT 授予写权限；最近候选 SHALL 限于确定归属的成员与路径域。加载、校验、D5 渲染与结构摘要 SHALL 同批消费共享完整基线；显示截断 MUST NOT 缩小校验集合，固定面 SHALL 不变、上下文面 SHALL ≤32,000 B。非默认成员路径增删或归属映射变化 SHALL 改变结构 digest；摘要不扩为内容快照。

#### Scenario: markdown 与 IR 漂移被拒绝

- **WHEN** 发布前 typed IR 的 source_revision_hash 与当前 markdown 源不匹配，或 compiler_version 过期
- **THEN** 系统拒绝发布，提示重新编译；已发布的 binding 不受影响

#### Scenario: 编译错误可诊断

- **WHEN** markdown 源不满足 grammar（缺 section、ID 格式错误、EARS 句式非法）
- **THEN** 编译器返回行号、字段名与一个修复示例；该错误信息可直接作为返修反馈回喂模型

#### Scenario: 固定词表中文结构标题确定性归一化后可编译

- **WHEN** provider 交付的 markdown 把固定词表结构标题翻成已知中文变体（如 `# 工作项计划`、`## 工作项 WI-001: x`、`### 身份`、`### 写入策略 (Write Policy)`），且正文其余部分满足语法
- **THEN** 系统在编译前把结构标题行逐字归一化为规范英文并编译通过，正文中文逐字保留，并记录一条归一化诊断/事件；表外未知标题（如 `### 溯源清单`）不被猜测改写，仍按既有语法契约 fail-closed 拒绝

#### Scenario: EARS 关键字缺空格确定性补齐后可编译

- **WHEN** provider 交付的 statement 条件以 CJK 标点（如全角引号 `」`）收尾后无半角空格直接接 `THE SYSTEM SHALL`，或 `WHEN` 后缺半角空格，或关键字邻位为 U+3000/NBSP
- **THEN** 系统在编译前确定性补齐/归一为单个半角空格后编译通过；条件文本与标点本身逐字保留，并记录一条归一化诊断/事件；除关键字邻位空白外任何正文内容不被改写

#### Scenario: 归一化不救非空白语法错误

- **WHEN** statement 的语法错误不是关键字空白形态（如缺 WHEN 前缀、THEN 语义缺失、顺序错误）
- **THEN** 归一化层不修改该行，编译器照旧以 fail-closed 拒绝并返回行号诊断

#### Scenario: 重复能力行累积不丢能力

- **WHEN** 同一 Outputs contract 的 markdown 含 N 行 `capabilities`（如 F-52 CT-001 的 6 行复合/独立能力混排）
- **THEN** IR 中该 contract 的 capabilities 为全部 N 行 split 值的并集（完全相同值只保留一次），coverage 校验不再因行序或行数产生缺口；真实缺能力仍报 Error

#### Scenario: 重复需求行同构累积

- **WHEN** 同一 Inputs contract 含多行 `required_capabilities`
- **THEN** 与 capabilities 同构累积并集，不得 last-write-wins

#### Scenario: 相邻 contract 严格隔离

- **WHEN** lowering 顺序经过两个相邻 contract 且前者的字段行尚未闭合
- **THEN** 边界 flush 使前 contract 的字段不得归并到后 contract，反向亦然

#### Scenario: 累积不改写值形态

- **WHEN** 累积与去重生效
- **THEN** capability 值保持原文（顺序、大小写、标点、内部空白零改写），仅完全相同的重复元素被消除

#### Scenario: 重新编译的 IR 差异仅源于重复字段语义

- **WHEN** 含重复字段的相同 source 在修复前后各编译一次
- **THEN** 新旧 IR 的差异仅体现为重复字段的累积并集，无其他字段变化；历史 publication 不被重算或迁移

#### Scenario: options 缺口首轮即拦

- **WHEN** 创建选项 include_integration_tests=true 而候选首轮交付不含 integration 类 Work Item
- **THEN** generate/evaluate 期校验产出 Error 级 finding（附修复动作：新增对应 kind 或回创建选项关闭 flag），经既有回灌通道驱动修订；SHALL NOT 到 approve→Final Compile 才首次报错

#### Scenario: options 三族同报

- **WHEN** integration/e2e/split 三个 flag 同时启用而候选各有缺口
- **THEN** 一次校验报告全部缺口；flag=false 时保持既有 skipped-risk warning、不产生 Error

#### Scenario: AC 引用基线外路径被拦

- **WHEN** 候选的验收标准或验证计划引用仓库内文件路径（如某分支才存在的文件），而该路径不存在于 plan 基线树
- **THEN** generate/evaluate 期产出 Error（附路径清单与三种修复建议：删除该 AC / 改用基线内路径 / 将新建文件先提交入基线或改引基线内既有脚本，不以写范围声明豁免）；SHALL NOT 让 coder 在执行期发现并被迫越界恢复基线

#### Scenario: item 映射为 typed IR

- **WHEN** 编译器产出每个 item
- **THEN** 每个 item SHALL 为 `PlanCandidateItemIr { target_repository_id, contract, verification_plan: WorkItemDraftVerificationPlan, trusted_commands }`

#### Scenario: 版本指纹仅顶层

- **WHEN** 检查 typed IR 中 source revision hash 与 compiler version 的位置
- **THEN** typed IR 的 source revision hash 与 compiler version 仅位于顶层

#### Scenario: 发布前校验指纹

- **WHEN** **publish 前** hash 或版本不匹配
- **THEN** **publish 前** hash 或版本不匹配时系统 SHALL 拒绝发布并提示重新编译，hash/version 随不可变 publication provenance 落盘

#### Scenario: 执行只消费发布绑定

- **WHEN** coding 段执行期间取用计划内容
- **THEN** coding 段只消费已发布的 immutable runtime binding，SHALL NOT 在执行期间解析 markdown 或重新解释 compiler version

#### Scenario: 安全边界强类型

- **WHEN** 编译产出承载 write_policy 与 trusted commands
- **THEN** write_policy 与 trusted commands 等安全边界 SHALL 保持强类型

#### Scenario: 修改触发重编译

- **WHEN** 对 markdown 的人工或模型修改发生
- **THEN** 对 markdown 的人工或模型修改 SHALL 产生新 revision 并触发重新编译

#### Scenario: 结构标题行归一化

- **WHEN** SC 交付（author 与人工修订两条路径）进入编译前
- **THEN** SC 交付（author 与人工修订两条路径）进入编译前，系统 SHALL 允许两类确定性归一化：其一为结构标题行（一级文档标题、Work Item 二级标题、三级结构 section 标题）归一化——以固定中文→英文映射表逐字映射回规范英文，仅覆盖固定词表的已知翻译变体

#### Scenario: EARS 空白归一化

- **WHEN** statement 行内 `WHEN` 后或 `THE SYSTEM SHALL` 前缺失半角空格、或关键字邻位为 U+3000/NBSP
- **THEN** 其二为 **EARS 关键字空白归一化**——当 statement 行内 `WHEN` 后或 `THE SYSTEM SHALL` 前缺失半角空格、或关键字邻位为 U+3000/NBSP 时，确定性补齐/归一为单个半角空格

#### Scenario: 归一化零触碰正文

- **WHEN** 两类归一化执行
- **THEN** 两类归一化均只触碰上述空白与固定词表位置，正文其余内容（含 CJK 标点如全角引号）零触碰

#### Scenario: 未知标题不猜测

- **WHEN** 结构标题行归一化遇到表外未知标题
- **THEN** 表外未知标题 SHALL NOT 被猜测改写，SHALL 照旧交给编译器以 fail-closed 拒绝

#### Scenario: 归一化落诊断

- **WHEN** 任一归一化发生
- **THEN** 任一归一化发生时系统 SHALL 落一条可判定该次交付被救回的诊断/事件

#### Scenario: 重复字段累积并集

- **WHEN** lowering 处理同一 Outputs contract 的多行 `capabilities`（或同一 Inputs contract 的多行 `required_capabilities`）
- **THEN** **lowering 重复字段累积语义**：同一 Outputs contract 的多行 `capabilities`（或同一 Inputs contract 的多行 `required_capabilities`）SHALL 按出现顺序累积为全体行的并集（每行 split 后的值依次 append），完全相同的 capability 值仅保留首次出现

#### Scenario: 禁止末行覆盖

- **WHEN** lowering 累积多行重复字段
- **THEN** SHALL NOT 以最后出现的行覆盖先行行（last-write-wins 禁止）

#### Scenario: 相邻契约严格隔离

- **WHEN** lowering 经过相邻 contract 的字段
- **THEN** 相邻 contract 的字段 SHALL 严格隔离（contract 边界 flush），不得跨 contract 归并或串项

#### Scenario: 累积不改原文形态

- **WHEN** 累积与去重执行
- **THEN** 累积与去重 SHALL 不改变 capability 值的原文形态（不排序、不改大小写/标点/内部空白）

#### Scenario: 校验携带存储选项

- **WHEN** 构建 SC 候选校验上下文
- **THEN** **候选校验携带存储 options 与基线树**：SC 候选校验上下文 SHALL 携带创建计划时落库的 `IssueWorkItemPlanOptions` 与 plan 基线（worktree fork base / target ref 树），SHALL NOT 从 IR items 反推 options

#### Scenario: 选项一致性前移

- **WHEN** 校验 options×items 一致性（integration_work_item_required / e2e_work_item_required / frontend_backend_split_required 三族）
- **THEN** options×items 一致性（integration_work_item_required / e2e_work_item_required / frontend_backend_split_required 三族）SHALL 在 generate/evaluate 期校验，Error 级结果经既有机械 ReviewVerdict 回灌修订，SHALL NOT 延迟至 Approval/Final Compile 才首次出现

#### Scenario: 基线路径交叉核对

- **WHEN** 验收标准（AC）/验证计划引用仓库内文件路径
- **THEN** 验收标准（AC）/验证计划引用的仓库内文件路径 SHALL 与基线树交叉核对：引用基线中不存在的路径 SHALL 产生 Error（附路径清单与修复建议），SHALL NOT 进入人工确认后才由 coder 发现

#### Scenario: 明确 frontend 或 gateway 基线路径不再误拒

- **WHEN** 多仓候选的 AC 或验证计划引用 `frontend/src/App.vue` 或 gateway 的明确成员路径，且该仓对应已提交基线树真实存在该路径
- **THEN** 校验 SHALL 接受该路径的基线存在性，MUST NOT 因 issue.repo_id 指向 busi 而产生 acceptance_path_not_in_baseline；此接受面是构造可达行为变化，MUST NOT 作为本批真实 run 收益

#### Scenario: 同名文件按明确成员前缀消歧

- **WHEN** busi 与 frontend 均有 package.json，候选分别引用 `busi/package.json` 与 `frontend/package.json`
- **THEN** 两个引用 SHALL 分别解析到各自明确成员并在各自树中核对，MUST NOT 折成无归属的同名命中

#### Scenario: 明确成员路径不得回退到其它仓

- **WHEN** 候选引用 `frontend/package.json`，frontend 基线没有该文件但 busi 基线存在同名文件
- **THEN** 该引用 SHALL 产生基线缺失 Error，MUST NOT 因其它成员存在 package.json 而通过

#### Scenario: 裸路径仅绑定本 WI 目标仓

- **WHEN** busi WI 的 target_repository_id 绑定 busi，AC 引用不带成员前缀的 `src/App.vue`，而该路径仅在 frontend 基线存在
- **THEN** 校验 SHALL 在 busi 基线内判定并产生 Error，MUST NOT 在任意成员存在即通过；busi 自身基线存在的裸仓内路径仍按兼容语义核对

#### Scenario: AC 命中不豁免跨仓 exclusive 越界

- **WHEN** busi WI 明确读引用的 frontend 路径在 frontend 基线存在，同时声明了超出自身授权成员的 exclusive 写范围
- **THEN** AC 基线存在性 SHALL 通过，但跨成员 exclusive 越界 SHALL 继续产出 Error，MUST NOT 因读引用命中而扩展写权限

#### Scenario: 基线外文件不因写范围声明而通过

- **WHEN** 候选引用仅其它分支存在、未提交或新建尚未进入所选基线的脚本，且该路径落在声明的写范围内
- **THEN** generate/evaluate 期 SHALL 继续产生 acceptance_path_not_in_baseline Error，保留路径清单与删除 AC、改用基线内路径、先提交入基线或改引既有脚本三类修复建议，MUST NOT 用写范围声明豁免

#### Scenario: 最近候选限制在正确成员与路径域

- **WHEN** frontend 明确路径或 frontend WI 的裸路径缺失，frontend 内存在可用最近候选，而 busi 也有同名或同目录形态文件
- **THEN** 回灌候选 SHALL 仅来自已确定 frontend 成员的完整基线并使用正确路径域，MUST NOT 把 frontend 引用导向 busi 文件；归属成员内无候选时 SHALL 如实说明，不臆造或跨仓搜索

#### Scenario: 四端与全部校验入口消费同一基线契约

- **WHEN** 同一多仓事实分别进入基线加载、候选校验与最近候选、D5 渲染、结构摘要，以及生成评估和对话式修订的校验入口
- **THEN** 全部入口 SHALL 使用同一授权成员范围及成员归属结构，MUST NOT 保留旧单树消费或仅修 author 输入一端；相同引用的基线存在性结论 SHALL 一致

#### Scenario: D5 分组路径可回溯到完整基线

- **WHEN** D5 展示 busi、gateway、frontend 的路径清单，其中各仓均有 src 目录
- **THEN** 分组标签 SHALL 来自真实成员归属而非目录首段，每条已显示的成员前缀路径 SHALL 能在对应完整成员树中核实，MUST NOT 把 src 当仓名或制造基线外路径

#### Scenario: D5 截断不缩小 validator 事实集合

- **WHEN** D5 因上下文配额截断或省略清单，而被省略的明确成员路径真实存在于完整基线，且候选引用它
- **THEN** validator SHALL 仍对全量基线核对并接受该路径的存在性，MUST NOT 把显示子集当成基线事实；结构摘要亦 SHALL 使用完整基线而非截断文本

#### Scenario: 多仓渲染保持两面预算

- **WHEN** 将完整三成员基线按既有 D5 配额机制渲染并装入 author 上下文，包含其它既有上下文段
- **THEN** 上下文面 SHALL ≤32,000 B，固定规范面 SHALL 逐字节不变且保持既有 23,500 B 预算，MUST NOT 提高预算常量或以动态段名义逃避计量；主方案字节数只作基线对照，实施后 SHALL 复测

#### Scenario: 非默认成员路径增加改变结构摘要

- **WHEN** 默认成员基线与其它结构事实不变，仅在一个非默认授权成员的基线分支提交中增加一个路径
- **THEN** 重新构建的结构上下文 digest SHALL 改变，MUST NOT 因 issue.repo_id 未变化而沿用旧摘要

#### Scenario: 非默认成员路径删除改变结构摘要

- **WHEN** 默认成员基线与其它结构事实不变，仅从非默认授权成员基线删除一个路径
- **THEN** 结构上下文 digest SHALL 改变，MUST NOT 继续仅摘要默认成员树

#### Scenario: 路径相同但归属映射变化改变结构摘要

- **WHEN** 基线路径字符串集合保持相同，但物理成员身份与 checkout 前缀的归属映射改变
- **THEN** 结构上下文 digest SHALL 改变，MUST NOT 只对无归属路径字符串集合求摘要

#### Scenario: 摘要仍限于路径与归属事实

- **WHEN** 文件内容或提交 HEAD 变化但基线路径集合与成员归属映射均未变化
- **THEN** 本契约 MUST NOT 把该变化宣称为现有路径摘要已检测的事实，MUST NOT 借本 change 扩成全仓内容快照
