## ADDED Requirements

### Requirement: SC 逐轮失败证据与原始输出耐久绑定（REQ-WSC-33）

系统 SHALL 仅在 SC 作者链统一分配 split run 身份，并在 provider 启动前耐久绑定 chain／round 与真实 split handle。每轮机械失败的全部 `blocking_reasons`、raw 输出来源、失败 source 摘要和教学 prompt 状态 SHALL 在 claim／下一次启动或终局返回前持久化并回读。证据缺项 SHALL 显式列出轮次与原因；不可验证来源 MUST NOT 驱动教学。该证据 SHALL NOT 改变既有候选尝试身份、typed 资格或启动预算。

#### Scenario: SC 原轮和教学轮分别绑定真实 split 输出

- **WHEN** LC 或非 LC 的 SC 作者原轮及唯一教学轮各产生一次完整交付
- **THEN** 两轮 MUST 各自绑定独立真实 `role_run_seq`／`run_ref`，raw 引用 MUST 回读该轮 `structured_output.json` 的完整 JSON 字符串；轮次顺序 MUST 与实际调用一致，MUST NOT 用目录最新项或教学 attempt 序号猜配
- **判定测试**：驱动集成测试交替输出两段不同 UTF-8 文本，重建 LifecycleStore 后按两个 round 读取并逐字比较，断言两 handle 不同

#### Scenario: 非 SC legacy 不新增 split 身份

- **WHEN** 非 SC、非 Logical 的 legacy 逐段作者路径原本不会分配 split run 身份
- **THEN** 本 change MUST NOT 为该路径创建 split run 身份、原始输出文件或 SC 证据日志，其输入、输出与启动次数 MUST 与既有 fixture 逐字／逐项一致；已有 Logical legacy 审计行为 MUST 保持不变
- **判定测试**：legacy 回归 fixture 比较前后 run 目录清单、provider 输入与启动计数，并以已有 Logical fixture 断言原审计记录仍存在

#### Scenario: 历轮完整阻断原因不被摘要替代

- **WHEN** 第一轮 compile 失败含多于四条修复指令，教学轮以 IR 或结构门失败终止
- **THEN** 证据与终局组包输入 MUST 保留两轮全部原始顺序的 `blocking_reasons` 和未截断修复指令，结构门 MUST 保留该次 typed 失败集合全部原因，MUST NOT 仅保存第一／最后摘要或固定结构失败句
- **判定测试**：两轮故障矩阵分别覆盖 compile→IR、compile→structure，读取日志数组与原完整输入 deep-equal，断言第五条指令和所有结构成员在场

#### Scenario: 轮次事实先落盘后允许教学

- **WHEN** 某轮存在机械失败且系统准备 claim 教学机会
- **THEN** 对应 FailureObserved MUST 已写入并回读通过；日志写入或回读失败时 MUST 显式报告 `sc_failure_evidence_persist_failed`、拒绝新增 provider 启动，并保留原失败原因
- **判定测试**：在证据写入／回读点注入故障，断言调用时序、claim 未发生、额外启动计数为零以及错误含原失败与新故障码

#### Scenario: split 身份或 raw 缺失必须指出哪一轮

- **WHEN** compile 或 IR 失败轮没有绑定 split 身份、raw 文件不存在、读取失败或内容不是 JSON 字符串
- **THEN** 系统 MUST 显式记录该 round 的 `missing`、具体原因和已知 locator，MUST 返回 `sc_failure_source_unavailable` 并零新增启动，MUST NOT 跳过 source 注入或改取另一轮输出
- **判定测试**：四类缺项参数化 store／驱动测试，逐个断言 missing.round_id、缺失码、零新增输入及其它轮文件未被读取

#### Scenario: 日志重放幂等且坏行不可跳过

- **WHEN** 同一个 round 事件重放，或日志含同 entry_key 异文、损坏行、断号或跨 scope 内容
- **THEN** 同键同内容 MUST 不增行；其余情况 MUST 显式拒绝并标记证据不完整，MUST NOT 静默跳过坏行后宣称历史完整
- **判定测试**：存储单测比较重放前后文件 bytes；分别篡改键／JSON／序号／scope 后严格读取断言稳定错误及 incomplete

#### Scenario: 新证据不改变原候选流水身份

- **WHEN** 同一失败同时写入新证据和既有 `candidate-attempts.jsonl`
- **THEN** 原 `attempt_key`、`failure_id`、稳定码与原 A1 审计内容 MUST 保持不变，MUST NOT 为同一失败追加第二条候选尝试；旧流水失败仍按原非阻断诊断处理，新证据写失败则按本 requirement 拒绝教学
- **判定测试**：固定旧 fixture 的 key／id／JSON 内容快照并计数；分别注入两个写点故障，断言不同处理与没有重复候选行

### Requirement: SC 终局失败修复包及严格持久化发布（REQ-WSC-34）

SC 作者链因 compile、IR 预检或结构门失败终止，或已有机械失败后教学被中断时，系统 SHALL 以版本化 `ArtifactPayload::ScFailureRepairPackage` 保存修复包。包 SHALL 携带历轮全部阻断原因、最后物理轮 raw 输出引用、教学 prompt 正文引用与 digest、最后机械失败 source 的 SHA-256 与 UTF-8 字节数；缺项 SHALL 显式标为 incomplete。A4 SHALL 使用「写→回读校验→登记引用→回执写读→广播」屏障，MUST NOT 以 `update_artifact` 返回 ArtifactRef 冒充耐久成功。未发布耐久条目 MUST NOT 被公开视图作为已发布工件；store raw表保留真实pending且A4版本恒is_current=false，不替换普通当前工件。

#### Scenario: 三类终局失败均生成完整包

- **WHEN** 三类机械失败之一首次不获教学或唯一教学轮仍失败，且所有证据与存储均可用
- **THEN** 系统 MUST 保存含版本 1 schema、正确 scope／chain／round 顺序、历轮全部 `blocking_reasons`、末轮 raw 引用、全部教学 prompt ref／digest／bytes、失败 source ref／sha256／bytes 的包；未生成 prompt 时 MUST 显式记录未生成原因，MUST NOT 伪造引用
- **判定测试**：六组参数化终局驱动测试（3 类×首次拒绝／重驱仍失败），对磁盘 payload 与预期全字段比较并逐个解析引用

#### Scenario: 包写入成功必须逐字回读校验后发布

- **WHEN** A4 完成包写入并宣称发布成功
- **THEN** 修复包文件 MUST 实际存在，磁盘读取的版本列表 bytes MUST 与拟写入序列化 bytes 逐字一致，反序列化后成员 MUST 齐备；回读通过后才允许登记 ArtifactRef，引用回读与发布回执写读通过后才允许成功 ArtifactUpdate 广播
- **判定测试**：带真实存储／事件边界spy的屏障测试，以独立手写小型JSON fixture提供expected bytes（不调用生产serializer生成expected），独立测同输入真实写后确定性，反序列化核全字段，并断言写→读→引用写读→回执写读→广播顺序

#### Scenario: 包写入或回读校验失败不得谎报产出

- **WHEN** 修复包写入失败，或回读不存在、字节不一致、反序列化失败、成员或摘要校验不通过
- **THEN** 终局错误 MUST 同时暴露原失败与 `sc_failure_package_persist_failed` 及具体步骤，MUST NOT 登记该包引用、发成功 ArtifactUpdate 或呈现「已产出修复包」；重建、重连与REST读取 MUST NOT 将无有效发布回执的文件当作已发布包，会话 MUST NOT 被改成候选成功
- **判定测试**：逐点故障注入断言 WS Error／失败节点内容、引用缺席、成功事件计数为零及候选 refs 不变

#### Scenario: 文件已核验但引用登记失败必须区分发布不完整

- **WHEN** artifact 文件写入与回读已成功，而 node ArtifactRef 写入或回读失败
- **THEN** 系统 MUST 显式标记 `publication_incomplete` 并报告持久化失败，MUST NOT 发成功广播或声称发布全部完成；读端 MUST 不将待发布版本显示为已产出包，同 package 重放 MUST 只补齐同一版本而不新增版本或启动 provider
- **判定测试**：引用／回执写读点注入故障，夹stale引擎普通update、mark_latest_confirmed和旧表save/CAS回写，再解除故障；断言pending仍保原version/created_at、新普通分配同锁max(raw)+1无碰撞、目标外普通元数据及A4元数据未改、公开视图仍过滤pending；重放同版本且provider启动数不变
- **补充判定**：集中分配/save的锁、raw读或保存失败必须在普通update修改engine.artifact/versions前返回错误，且无ref/成功ArtifactUpdate；唯一void作者完成入口发既有Error并失败返回，不得继续checkpoint/AuthorConfirm。旧普通条目的合法删除/回滚不复活，保留范围仅A4。

#### Scenario: 重连可回读同一修复包且旧工件仍可解析

- **WHEN** 修复包已严格发布后服务重建、WS 重连或用户读取历史工件版本
- **THEN** artifact version、session_state 与历史版本响应 MUST 提供同 package_id 的完整载荷，客户端 MUST 识别独立键 `sc_failure_repair_package`、显示只读摘要及完整 JSON，MUST NOT 误入 markdown 分支崩溃；旧变体解析行为 MUST 不变
- **判定测试**：Rust untagged／flatten往返和raw重建读取，加live／WS session_state／REST三入口契约及JSON面板断言；raw保pending但最终公开视图无回执不可见，坏回执/digest显式错误，receipt成功后同package_id恢复且普通current不变

#### Scenario: 教学中断不拿前一轮冒充最后一轮

- **WHEN** 已有机械失败后教学 provider 实际已启动但没有交付完整输出，或启动被明确拒绝
- **THEN** 包 MUST 保留先前失败历史；实际启动无交付时末物理轮 raw MUST missing；明确未启动时拟调用轮 MUST 标 not_started 且不计物理轮，MUST NOT 冒充实际启动。最后失败 source MUST 标真实 source_round_id，MUST NOT 偷换 raw 或补跑；实际启动 Unknown MUST 维持既有等待／对账而不补造终局
- **判定测试**：序列 provider 首轮失败、次轮分别无交付／拒绝启动／Unknown，断言轮次实际状态、raw缺项、source_round_id、等待态与真实启动证据

#### Scenario: 缺项包必须诚实标记不完整

- **WHEN** raw、prompt 或历史证据存在已知缺失，但工件存储仍可用
- **THEN** 系统 MUST 保存可取得的真实成员、逐项 missing 列表与 `completeness=incomplete`，MUST NOT 以空串、全零摘要、另一候选 ref 或 complete 标志填补缺口
- **判定测试**：构造单缺项及日志损坏的组包单测，逐项断言 missing 与保留成员，不允许虚构 ref／digest

#### Scenario: 非终局路径不产生空修复包

- **WHEN** SC 作者交付成功，或结果为 Routed、admission waiting、健康 Superseded，或取消发生前没有观察到机械失败
- **THEN** 本 change MUST NOT 新建终局修复包或更改这些路径的既有相位／恢复动作，MUST NOT 自动打开人工门或批准候选
- **判定测试**：五类既有出口驱动回归，断言新增包数为零并比较相位、事件与 provider 调用计数

### Requirement: 教学 prompt 自含完整失败 source 与硬预算（REQ-WSC-35）

三类 SC 教学重驱 prompt SHALL 携带经受控引用装载及摘要核验的上一轮完整失败 source，以 `[previous_source]` 的 markdown 围栏块逐字保留，并绑定真实 `failure_source_ref`。最终发送字符串 SHALL 受单源 `WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES = 262_144` UTF-8 字节硬闸约束；超过即显式拒绝，MUST NOT 截断、摘要替代或省略 source。来源、完整性或预算不可证明时 MUST 零新增启动，不改变原 typed 资格与教学次数上限。

#### Scenario: compile 与 IR 从绑定 raw 恢复同一完整失败源

- **WHEN** compile 或 IR 预检失败轮没有 source revision，但有可读的绑定 raw JSON 字符串
- **THEN** 系统 MUST 从该轮 split 输出回读全文，经既有确定性归一化／去 preamble 取得与实际失败 compile_source 相同的完整 source，核验 SHA-256／bytes 后全部注入，MUST NOT 直接注 raw 或读上一成功候选
- **判定测试**：compile／IR 两组含中文标题、EARS 空白及 provider preamble 的来源测试，逐字比较实际失败 source 与围栏正文，分别验证 raw 和 source 的不同摘要

#### Scenario: 结构门从失败自身 revision 回读而非 last-valid

- **WHEN** 结构门已 seal 的 failure.source_revision_ref 指向失败候选，session 的 last-valid ref 指向另一候选
- **THEN** 教学 MUST 用失败自身 ref 调 get_source_revision 并验证 scope／hash，完整注入该失败 source；MUST NOT 使用 session last-valid source 或在 source 已 move 后猜测原文
- **判定测试**：双 revision 夹具的结构拒收测试，断言装载 ref、prompt 正文与失败 revision 一致而非 last-valid

#### Scenario: source 含围栏与首尾空白仍逐字完整

- **WHEN** 失败 source 含中文、多行代码围栏、连续反引号和首尾空白
- **THEN** `[previous_source]` 块 MUST 使用长于 source 内最长反引号串的完整围栏，围栏内原文字节 MUST 不被 trim、转义或截断；包装分隔换行 MUST 不计入 source_sha256／source_bytes
- **判定测试**：source block 单测对多种围栏长度提取正文并比较 bytes，独立计算 SHA-256 与长度

#### Scenario: failure_source_ref 有真实消费且历史缺项零启动

- **WHEN** 新教学请求提供 split locator 或 canonical revision ref，或历史活动请求的 failure_source_ref 为 None
- **THEN** 新请求 MUST 在 claim／启动前装载对应内容并核验该轮 scope 与 failure_source_sha256，MUST NOT 只填写无人读取的字段；历史 None MUST 可读取但显式报 `sc_failure_source_unavailable`，MUST NOT 自动补造引用或启动
- **判定测试**：来源装载 spy 验证两个 ref 分支有读取；历史 None fixture 反序列化成功、错误可见且零额外输入

#### Scenario: 窗口绑定真正可读的教学正文引用

- **WHEN** 新教学窗口建立时已完成不可变 prompt 正文写入及回读
- **THEN** 窗口请求的 prompt_ref MUST 是该正文可读 locator，prompt_digest MUST 等于该正文 UTF-8 SHA-256；构造器名单列 builder_id，MUST NOT 用它冒充 prompt_ref；同窗口重放换正文或 locator MUST 按既有冻结输入规则拒绝
- **判定测试**：窗口 fixture 读取 prompt_ref 正文核对摘要，并分别篡改 ref／正文断言冻结校验或完整性校验失败、零额外启动

#### Scenario: 跨轮引用或摘要篡改被显式拒绝

- **WHEN** source ref 指向另一轮／scope，或 raw／revision 内容被改动导致失败 source hash 或 bytes 不符
- **THEN** 系统 MUST 显式报告 `sc_failure_evidence_invalid` 并拒绝教学，缺项／完整性状态 MUST 进入终局包，MUST NOT 静默使用内存副本或其它输出继续
- **判定测试**：跨轮／跨session／跨plan及单字节篡改的参数化 store／驱动测试，断言稳定码、missing／invalid 明细和零启动

#### Scenario: 最终 prompt 恰在硬预算边界可以继续

- **WHEN** 包含 source、围栏、typed 事实、合法 ID、IR 尾段及输出指令的最终教学字符串为 262,144 UTF-8 字节，且原授权与教学资格均合法
- **THEN** 预算检查 MUST 通过并按既有 claim／启动链最多重驱一次，MUST NOT 把字节数误算为 Unicode 字符数或只算 source 大小
- **判定测试**：含中文的精确边界 fixture 独立计算最终 bytes=262144，断言 builder 成功和原计数上限

#### Scenario: 最终 prompt 超一字节显式拒绝并留存全文引用

- **WHEN** 最终教学 prompt 为 262,145 UTF-8 字节
- **THEN** 系统 MUST 返回 `sc_teaching_prompt_budget_exceeded`、所需／允许字节与失败轮次，存档完整 prompt ref／digest／bytes 并标 rejected_budget；MUST NOT 发送该 prompt、claim 新 attempt、增加 provider 启动或截断 source
- **判定测试**：超一字节构造测试与驱动 spy 检查错误字段、审计文件全文及 hash，断言 claim／发送／额外启动均为零

#### Scenario: IR 追加尾段计入最终硬闸

- **WHEN** 教学 base 加完整 source 未超预算，但追加 IR 修复指令和登记需求 ID 后超出 262,144 字节
- **THEN** 系统 MUST 按最终字符串拒绝并记录实际总字节，MUST NOT 因内部模板已检查而绕过最终闸
- **判定测试**：构造 base≤max 且 final>max 的 IR facade 单测，断言错误 required_bytes 等于 final.len 而非 base.len

#### Scenario: prompt 审计写失败不冒充已持有引用

- **WHEN** 完整 prompt 构造成功但不可变正文写入或回读失败
- **THEN** 系统 MUST 显式报告 `sc_teaching_prompt_persist_failed`、将 prompt 证据标为 missing，并零新增启动，MUST NOT 将 builder 名、事件 ID 或计算出的 digest 当作可读正文 ref
- **判定测试**：prompt store 写读故障注入，断言 ref 缺席、missing 原因、错误字段和 provider 输入计数

#### Scenario: A1 事实预算与 typed 资格不被 source 注入改变

- **WHEN** parse 事实块超 32,000 字节而最终完整教学 prompt 仍在 262,144 字节内，或失败本身不满足既有 typed 资格
- **THEN** 前者 MUST 继续按 REQ-WSC-32 整条省略且显式记账并保留完整 source；后者 MUST 仍不获教学，MUST NOT 因 source 可读取得资格；两者均 MUST NOT 增加既有物理调用上限或消耗 A/B 池
- **判定测试**：沿用 A1 块预算与 typed fault 矩阵，比较省略计数、完整 source、资格结果及 A/B／教学 ledger 计数
