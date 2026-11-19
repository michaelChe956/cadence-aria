## MODIFIED Requirements

### Requirement: 跨仓只读证据检索（REQ-COD-05）

系统 SHALL 使 Coder 可经受控检索接口获取其他已覆盖成员仓的只读证据（改 A 的接口时查 B 的调用点），证据带 ACL/snapshot/token 预算/审计；Coder 不持有聚合根；跨仓调用点检索为启发式符号/字符串近似（CodeGraph 非 Java 语义级），非精确调用图；「无法写入其他仓」为 best-effort 表述，不宣称 OS 级强制。

在既有鉴权、ACL 与快照钉住有效的前提下，若钉住的验收世代确认目标成员的主 checkout 已被判定为「确认无源码而被跳过」，系统 SHALL **不执行 CodeGraph 查询**（不建查询、不探测库目录、不调用 CLI），并返回 HTTP 200 与**显式的未检索结果**：回包含稳定的未检索码与同源中文说明，纯文本字段携带同一说明，`truncated` 为假，配额照常核算，审计记零命中与可读的未检索原因。跳过 MUST NOT 被表达为「查询无命中」，MUST NOT 被表达为 ACL 放行。

未检索事实在回包中 SHALL 是结构化对象 `not_searched: {code, message_zh}`（`code` 为已登记的告警码 `aggregate_index_member_source_less`——同一事实同一码，MUST NOT 另造平行未检索码，MUST NOT 复用 `evidence_index_unavailable` 那种「索引不可用」的码）；`message_zh` 非空且与卡片/成员行同源。跳过分叉 SHALL 在凭据、ACL、输入校验与钉住记录校验**全部通过之后**才判定：未通过既有校验的请求 MUST NOT 因目标恰好是跳过成员而返回 200 或跳过这些检查。

审计 SHALL 是**纯加法**：既有审计字段与判定口径（命中计数、快照引用、配额、越界与凭据失败分类）保持不变；本次只为「未检索」增加一个**可选**的原因字段，其唯一职责是解释「为什么没有检索」，MUST NOT 改变任何既有字段的含义或把原「查询无命中」记录改写成「未检索」。

Coder SHALL 使用已授权的目标 worktree 直接处理其配置/文档；MUST NOT 因跳过扩大写入范围或授予聚合根路径，MUST NOT 要求把配置/文档文件纳入索引。

目标仓正常检索时，结果必须位于既有 ACL 与钉住世代已覆盖成员的交集，MUST NOT 返回被跳过成员的索引证据；不得扩大查询授权。钉住记录缺失或损坏、目标越界、凭据不合法等既有失败判定保持不变。

#### Scenario: coding 中检索跨仓证据

- **WHEN** coding 过程中 Coder 需要了解其他仓的调用关系，且目标仓未被该验收世代跳过
- **THEN** 通过受控检索接口获取只读证据注入上下文；证据记录来源 snapshot 与预算；Coder 的写入配置目标仅为目标 worktree

#### Scenario: 配置目标不进入 CodeGraph

- **WHEN** 合法 attempt 请求跨仓证据且目标主 checkout 在钉住世代被确认为无源码跳过
- **THEN** SHALL 返回 HTTP 200 与显式未检索事实（稳定码 + 同源中文说明、零命中），MUST NOT 调用 CodeGraph；Coder 可直接在目标工作区处理配置/文档

#### Scenario: 未检索不是无命中也不是放行

- **WHEN** 请求因目标被跳过而未检索
- **THEN** 回包 SHALL 显式区分「未检索」与「检索过但无命中」，且既有 ACL 判定 MUST NOT 因跳过而被绕过或放宽

#### Scenario: 跳过不改变权限或扩大证据范围

- **WHEN** 请求凭据不合法、目标越界、应钉住的记录不可读，或普通目标查询结果含跳过成员路径
- **THEN** 既有错误判定保持；搜索命中不得包含钉住世代的跳过成员、界外成员或目标自身

#### Scenario: 有效钉住但库目录缺失仍是未检索

- **WHEN** 凭据、ACL、输入校验与钉住记录都有效，目标在钉住世代被跳过，且聚合根上的 CodeGraph 库目录不在
- **THEN** SHALL 仍返回 HTTP 200 与 `not_searched`（零命中、配额照常核算、审计记未检索原因）；MUST NOT 改判为 `evidence_index_unavailable` 那类「索引不可用」失败

#### Scenario: 坏钉住与非法输入仍先拒绝

- **WHEN** 凭据不合法、目标越界或钉住记录缺失/损坏
- **THEN** SHALL 按既有规则拒绝；MUST NOT 因为目标恰好属于跳过成员而返回 200、跳过校验或省去审计

#### Scenario: 普通目标的交集与跳过目标的未检索各自保留

- **WHEN** 同一 attempt 先对普通目标取证、再对跳过成员取证
- **THEN** 普通目标 SHALL 返回 ACL 与钉住世代已覆盖成员的交集证据（不含跳过成员路径）；跳过目标 SHALL 返回未检索对象；两者的审计条目 SHALL 各自保留自己的命中/未检索事实，MUST NOT 互相覆盖
