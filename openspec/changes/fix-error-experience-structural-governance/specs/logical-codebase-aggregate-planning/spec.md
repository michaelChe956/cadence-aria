# Spec Delta

## ADDED Requirements

### Requirement: candidate 与 compile 同源有效图（REQ-PLN-09）

系统 SHALL 在 candidate/reviewer 前置门及 Final Compile 使用同源结构规则与上下文digest：有效图边为 canonical Inputs 的 provider→consumer 与已确认change_order的前置仓全部WI→后置仓全部WI之并集。声明Dependencies SHALL 单独保留并按原规则校验一致性，不将未参与执行的声明边冒称实际边。同端点多来源 SHALL 全部保留；自依赖、未知引用、成员归属、Plan Intent与物理target同构分别验证。纯结构检查 SHALL 无selection写回、WI/transaction创建或provider调用，外层只加载一次授权/成员事实；logical→physical ID映射不得改变规则。

#### Scenario: R1 两种边相撞
- **WHEN** gateway序3/frontend序4，WI-005归gateway且真实Inputs要求WI-004(frontend)
- **THEN** candidate门确定拒收，记录WI-004→WI-005 canonical_input与WI-005→WI-004 change_order两种实际来源，坏候选不评审/确认；机器指令保AC/真实Inputs/仓序，交原授权author重组，Final Compile同规则复验

#### Scenario: 同仓与单仓原语义
- **WHEN** 两WI同target或单仓Design允许空change_order
- **THEN** 不添加同仓自动边，单仓编译行为不变；多仓缺顺序仍原blocker，不临时取消自动边逃过检测

#### Scenario: 成员与物理身份
- **WHEN** IR物理target不在授权成员、Plan Intent归属不符或成员事实失效/不可读
- **THEN** 拒绝候选并保具体typed结构或operational原因，不把聚合native target当业务target、不选首仓、不在reviewer纯门重复selection写回

### Requirement: 来源 witness 与语义保护（REQ-PLN-10）

有效图阻断 SHALL 携实际邻接边可验证的确定性有向cycle witness及每跳来源、contract/字段定位与仓序索引；未排序节点集合不得直接渲染成有向环。机器建议 SHALL 使用完整已读事实，保持真实供需/AC/能力/授权仓序与写面；唯一性只对可完整枚举局部域给证据，拓扑/删边空间不能因只生成一个建议就称唯一。开放语义等价由原reviewer判断；无环不是业务正确证明。

#### Scenario: witness 每跳真实
- **WHEN** Kahn返回含不在环上的阻断残余节点
- **THEN** 系统从实际邻接边提取稳定有向witness、逐跳可核验来源，不把sort/dedup节点串画箭头，不穷举全部最小删边集

#### Scenario: 保留真实供需的多解
- **WHEN** 改归属、拆分验证与历史恢复存在多个保持业务要求的候选
- **THEN** author收到各具体约束/证据，不要求用户选择仓UUID；删/替依赖仅有同供需已满足证据才成为安全候选，否则只给保护约束下靶向重组指令
