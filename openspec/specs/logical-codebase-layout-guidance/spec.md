# logical-codebase-layout-guidance Specification

## Purpose
把「多仓目录该怎么摆」变成产品面可执行的事实：定义聚合根与成员仓的布局规则及其稳定错误码，要求每条规则都绑定一句「用户该怎么摆」，并要求多仓入口与失败面按错误码给出动作。本能力只承载**引导与动作文案**；布局规则的判定本身由 `logical-codebase-registration` 的 `REQ-REG-09` 与聚合索引既有契约拥有，本能力 MUST NOT 新增或放宽任何判定。

## Requirements

### Requirement: 布局规则与稳定错误码（REQ-LAYOUT-01）

系统 SHALL 维护单一布局规则表，覆盖六条规则：(1) 聚合根 MUST NOT 是 git 仓；(2) 每个成员仓 MUST 是聚合根的直接子目录；(3) 成员路径 MUST NOT 经符号链接绕出根；(4) 同一聚合根 MUST NOT 被两个逻辑代码库占用（含跨项目）；(5) 成员仓 MUST NOT 是嵌套 worktree；(6) 每个成员仓 MUST 至少含 1 个可索引代码文件。

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

#### Scenario: 成员无可索引代码文件

- **WHEN** 某成员仓只含 `.md`/`.json`/`.html` 之类文件，或代码只在 `.worktrees/` 下
- **THEN** 系统 SHALL 返回 `aggregate_index_member_coverage_failed`，动作说明为「放真源码（`.js`/`.ts`/`.py`/…）；`.html`/`.json`/`.md` 不算；只在 `.worktrees/` 下的代码同样不算」

#### Scenario: 每个布局错误码都有动作

- **WHEN** 任一布局错误码呈现给用户
- **THEN** 该码 SHALL 在规则表中有一行映射，且失败面 MUST 带上该行动作说明；MUST NOT 出现只有错误码的布局失败

### Requirement: 入口目录树示意（REQ-LAYOUT-02）

多仓创建/登记入口与聚合初始化卡片 SHALL 在显著位置给出目录树示意，表达两件事：LC 根是**普通空目录**（不是 git 仓）、成员仓是**直接子目录**。同时 SHALL 说明可索引范围（成员仓代码要放在仓库根或普通子目录；只在 `.worktrees/` 下的代码不可被索引）。

#### Scenario: 入口展示目录树

- **WHEN** 用户打开多仓创建/登记入口或聚合初始化卡片
- **THEN** 界面 SHALL 呈现形如
  ```
  my-lc-root/            ← LC 根目录：普通空目录，不是 git 仓
    backend/             ← 成员仓（真实检出，直接子目录）
    frontend/
  ```
  的示意，并说明「成员仓里的代码要放在仓库根或普通子目录；只在 `.worktrees/` 下的代码不可被索引」

### Requirement: 失败面三段式渲染（问题 / 原因 / 怎么做）（REQ-LAYOUT-04）

失败面 SHALL 按错误码呈现「问题 / 原因 / 怎么做」三段，中文主文案在前、机器码降级到次要行。文案 MUST **取自** `non-interrupt-repository-bootstrap` 的 `REQ-BOOT-10` 登记的规则表，MUST NOT 另抄一份。三段分别由必填非空的 `title_zh: String`、`reason_zh: String`、`action_zh: String` 承载，并一并消费同源 `triage` 与 `machine_line`；布局引导侧只渲染成品，MUST NOT 自补问题、原因或动作的中文模板。七个布局码即使无 `action` 字段，仍 MUST 呈现完整三段；缺reason_zh或缺copy MUST NOT 作为正常缺省降级验收通过。

#### Scenario: 失败面给出具体动作

- **WHEN** 成员仓 `frontend` 不在 LC 根目录下
- **THEN** 失败面 SHALL 给出：问题（`frontend` 不在 LC 根目录下）、原因（成员仓必须是 LC 根目录的直接子目录，一层）、怎么做（把 `frontend` 移到 `<LC根>/frontend/`，不要多套一层目录，然后重试）

### Requirement: 布局规则表是唯一映射来源（REQ-LAYOUT-05）

六条布局规则及稳定错误码到「用户该怎么摆」的映射 MUST 登记在 `non-interrupt-repository-bootstrap` 的 `REQ-BOOT-08` / `REQ-BOOT-10` / `REQ-BOOT-11` / `REQ-BOOT-12` 共同定义的唯一失败面文案来源中；本能力 MUST NOT 自建第二份规则表或文案表。每项 MUST 绑定稳定错误码，并引用该来源登记的非空 `title_zh`（问题）、`reason_zh`（原因）、`action_zh`（怎么做）以及 `triage`、`machine_line` 五字段合同（五字段合同见 `REQ-BOOT-11`）；其中reason_zh MUST 为必填String而非Option。既有内部码 SHALL 反查到对应对外稳定码，任一布局错误码 MUST 能映射到六条规则中的一行；系统 MUST NOT 呈现只有错误码、没有动作说明，或缺少原因段的布局失败。

#### Scenario: 文案单一来源

- **WHEN** 布局动作文案需要修改
- **THEN** 该修改 SHALL 落在 `REQ-BOOT-10` 登记的六规则/七个对外码唯一来源中；两条消费链 SHALL 直接获得修改后的title_zh/reason_zh/action_zh，MUST NOT 在布局引导侧维护第二份同义文案，MUST NOT 因原因字段缺失而静默省略原因段。

### Requirement: 入口的依赖前置提示（REQ-LAYOUT-03）

多仓入口与布局失败面 SHALL 把聚合冷启动的依赖前置列入引导清单：`codegraph_missing` 与 `codegraph_version_mismatch` MUST 触发「需先安装/更新 codegraph」的引导，并给出可执行的安装/更新入口；提示文案 MUST 与 `REQ-BOOT-08` / `REQ-BOOT-10` / `REQ-BOOT-11` / `REQ-BOOT-12` 同源引用。

安装/更新机制本身由块2 的 `provider-cli-installation` 定义（`REQ-PCI-02` 探测与按钮显示规则、`REQ-PCI-05` 异步作业与装完重新探测），本能力 MUST NOT 重新定义它。

#### Scenario: codegraph 缺失给出安装入口

- **WHEN** 入口的依赖探测发现 codegraph 缺失
- **THEN** 界面 SHALL 给出「需先安装 codegraph」的说明与可点击的【安装】入口；MUST NOT 只显示机器码

#### Scenario: codegraph 版本不符给出更新入口

- **WHEN** 依赖探测发现 codegraph 版本与要求不符
- **THEN** 界面 SHALL 给出「需更新 codegraph」的说明与可点击的【更新】入口，并显示当前版本与要求版本

#### Scenario: 安装机制不重复定义

- **WHEN** 用户点击安装/更新
- **THEN** 系统 SHALL 走 `provider-cli-installation` 已定义的安装/更新作业（含镜像源、失败分类与手动命令兜底）；本能力 MUST NOT 自带第二套安装实现或第二套文案
