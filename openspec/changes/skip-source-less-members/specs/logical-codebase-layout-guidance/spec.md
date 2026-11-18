## MODIFIED Requirements

### Requirement: 布局规则与稳定错误码（REQ-LAYOUT-01）

系统 SHALL 维护单一布局规则表，覆盖六条规则：(1) 聚合根 MUST NOT 是 git 仓；(2) 每个成员仓 MUST 是聚合根的直接子目录；(3) 成员路径 MUST NOT 经符号链接绕出根；(4) 同一聚合根 MUST NOT 被两个逻辑代码库占用（含跨项目）；(5) 成员仓 MUST NOT 是嵌套 worktree；(6) 聚合索引至少需要一个已覆盖成员；索引未覆盖且经 Aria 磁盘交叉验收确认无源码的成员 MAY 跳过并告警。规则六的判定由聚合索引契约拥有，本能力 MUST NOT 修改登记准入或另建判定。

#### Scenario: 聚合根是 git 仓

- **WHEN** 用户把某个 git 仓目录当作 LC 根登记
- **THEN** 系统 SHALL 返回 `aggregate_root_is_git`，动作说明为「另建一个空的普通目录当 LC 根（不要在某个 git 仓里）」

#### Scenario: 成员仓不在直接子目录层

- **WHEN** 某成员仓不在 LC 根的直接下一层（层级不对或跑到根外）
- **THEN** 系统 SHALL 返回 `member_path_outside_root` 或 `aggregate_index_layout_unsupported`，动作说明为「`LC根/<成员名>/` 一层，不要再套一层」

#### Scenario: 成员路径是符号链接逃逸

- **WHEN** 成员路径经符号链接指向 LC 根之外
- **THEN** 系统 SHALL 返回 `member_symlink_escape`，动作说明为「用真实路径，不要 symlink 到别处」

#### Scenario: 同一根被两个 LC 占用

- **WHEN** 某聚合根已被一个存活或软删（墓碑）的 LC 占用，另一个 LC 尝试以它为根
- **THEN** 系统 SHALL 返回 `aggregate_root_overlap` 并点名被占用根与占用方，动作说明为「一个根只对应一个 LC；要多个就换根目录」

#### Scenario: 成员是嵌套 worktree

- **WHEN** 某成员仓是嵌套 worktree
- **THEN** 系统 SHALL 返回 `nested_worktree`，动作说明为「成员要用正常检出，不要塞嵌套 worktree」

#### Scenario: 混合 LC 跳过无源码成员

- **WHEN** 部分成员被覆盖，其余成员已由 Aria 确认无源码且其余验收通过
- **THEN** 系统 SHALL 以 `aggregate_index_member_source_less` 告警点名未纳入索引的成员，不阻断激活或把告警说成布局失败

#### Scenario: 未覆盖且确有源码或不能确认

- **WHEN** 索引未覆盖成员，但成员普通路径有源码、源码仅在 `.worktrees/`，或磁盘判定未完成
- **THEN** 系统 SHALL 按 REQ-IND-01 分别采用 `aggregate_index_member_coverage_failed`、`aggregate_index_member_source_excluded` 或 `aggregate_index_source_scan_incomplete`；MUST NOT 把这些事实标为无源码

#### Scenario: 全部成员无源码仍失败

- **WHEN** 无已覆盖成员且全部成员经磁盘交叉验收确认无源码
- **THEN** 系统 SHALL 返回 `aggregate_index_all_members_source_less`，提示加入含源码的成员或在成员普通路径下放置实际源码后重建

#### Scenario: 每个布局错误码都有动作

- **WHEN** 任一布局错误码或第六条的补充失败码呈现给用户
- **THEN** 该码 SHALL 在规则表中有映射，且失败面 MUST 带上同行动作说明；MUST NOT 出现只有错误码的布局失败

### Requirement: 入口目录树示意（REQ-LAYOUT-02）

多仓创建/登记入口与聚合初始化卡片 SHALL 在显著位置给出目录树示意，说明 LC 根是**普通空目录**（不是 git 仓）、成员仓是**直接子目录**；同时 SHALL 说明可索引范围：源码应放成员仓根或普通子目录，`.worktrees/` 下的源码不纳入索引。入口 SHALL 明示确认无源码的成员在其余验收通过时可以跳过，但没有任何已覆盖成员的 LC 不能激活聚合索引。

#### Scenario: 入口展示目录树

- **WHEN** 用户打开多仓创建/登记入口或聚合初始化卡片
- **THEN** 界面 SHALL 呈现形如
  ```text
  my-lc-root/            ← LC 根目录：普通空目录，不是 git 仓
    backend/             ← 成员仓（真实检出，直接子目录）
    frontend/
  ```
  的示意，并消费 REQ-BOOT-10 的同源范围说明，说明普通路径与 `.worktrees/` 的区别、无源码成员可跳过及至少一个成员须被索引覆盖的边界

### Requirement: 失败面三段式渲染（问题 / 原因 / 怎么做）（REQ-LAYOUT-04）

失败面 SHALL 按错误码呈现「问题 / 原因 / 怎么做」三段，中文主文案在前、机器码降级到次要行。文案 MUST **取自** `non-interrupt-repository-bootstrap` 的 `REQ-BOOT-10` 登记的规则表，MUST NOT 另抄一份。三段分别由必填非空的 `title_zh: String`、`reason_zh: String`、`action_zh: String` 承载，并一并消费同源 `triage` 与 `machine_line`；布局引导侧只渲染成品，MUST NOT 自补问题、原因或动作的中文模板。七个既有布局码及第六条的补充失败码即使无 `action` 字段，仍 MUST 呈现完整三段；缺 `reason_zh` 或缺 copy MUST NOT 作为正常缺省降级验收通过。非阻断的无源码告警 SHALL 由聚合索引卡片独立呈现一行、点名成员，不得作为三段失败渲染或改变状态和重建动作。

#### Scenario: 失败面给出具体动作

- **WHEN** 成员仓 `frontend` 不在 LC 根目录下
- **THEN** 失败面 SHALL 给出：问题（`frontend` 不在 LC 根目录下）、原因（成员仓必须是 LC 根目录的直接子目录，一层）、怎么做（把 `frontend` 移到 `<LC根>/frontend/`，不要多套一层目录，然后重试）

#### Scenario: 不把漏收责怪为没有源码

- **WHEN** 当前验收证实 `frontend` 普通路径有源码但未被索引
- **THEN** 三段 SHALL 说明「索引漏了这个仓的源码」、点名该仓并指引检查忽略规则与重建；MUST NOT 说用户没有放源码或断言未证实的原因

#### Scenario: 告警与失败保持独立

- **WHEN** 可读验收世代中有确认无源码的跳过成员
- **THEN** 聚合索引卡片 SHALL 使用结构化告警的一行同源成品；降级时如同时有失败，仍显示既有三段成品，并把告警标为上次可用索引的事实

### Requirement: 布局规则表是唯一映射来源（REQ-LAYOUT-05）

六条布局规则及稳定错误码到「用户该怎么摆」的映射 MUST 登记在 `non-interrupt-repository-bootstrap` 的 `REQ-BOOT-08` / `REQ-BOOT-10` / `REQ-BOOT-11` / `REQ-BOOT-12` 共同定义的唯一文案来源中；本能力 MUST NOT 自建第二份规则表或文案表。每条规则 MUST 绑定稳定错误码；七个既有对外码与第六条的补充失败码各自引用该来源登记的非空 `title_zh`（问题）、`reason_zh`（原因）、`action_zh`（怎么做）以及 `triage`、`machine_line` 五字段合同（见 `REQ-BOOT-11`）；其中 `reason_zh` MUST 为必填 `String` 而非 `Option`。既有内部码 SHALL 反查到对应对外稳定码，任一布局错误码 MUST 能映射到六条规则中的一行；系统 MUST NOT 呈现只有错误码、没有动作说明，或缺少原因段的布局失败。第六条的范围说明和跳过告警也 SHALL 取自该来源，但不使用失败五字段。

#### Scenario: 文案单一来源

- **WHEN** 布局动作文案需要修改
- **THEN** 该修改 SHALL 落在 `REQ-BOOT-10` 登记的同一来源中；两条消费链 SHALL 直接获得修改后的 `title_zh`/`reason_zh`/`action_zh`，MUST NOT 在布局引导侧维护第二份同义文案，MUST NOT 因原因字段缺失而静默省略原因段
