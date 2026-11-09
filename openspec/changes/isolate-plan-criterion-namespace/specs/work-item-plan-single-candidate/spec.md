## ADDED Requirements

### Requirement: plan 域 criterion 命名域与双前缀只读兼容（REQ-WSC-30）

plan 编译器的 criterion 标识符 SHALL 以 `WIC-` 或 `AC-` 双前缀只读兼容解析，两者共享同一次全 plan 唯一性检查；标识符词法 SHALL 保持「固定前缀 + 非空 ASCII 数字」，MUST NOT 放宽为任意子串、大小写变体或 `WAC-` 之类混合前缀。spec 域 `REQ-*` / `NFR-*` / `AC-*` SHALL 只服务 Traceability 与 `requirement_refs`；plan 域 ID SHALL 只服务本 Work Item 的 criterion、`done_when_refs` 与 `reviewer_check_refs`。平台 MUST NOT 为适配新前缀而改写存量 source 字节。

#### Scenario: 双前缀均可解析且共享唯一性

- **WHEN** 一份计划同时出现 `criterion_id: WIC-001` 与 `criterion_id: AC-002`
- **THEN** 两者 SHALL 均通过标识符校验并进入同一 criterion 集合，唯一性检查 SHALL 对两个前缀合并执行一次；同一前缀重复 SHALL 仍被拒

#### Scenario: 非法前缀与非法后缀被拒

- **WHEN** criterion 标识符缺少合法前缀、后缀为空或后缀含非 ASCII 数字字符
- **THEN** 编译器 SHALL 以既有 `invalid_work_item_id` 诊断拒收该候选，MUST NOT 推断或修补前缀

#### Scenario: 标识符词法不放宽

- **WHEN** criterion 标识符写作 `WAC-001`、`wic-001`、`WIC-` 或 `WIC-001a`
- **THEN** 编译器 MUST 拒绝该标识符，MUST NOT 将其接受为任一合法前缀形式

#### Scenario: spec 域与 plan 域职责分界

- **WHEN** `requirement_refs` 引用 spec 域已登记的 `REQ-*` / `NFR-*` / `AC-*`，而 `done_when_refs` / `reviewer_check_refs` 引用本 Work Item 的 plan 域 criterion
- **THEN** 平台 SHALL 按域分别校验：spec 域引用 MUST 落在已登记需求清单内，plan 域引用 MUST 落在本 Work Item 的 criterion 集合内；spec 域 `AC-*` MUST NOT 被当作本 Work Item 的 criterion 接受

#### Scenario: 其他 Work Item 的 criterion 不可被引用

- **WHEN** 某 Work Item 的 `done_when_refs` 引用了同一计划中另一个 Work Item 存在的 `WIC-` / `AC-` criterion
- **THEN** 平台 MUST 以既有成员校验错误拒收该引用，MUST NOT 跨 Work Item 放宽成员范围

#### Scenario: 存量 source 字节不被平台改写

- **WHEN** 一份含存量 `AC-` criterion 的历史计划被重新编译或修订
- **THEN** 平台 SHALL 逐字保留该 source 的既有字节与标识符，MUST NOT 为迁前缀改写 source 或其内容寻址

#### Scenario: 唯一性与引用范围不变

- **WHEN** 双前缀并存后重新检查全 plan criterion 唯一性与本 WI 成员关系
- **THEN** 唯一性判据与成员关系判据 SHALL 与双前缀引入前等价，MUST NOT 因新增前缀扩大合法引用集合

#### Scenario: 双前缀不改变旧 source 的 IR 与内容寻址

- **WHEN** 一份只含存量 `AC-` criterion 的旧 source 在双前缀 parser 上重新编译
- **THEN** 其 lower 结果 IR 与内容寻址 SHALL 与双前缀引入前逐位一致，MUST NOT 因 parser 兼容新前缀而漂移

#### Scenario: 不同 compiler_version 仍按 freshness 拒绝

- **WHEN** 一份以不同 `compiler_version` 产出的候选进入发布 / 复验判定
- **THEN** 平台 SHALL 维持既有 freshness 拒绝语义，MUST NOT 因本轮不 bump compiler version 而放宽版本门

### Requirement: 服务端 lineage 命名策略与旧实体 ID 保持（REQ-WSC-31）

平台 SHALL 以 server-only 命名策略元数据（`WicV1` / `LegacyPreserved`）记录每个 plan lineage 的命名域，并在真实 SC 首次生成准入时以 CAS 冻结该策略。`WicV1` lineage 的新交付 MUST NOT 出现 plan 域 `AC-` criterion；`LegacyPreserved` lineage 的修订 SHALL 保留基线既有 criterion ID，新增 criterion MUST 使用 `WIC-`。未经修复闭包与业务授权，平台 MUST NOT 删除、重编号或替换旧 baseline 的 criterion ID（经授权的删除走修复闭包既有合同），且「仅前缀升级」替换整组身份在任何情况下 MUST NOT 发生。平台 MUST NOT 批量改写存量不可变记录与其 hash，MUST NOT 接受客户端入参绕过该策略。

#### Scenario: 新生 lineage 准入冻结为 WicV1

- **WHEN** 一个此前无 source / IR / 评审 / 发布事实、且存在真实新生事实的 plan 首次通过真实 SC 生成准入
- **THEN** 平台 SHALL 以 CAS 把该 lineage 的命名策略冻结为 `WicV1`，且该 lineage 后续交付的 plan criterion MUST 全部使用 `WIC-`

#### Scenario: 存量 lineage 冻结为 LegacyPreserved 且旧 ID 保持

- **WHEN** 一个已存在 source / IR / 评审 / 发布事实的存量 lineage 首次准入时被判定为存量
- **THEN** 平台 SHALL 把其命名策略冻结为 `LegacyPreserved`，其既有 criterion ID SHALL 逐字保留，MUST NOT 因迁前缀被改写

#### Scenario: 存量修订新增 criterion 必须使用 WIC

- **WHEN** `LegacyPreserved` lineage 产生一份修订交付，其中既有 criterion ID 未变且新增了 criterion
- **THEN** 新增 criterion MUST 使用 `WIC-` 前缀，既有 criterion ID MUST NOT 被重编号、替换或删除

#### Scenario: 未知历史不得伪装成新生

- **WHEN** 某 lineage 的历史事实不足以判定是新生还是存量
- **THEN** 平台 MUST NOT 将其判定为 `WicV1`、MUST NOT 伪装成新生，SHALL 不冻结命名策略并按未知处理（走既有等待或人工面）且披露未知，MUST NOT 以缺省值推断存在新生事实；「已冻结不可改」只约束已判定的 `WicV1` / `LegacyPreserved`

#### Scenario: 不得仅前缀升级替换整组身份

- **WHEN** 某次修订把一组既有 criterion 整体从 `AC-` 改写为同号 `WIC-`
- **THEN** 平台 MUST 拒收该交付，MUST NOT 把前缀替换当作合法迁移

#### Scenario: 旧不可变记录与 hash 不被改写

- **WHEN** 存量 lineage 的 source、IR、机械报告或引用记录被读取或复用
- **THEN** 平台 SHALL 逐字保留其既有内容与内容寻址，MUST NOT 批量执行 `AC-` → `WIC-` 改写，也 MUST NOT 在读取时补写字段冒称旧内容原本属于新命名域

#### Scenario: 客户端不得绕过命名策略

- **WHEN** 客户端在请求中声明 lineage 为存量或请求关闭命名域守卫
- **THEN** 服务端 MUST 忽略该声明并以已冻结的命名策略为准，MUST NOT 依据客户端入参放宽或切换命名域

#### Scenario: 重入与接管不覆盖已冻结策略

- **WHEN** 同一 lineage 被重入创建、接管或重复准入
- **THEN** 已冻结的命名策略 SHALL 保持不变，MUST NOT 因重复准入被改写为另一代际

#### Scenario: WicV1 交付含 plan 域 AC 被拒且教学资格按 typed 清单判定

- **WHEN** 一个 `WicV1` lineage 的交付包含 plan 域 `AC-` criterion，被命名域守卫以独立 typed fault 拒收后寻求教学重驱
- **THEN** 平台 SHALL 以独立 typed 诊断拒收（`AC-` 在双前缀下语法合法，不触发 `invalid_work_item_id`）；其教学重驱资格 SHALL 按 REQ-WSC-09 的 typed 资格清单（由 `mechanicalize-plan-compile-repair` 扩充）判定，MUST NOT 因诊断码前缀或「缺少」措辞自动获得

#### Scenario: 旧 baseline criterion ID 的改变与消失按授权处理

- **WHEN** 一次修订试图删除、重编号或替换旧 baseline 的既有 criterion ID，或以「仅前缀升级」把整组身份改为 `WIC-`
- **THEN** 未经修复闭包与业务授权，平台 MUST NOT 删除、重编号或替换旧 baseline 的 criterion ID，经授权的删除 MUST 走修复闭包既有合同；「仅前缀升级」替换整组身份在任何情况下 MUST NOT 发生
