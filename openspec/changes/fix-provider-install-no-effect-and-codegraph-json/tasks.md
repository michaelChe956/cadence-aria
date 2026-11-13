# Tasks: fix-provider-install-no-effect-and-codegraph-json

> **实施状态（2026-11-13）**：契约已获批并**已实施**；勾选状态按仓库实现证据记录（`[x]` = 已落地，`[ ]` = 待办）。未闭环项集中在第 8 包（后端「不在终端 PATH 上」显式状态字段）、第 11 包（归档同步）与第 12 包（容器 E2E 复验）。
> 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`；本文件只记高层工作包，每个工作包映射到相关 requirement（编号逐字展开，一行一个）。
> 依据：`cadence/plans/2026-11-13_计划文档_修复Provider安装无效果与聚合索引截断_v1.0.md`；owner 决策见 `.openspec.yaml` 的 `x-owner-decisions`。
> 顺序约束：第 2 包必须早于第 5 包（装后登记依赖 overlay 就位）；第 4 包早于第 6 包（装后验证依赖前缀落点可信）；第 8 包依赖第 5 包的后端文案定稿；第 7 包依赖第 4 包与第 5 包（前缀落点与装后验证是更新语义的前提）；第 9 包依赖第 2 包与第 3 包（overlay 与登录 shell PATH 快照）；第 10 包与安装链无依赖。

## 1. 聚合索引专用抓取配额与截断专用码

映射 requirement：

- MODIFIED REQ-IND-01
- MODIFIED REQ-IND-04

- [x] 1.1 文件清单抓取走**有文档上限的专用配额**（64 MiB），与其余 codegraph 命令的既有上限明确区分；「默认上限」必须显式化，不得依赖调用处的隐含默认
- [x] 1.2 截断事实从命令执行层**透传到调用方**（截断标志不再在结果转换处被丢弃）
- [x] 1.3 截断判定必须**先于** JSON 解析：stdout 被截断时以专用机器码 `codegraph_files_output_truncated` 报告，消息含上限字节数与「输出被截断」说明；MUST NOT 报成 `codegraph_files_invalid_json`
- [x] 1.4 未截断的完整载荷 MUST 照常解析出成员清单（配额放大不得改变解析口径）
- [x] 1.5 文件清单调用 SHALL 只下发 `files --json`；MUST NOT 附加未经实测证明能降低载荷的额外旗标（owner 2026-11-13 裁决去掉实测零收益、且会引入「上游 CLI 移除该旗标 ⇒ 文件清单全线失败」风险的旗标）；MUST NOT 运行时靠去旗标重试来掩盖真实失败
- [x] 1.6 失败文案表补齐专用码条目（沿用既有五元组结构），且 MUST 区分「平台输出上限截断」与「CLI 输出协议不合法」
- [x] 1.7 截断属构建失败路径：保留 last-known-good、标记 degraded，MUST NOT 伪造 active，MUST NOT 丢弃失败事实

## 2. 共享 PATH overlay 与两个命令汇聚点合并

映射 requirement：

- MODIFIED REQ-PCI-02
- MODIFIED REQ-PCI-05

- [x] 2.1 在**共享命令层**建立进程级 PATH overlay 注册表：只追加、可去重、可查询、可清空；提供**单一合并函数**把 overlay 与基础 PATH 合成子进程 PATH
- [x] 2.2 **不修改进程真实 env**（平台进程 PATH 保持原样）；MUST NOT 走 `set_var` 路线（edition 2024 下 unsafe + 多线程竞态）
- [x] 2.3 在**两个命令汇聚点**合并 overlay（进程拉起通道 + 同步捕获通道），两个点都**必须显式写 PATH**（含「调用方未提供环境覆盖」那条路径）；MUST NOT 只在其中一条路径上合并
- [x] 2.4 四家 provider 的进程拉起 MUST 因共享命令层的合并而自动可见 overlay（**不改动四家 provider 目录**）
- [x] 2.5 overlay 是进程级共享状态 ⇒ 相关用例持**全局锁**执行并在结束时清空，MUST NOT 污染并行的命令可见性断言用例

## 3. 启动期登记用户工具目录

映射 requirement：

- MODIFIED REQ-PCI-02
- MODIFIED REQ-PCI-05

- [x] 3.1 服务启动时把**实际存在**的用户工具目录登记进 overlay（常见版本管理器 / 包管理器的用户级 bin 目录 + 本仓自有落点：npm 全局前缀 bin、`NPM_PREFIX` bin、Kimi 安装目录 bin）
- [x] 3.2 **只登记、不写盘、不创建目录**：不存在的目录 MUST NOT 被登记，MUST NOT 因此创建目录
- [x] 3.3 启动路径上的前缀探针用**短超时**（不得引入与安装作业同量级的阻塞）
- [x] 3.4 登记时机 MUST 在「监听就绪之后、首次 provider 健康刷新之前」；监听失败 MUST 保持零副作用

实施注记：启动期登记清单覆盖 `~/.local/bin` / `~/.npm-global/bin` / `~/.npmrc` 的 `prefix=` bin / nvm 默认 node bin / mise node installs 等**实际存在**的目录；`~/.kimi-code/bin` 不在启动清单内，由第 5 包的**装后登记**路径覆盖（Kimi 走官方脚本，安装时机才确定目录）。

## 4. 前缀候选收紧与真实可写性判定

映射 requirement：

- MODIFIED REQ-PCI-03

- [x] 4.1 「可写」判定改为**权限位预筛 + 真实写入探测**（唯一命名临时文件：建 → 删）；MUST NOT 只看权限位
- [x] 4.2 真实写入探测 MUST 有**可注入 seam**，使单测能确定性模拟「权限位说可写但实际写失败」，MUST NOT 依赖运行身份
- [x] 4.3 该判定有写副作用 ⇒ MUST 只用于「候选前缀探测」这类一次性场景，MUST NOT 进入热路径
- [x] 4.4 前缀候选链收紧为三级：PATH 上生效前缀 → npm 全局前缀（且其 bin 目录在 PATH 上）→ 用户级兜底前缀
- [x] 4.5 「bin 目录在 PATH 上」的判定 MUST 按**目录成员**语义（该目录出现在 PATH 中），MUST NOT 用「命令文件是否存在」判定（CLI 未装时该判据必然落空）
- [x] 4.6 系统级不可写前缀 MUST NOT 被选中；平台 MUST NOT 以提权方式写入或 sudo 重试
- [x] 4.7 既有的两条可写性断言（跳过不可写候选 / 回退最近存在祖先）MUST 保持通过，MUST NOT 削弱其强度

## 5. 装后登记 overlay、绝对路径兜底探测与 Kimi 目录识别

映射 requirement：

- MODIFIED REQ-PCI-05
- MODIFIED REQ-PCI-02

- [x] 5.1 安装成功后计算安装产物 bin 目录（npm 类 = 生效前缀的 bin；Kimi 类 = 脚本实际安装目录的 bin），**仅当该目录确实含刚装好的可执行文件**时登记进 overlay
- [x] 5.2 登记**先于**装后探测 ⇒ 平台自身探测立即可见新装 CLI
- [x] 5.3 绝对路径兜底探测（PATH 找不到时按 `<bin>/<command>` 探测）MUST 经平台的命令执行通道（否则 overlay 合并不生效）；其结果**只填已安装版本**，MUST NOT 绕过「已装但探针未过」的语义（能力核验仍走既有通道）
- [x] 5.4 Kimi 官方脚本的安装目录 MUST 可被平台识别并用于装后探测与提示；登记条件以「脚本实际输出 / 实际存在的文件」为准（默认目录常量属假设，标注在案）
- [x] 5.5 安装提示 MUST 对 **Kimi 行也产出**（文案指向其安装目录）；MUST NOT 因对象不是 npm 包就既不给提示也不做装后探测

## 6. npm 安装参数与装后三重验证

映射 requirement：

- MODIFIED REQ-PCI-03
- MODIFIED REQ-PCI-05

- [x] 6.1 npm 安装命令携带 `--allow-scripts=<pkg>`（让包的安装脚本在受支持的 npm 版本下真正执行）
- [x] 6.2 安装子进程的 PATH **前插目标前缀的 bin** ⇒ 从根上消除「装的 PATH ≠ 探测的 PATH」
- [x] 6.3 装完探测失败（装上但跑不起来）时，以 `--include=optional` **自动重试一次**；仍失败则给出**可复制的修复命令**
- [x] 6.4 装后验证三重：① 版本读取**优先读 npm 包 `package.json`**（读不到才跑 `--version`）；② MUST 跑一个**真实命令**证明可执行（`--version` 在平台包缺失时仍可能成功 ⇒ 不可作唯一依据）；③ 失败时区分「没装上 / 装上但缺平台包 / 装上但不在 PATH」三种语义并给对应可行动提示
- [x] 6.5 一次安装 MUST 仍然只包含一个包；镜像与既有安装形态（全局安装 / `--prefix` / `--omit=optional` / `--no-audit --no-fund`）MUST NOT 被破坏

## 7. 更新落点与更新后版本校验

映射 requirement：

- ADDED REQ-PCI-09
- ADDED REQ-PCI-10

- [x] 7.1 更新 SHALL 复用安装的同一条异步作业链（作业标识、轮询、durable 状态）；MUST NOT 为更新另开一条作业链
- [x] 7.2 更新落点 MUST 为**被更新对象当前所在的前缀**：已安装且其命令在宿主 PATH 上 ⇒ 装回同一个前缀；已安装但其命令不在宿主 PATH 上 ⇒ 从已安装包的真实路径反推 npm 前缀
- [x] 7.3 前缀反推失败时 SHALL 显式报错并给出可行动诊断，MUST NOT 静默回退到「另选一个前缀」
- [x] 7.4 更新 MUST NOT 把更新装到与被更新对象当前所在位置不同的前缀（避免「更新装到 A、用户跑的仍是 B」）
- [x] 7.5 前缀候选与可写性判定 MUST 沿用既有口径（真实写入探测 + 三级候选链），MUST NOT 为更新另开一套判定
- [x] 7.6 更新完成后 MUST 比对更新前后的命令版本：版本未变 ⇒ 明确报告「命令仍是旧版」并给可行动提示，MUST NOT 静默判成功、MUST NOT 呈现为更新已完成
- [x] 7.7 Kimi Code 的更新 MUST 沿用官方脚本原地替换，MUST NOT 走 npm 安装路径；SHALL 经装后验证确认更新后的命令真实可用

## 8. 前端提示原样渲染与「不在终端 PATH 上」状态

映射 requirement：

- ADDED REQ-PCI-08
- MODIFIED REQ-PCI-05

- [x] 8.1 安装提示句**原样渲染**（整句逐字呈现、无二次包裹）；后端未给出提示时 MUST NOT 渲染提示节点
- [x] 8.2 前端类型注释与后端文案语义对齐（提示是完整句子，不是「提示目录」）
- [ ] 8.3 安装目标视图增加「不在终端 PATH 上」的**显式状态**：平台能找到该 CLI、而其 bin 目录不在用户终端 PATH 上时为真；用户终端 PATH 取不到时 MUST NOT 判为真（**待实施**：后端安装目标视图尚无该字段；登录 shell PATH 快照已就位）
- [ ] 8.4 该状态在界面上**显式呈现**（MUST NOT 仅给一句自由文本让用户自行推断；MUST NOT 呈现成「未安装」）—— 随 8.3 一并落地
- [x] 8.5 平台 MUST NOT 自动改写用户 shell profile、MUST NOT 自动创建 symlink；处置内容以可复制形式给出；该状态 MUST NOT 阻断安装、探测或使用（用户**显式触发**的暴露动作见第 9 包与 REQ-PCI-11）

## 9. 「暴露到终端 PATH」显式动作

映射 requirement：

- ADDED REQ-PCI-11
- ADDED REQ-PCI-08

- [x] 9.1 暴露动作 MUST 仅在用户**显式触发**时执行（前端按钮：已安装才显示）；MUST NOT 自动创建软链
- [x] 9.2 MUST NOT 改写用户 shell 配置文件（`.bashrc` / `.zshrc` / 等价文件），MUST NOT 改写 PATH 环境变量
- [x] 9.3 候选目录 MUST 同时满足「确实在登录 shell PATH 上」+「存在且真实可写」+「无同名残留」；候选顺序 `~/.local/bin` → `~/bin` → 终端 PATH 上其它存在目录；MUST NOT 在终端 PATH 之外的目录建软链
- [x] 9.4 动作 MUST 只建**单个**软链（`<dir>/<command>` → 已安装二进制）；同名残留不指向本次目标时 MUST 跳过该目录，MUST NOT 覆盖用户已有文件
- [x] 9.5 建链后 MUST **重验**（目录仍在登录 shell PATH 上 + 软链可解析到目标）；失败 MUST 只清理本次新建的软链并如实报错
- [x] 9.6 未安装对象 MUST 明确拒绝（不得建任何软链）；Windows MUST 明确报不支持，不静默降级
- [x] 9.7 端点 `POST /api/providers/{target}/expose` + 前端按钮与「成功 / 失败原因」呈现

## 10. 聚合索引成员覆盖验收改走 codegraph 库 API

映射 requirement：

- MODIFIED REQ-IND-01
- MODIFIED REQ-IND-04

- [x] 10.1 成员覆盖验收 MUST 走 codegraph **库 API**（`CodeGraph.open(root)` + 逐成员 `hasFilesUnder(member)`），MUST NOT 以 `codegraph files --json` 的全量文件清单作为常规验收路径（载荷与仓库规模线性相关，3.1 GB 索引 ⇒ 清单约 230 MB，任何抓取上限都装不下）
- [x] 10.2 目录边界 MUST 是**目录语义**（`member-al` MUST NOT 命中 `member-alpha`）
- [x] 10.3 Node helper + **codegraph 自带 Node** 优先（PATH 上的 node 实测 ≥ 22.5 才回退采用）；helper 源码内嵌随源码走、运行时落唯一临时文件、执行后清理
- [x] 10.4 SDK 路径解析顺序 MUST 为：平台自身安装前缀（npm 标准布局）→ `npm root -g` 探针 → 从 `codegraph` 可执行真实路径反推
- [x] 10.5 库 API 不可用（未安装 / 与受支持版本不兼容）⇒ 回退既有 CLI 清单路径并留**可审计 warning**；回退 MUST NOT 成为常规路径
- [x] 10.6 健康闸（索引过期 / 仍有待处理变更 / state 异常 / 文件计数为零）MUST **只记 warning**，MUST NOT 单独阻断激活（保持「不因健康问题阻断」语义）
- [x] 10.7 排除性查询（非成员目录 / `.worktrees` / `.aria` / 构建产物未被索引）MUST 保持既有 CLI 查询机制与判定口径不变
- [x] 10.8 回退路径 MUST 保留 64 MiB 专用配额与专用截断码 `codegraph_files_output_truncated`；MUST NOT 把截断误报成协议不符
- [x] 10.9 失败文案表补齐新码：`codegraph_sdk_not_found` / `codegraph_node_unavailable` / `codegraph_coverage_failed` / `codegraph_sdk_incompatible`

### 界面验收状态矩阵（第 8 包 / 第 9 包）

映射 requirement：

- MODIFIED REQ-PCI-02
- MODIFIED REQ-PCI-05
- ADDED REQ-PCI-08
- ADDED REQ-PCI-09
- ADDED REQ-PCI-10
- ADDED REQ-PCI-11

| 状态 | 用户看到 / 能点什么 | 不得出现什么 |
|---|---|---|
| 未安装 | 「安装」+ 手动命令；点击发异步安装请求 | 「更新」 |
| Kimi 点击后未确认 | 脚本来源提示、确认 / 取消 | 未确认即发安装请求 |
| 安装进行中 | 安装中提示、按钮禁用、轮询进度 | 重复提交 |
| 装完可用但未核验 | 真实版本 + 整句 PATH 提示；显式「核验」 | 仍显示「未安装」、自动标记已核验 |
| 装完已核验且版本落后 | 「更新」+ 本地 / 最新版本 | 「安装」 |
| 装完已核验且版本最新 | 不显示安装 / 更新 | 无意义重装 |
| 安装失败 | 失败分类、诊断、手动命令与可重试入口 | 伪报成功 / 吞掉错误 |
| CLI 在平台登记目录内但不在用户终端 PATH 上 | 显式「不在终端 PATH 上」状态（待 8.3 落地）+ 可复制处置内容 | 呈现成「未安装」；自动改用户 shell |
| 已安装且用户点「暴露到终端 PATH」 | 暴露成功提示（落在哪个目录、新终端可运行哪个命令） | 自动改 `.bashrc` / `.zshrc`；自动创建软链 |
| 暴露失败（无合格候选目录 / 重验失败） | 如实失败原因 + 各候选目录的不合格原因 | 伪报暴露成功；覆盖用户同名文件 |
| codegraph 可用 | 入口刷新可用性 | 因截断码显示「安装」（截断不是工具缺失） |
| 更新完成但版本未变 | 明确「命令仍是旧版」+ 可行动诊断 | 静默成功 / 呈现为「更新已完成」 |
| 更新落到与被更新对象不同的前缀 | 明确失败并说明落点不一致 | 出现「更新装到 A、用户跑的仍是 B」 |

### 聚合索引验收状态矩阵（第 10 包）

映射 requirement：

- MODIFIED REQ-IND-01
- MODIFIED REQ-IND-04
- MODIFIED REQ-IND-05

| 状态 | 平台行为 | 不得出现什么 |
|---|---|---|
| 库 API 可用、成员全覆盖 | 激活索引；无告警 | 拉取全量文件清单 |
| 库 API 可用、某成员目录下无可索引文件 | 报覆盖不足（点名未覆盖成员）⇒ 保留 last-known-good + degraded | 伪造 active；把覆盖不足当成功 |
| 库 API 可用、索引健康异常（过期 / 有待处理 / state 异常 / 文件计数为零） | 激活索引 + **告警**（随索引记录留存） | 仅因健康问题阻断激活 |
| 库 API 不可用（未安装 / 不兼容） | 回退 CLI 清单路径 + 可审计回退痕迹 | 静默切换路径；把回退当常规路径 |
| 回退路径上 `files --json` 被截断 | 报专用码 `codegraph_files_output_truncated` | 误报 `codegraph_files_invalid_json` |
| codegraph CLI 本身缺失 | 致命路径（`codegraph_missing`） | 因库 API 不可用而误判致命 |

## 11. OpenSpec 归档同步与留痕

映射 requirement：

- MODIFIED REQ-PCI-02
- MODIFIED REQ-PCI-03
- MODIFIED REQ-PCI-05
- MODIFIED REQ-IND-01
- MODIFIED REQ-IND-04
- MODIFIED REQ-IND-05
- ADDED REQ-PCI-08
- ADDED REQ-PCI-09
- ADDED REQ-PCI-10
- ADDED REQ-PCI-11

- [ ] 11.1 本 change 的 `proposal.md` / `design.md` / 两份 delta / `tasks.md` 四件齐备且互相对齐（requirement 编号逐字展开、无缩写）
- [ ] 11.2 delta 为 **MODIFIED**（REQ-PCI-02、REQ-PCI-03、REQ-PCI-05、REQ-IND-01、REQ-IND-04、REQ-IND-05）+ **ADDED**（REQ-PCI-08、REQ-PCI-09、REQ-PCI-10、REQ-PCI-11）；既有 requirement 的正文与既有场景 MUST 逐字保留，只做增补 —— **例外（按实施口径更新）**：REQ-IND-01 的成员覆盖验收条款（改为库 API 路径）、REQ-PCI-03 的安装参数形态（补 `--allow-scripts=<pkg>` 与 `--include=optional` 重试）、REQ-PCI-08 的 symlink 禁止条款（收敛为「不自动」，与 REQ-PCI-11 的显式动作对齐）、REQ-PCI-09 的前缀反推口径（改为「从已安装包真实路径反推」）
- [ ] 11.3 MUST NOT 改主规格（`openspec/specs/**`）—— 归档时由 delta 自动同步
- [ ] 11.4 归档同步时对齐既有漂移：主规格 REQ-IND-01 的「exact version 钉定 v1.5.0」与实现现状（`>=1.6.0, <2.0.0`）—— **版本门本轮不动**，对齐由双审处理
- [ ] 11.5 复核四家 provider 目录**零改动**（`git diff --stat` 复核），并把本轮的口径变更（overlay、可写性、装后验证、专用码、库 API 覆盖验收、暴露动作）记入变更说明

## 12. 容器 E2E 复验与清理

映射 requirement：

- MODIFIED REQ-PCI-02
- MODIFIED REQ-PCI-03
- MODIFIED REQ-PCI-05
- MODIFIED REQ-IND-01
- MODIFIED REQ-IND-04
- MODIFIED REQ-IND-05
- ADDED REQ-PCI-08
- ADDED REQ-PCI-09
- ADDED REQ-PCI-10
- ADDED REQ-PCI-11

- [ ] 12.1 场景 A（未装全部 provider）：走真实 HTTP 安装 → 轮询作业；修复后判据 = 作业的已安装版本非空、install-targets 的 `cli_available` 为真且本地版本非空（修复前基线：作业完成但版本为空、仍显示未安装）
- [ ] 12.2 场景 B（Kimi 官方脚本安装）：修复后判据 = 已安装版本非空、安装提示含其安装目录、overlay 含该目录（修复前基线：探测失败、无提示）
- [ ] 12.3 场景 C（长路径合成聚合根）：修复后判据 = 成员覆盖验收走库 API 且不再出现 `codegraph_files_invalid_json`（修复前基线：JSON 解析报错）
- [ ] 12.4 截断路径的降级判定由单元级覆盖（构造超配额负载）⇒ 容器侧只断言「原因码是专用码、索引 degraded、保留 last-known-good、不伪造 active」
- [ ] 12.5 可见性证据 MUST 来自平台自身的探测结果；MUST NOT 用手工改 PATH 的方式冒充平台可见性
- [ ] 12.6 清理本轮容器 / 镜像 / 临时资产（只删本次产物）；宿主 dotfiles 零改动
