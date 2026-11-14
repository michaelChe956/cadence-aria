# Tasks: fix-provider-install-no-effect-and-codegraph-json

> **实施状态（2026-11-13）**：契约已获批并**已实施**；勾选状态按仓库实现证据记录（`[x]` = 已落地，`[ ]` = 待办）。未闭环项集中在第 11 包（归档同步：主规格漂移对齐与 CHANGELOG 留痕）与第 12 包（容器 E2E 复验）；第 13 包（终审遗留项）**已由本 change 的修复波落地**，逐条附实现证据（提交 / 红测试）。
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

## 3. 启动期登记用户工具目录与登录 shell 探测

映射 requirement：

- MODIFIED REQ-PCI-02
- MODIFIED REQ-PCI-05
- ADDED REQ-PCI-12

- [x] 3.1 服务启动时把**实际存在**的用户工具目录登记进 overlay（常见版本管理器 / 包管理器的用户级 bin 目录 + 环境驱动项：`~/.npmrc` 的 `prefix=` bin、nvm 默认 node 的 bin、mise 的 node installs、fnm 目录）；npm 全局前缀（`npm prefix -g`）/`NPM_PREFIX`/Kimi 安装目录**不在启动清单内** —— 由第 5 包的**装后登记**按「实际产物所在目录」覆盖
- [x] 3.2 **只登记、不写盘、不创建目录**：不存在的目录 MUST NOT 被登记，MUST NOT 因此创建目录
- [x] 3.3 启动路径上的前缀探针用**短超时**（不得引入与安装作业同量级的阻塞）
- [x] 3.4 登记时机 MUST 在「监听就绪之后、首次 provider 健康刷新之前」；监听失败 MUST 保持零副作用
- [x] 3.5 启动期探测**交互式登录 shell**（登录 + 交互、`kill_on_drop` 兜底回收）取回 agent 专属环境变量，最少覆盖 `PI_CODING_AGENT_DIR` / `CODEX_HOME` / `CLAUDE_CONFIG_DIR`（收敛清单集中一处、可追加）；探测 MUST 有 **2s 硬上限**且在**后台异步**执行，MUST NOT 阻塞启动
- [x] 3.6 探测值 MUST 只**补进平台发起的子进程 env**，且严格遵守优先级：调用方显式覆盖层 > 平台进程自身 env > 探测值；MUST NOT 用探测值覆盖用户显式指定的值；MUST NOT 生成空值变量
- [x] 3.7 注入 MUST 在**两个命令汇聚点**（进程拉起通道 + 同步捕获通道，含「调用方未提供环境覆盖」那条路径）都生效；MUST NOT 只在其中一条生效；MUST NOT 因此改变 REQ-PCI-02 探针口径、REQ-PCI-08 的「不在终端 PATH 上」判定或 REQ-PCI-11 的暴露动作语义
- [x] 3.8 探测失败 / 超时 / 不可用（未设 `SHELL` / shell 不可执行 / 输出不含标记行）MUST 是空操作：不注入、不阻塞，安装、装后探测、终端 PATH 判定与索引验收 SHALL 照常进行

实施注记：启动期登记清单覆盖 `~/.local/bin` / `~/.npm-global/bin` / `~/.npmrc` 的 `prefix=` bin / nvm 默认 node bin / mise node installs / fnm 目录 / `/opt/homebrew/bin` / `/usr/local/bin` 等**实际存在**的目录（`src/cross_cutting/path_overlay.rs` 的 `user_tool_dirs` + `host_absolute_tool_dirs`）；`~/.kimi-code/bin`、npm 全局前缀与 `NPM_PREFIX` 不在启动清单内，由第 5 包的**装后登记**路径覆盖（Kimi 走官方脚本，安装时机才确定目录）。

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
- [x] 8.3 安装目标视图增加「不在终端 PATH 上」的**显式状态**：平台能找到该 CLI、而其 bin 目录不在用户终端 PATH 上时为真；用户终端 PATH 取不到时 MUST NOT 判为真（**已实施**：`src/web/handlers/provider_installation.rs` 的 `off_path` DTO 字段 + `resolve_off_path`（登录 shell PATH 快照 ∪ 平台登记目录），提交 `d7d759c7`；红测试 `9d0abe95`）
- [x] 8.4 该状态在界面上**显式呈现**（MUST NOT 仅给一句自由文本让用户自行推断；MUST NOT 呈现成「未安装」）（**已实施**：`web/src/api/types/provider.ts` 的 `off_path` + `web/src/components/lifecycle/ProviderCapabilityCard.tsx` 的「已安装，但不在你的终端 PATH 上」徽标，提交 `621d3269`；红测试 `9edc521f`）
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
| CLI 在平台登记目录内但不在用户终端 PATH 上 | 显式「不在终端 PATH 上」徽标（`off_path` 已实施）+ 可复制处置内容 / 「暴露到终端 PATH」入口 | 呈现成「未安装」；自动改用户 shell |
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
- ADDED REQ-PCI-12

- [x] 11.1 本 change 的 `proposal.md` / `design.md` / 两份 delta / `tasks.md` 四件齐备且互相对齐（requirement 编号逐字展开、无缩写）（**已对齐**：2026-11-13 补 REQ-PCI-12 并清理与实现冲突的旧非目标/设计文本后，`openspec validate ... --strict` 通过）
- [x] 11.2 delta 为 **MODIFIED**（REQ-PCI-02、REQ-PCI-03、REQ-PCI-05、REQ-IND-01、REQ-IND-04、REQ-IND-05）+ **ADDED**（REQ-PCI-08、REQ-PCI-09、REQ-PCI-10、REQ-PCI-11、REQ-PCI-12）；既有 requirement 的正文与既有场景 MUST 逐字保留，只做增补 —— **例外（按实施口径更新）**：REQ-IND-01 的成员覆盖验收条款（改为库 API 路径）、REQ-PCI-03 的安装参数形态（补 `--allow-scripts=<pkg>` 与 `--include=optional` 重试）、REQ-PCI-08 的 symlink 禁止条款（收敛为「不自动」，与 REQ-PCI-11 的显式动作对齐）、REQ-PCI-09 的前缀反推口径（改为「从已安装包真实路径反推」）
- [x] 11.3 MUST NOT 改主规格（`openspec/specs/**`）—— 归档时由 delta 自动同步（**已核对**：本次同步只改本 change 目录，`openspec/specs/**` 零改动）
- [ ] 11.4 归档同步时对齐既有漂移：主规格 REQ-IND-01 的「exact version 钉定 v1.5.0」与实现现状（`>=1.6.0, <2.0.0`）—— **版本门本轮不动**，对齐由双审处理
- [x] 11.5 复核四家 provider 目录**零改动**（`git diff --stat` 复核）—— **已核对**：`git diff --stat c2929b02..HEAD -- 'src/cross_cutting/*_provider/**'` 空输出
- [ ] 11.6 把本轮的口径变更（overlay、可写性、装后验证、专用码、库 API 覆盖验收、暴露动作、登录 shell agent 专属环境变量注入）记入变更说明（CHANGELOG / 发布说明），由该文件所有者执行

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

## 13. 终审遗留项处置（**已由本 change 修复波落地**）

> 来源：终审（k3-reviewer，审阅快照 `44a1b4c4`）判定的必修项与补充建议。**修复波已在本 change 分支内逐条落地**（提交 `0f69fe90` / `f9c776d8` / `84675998` / `55296a8f` / `ada50bca` / `ea1434b1` / `f165474e` / `60b2e064`）；下列勾选状态按**仓库实现证据**记录，每条括注实现落点、提交与红测试。

映射 requirement：

- MODIFIED REQ-PCI-02
- MODIFIED REQ-PCI-03
- MODIFIED REQ-PCI-05
- MODIFIED REQ-PCI-08
- MODIFIED REQ-PCI-09
- MODIFIED REQ-PCI-10
- MODIFIED REQ-PCI-11
- MODIFIED REQ-IND-01

- [x] 13.1 PATH overlay 合并顺序 MUST 为 **Prepend → base → Append**（装完目录抢在 base 之前、启动期发现目录排在 base 之后）—— **已落地**：`src/cross_cutting/path_overlay.rs` 的 `effective_path_in` 按 Prepend → base → Append 合并并去重；提交 `55296a8f`；红测试 `effective_path_append_never_preempts_base_entries`（契约归属 REQ-PCI-02 / REQ-PCI-05）
- [x] 13.2 更新落点的两个独立触发 MUST 都成立（契约归属 REQ-PCI-09）—— **已落地**：`src/product/provider_installation/runner.rs` 的 `resolve_effective_prefix` 先取 visible（真实可写）前缀，①「PATH 上可见且真实可写的前缀」**优先于**「已离 PATH 的旧包所在前缀」；② `installed_prefix` 使用**未经过滤**的 `npm prefix -g` 原始结果（`bin ∈ PATH` 过滤只作用于新装候选），过滤与已装位置查找互不短路；提交 `0f69fe90`；红测试 `update_prefers_prefix_of_cli_visible_on_path_over_stale_fallback_copy` / `update_targets_raw_npm_global_install_when_its_bin_is_off_path`
- [x] 13.3 装后验证 MUST 在**首次安装与 `--include=optional` 重试两条路径**都消费「真实命令」结果（`real_command_ok`）—— **已落地**：`runner.rs` 的完成判定为「版本可读 **且** 真实命令可跑」（`detected.filter(|_| real_command_ok)`），真实命令失败不判 `completed` 并转入重试；重试成功后按新结果重算安装提示；提交 `0f69fe90`；红测试 `real_command_failure_is_not_completed_and_retries_with_include_optional` / `installed_but_off_path_failure_is_distinguished_from_missing_platform_package`（契约归属 REQ-PCI-03 / REQ-PCI-05）
- [x] 13.4 codegraph 真实命令验证 MUST 不依赖「已建索引」且确实加载 bundle；MUST NOT 传元数据抑制旗标（与 REQ-IND-01 已裁决**只带 `--json`** 的同类问题）—— **已落地**：`runner.rs` 的 `post_install_verification_argv` 对 codegraph 下发 `status --json`（实测 1.6.2 在未初始化目录 exit 0，且一切命令经平台包自带 node 启动 ⇒ 平台包缺失时 exit 1），**不带**任何元数据抑制旗标；提交 `0f69fe90`；红测试 `codegraph_real_command_does_not_require_an_index`（与 REQ-PCI-05 的「真实命令」口径及 REQ-IND-01 的调用边界一致）
- [x] 13.5 bundled node 解析 MUST 覆盖「当前平台 / 版本下」的 nested、hoisted 与自愈缓存三种布局 —— **已落地**：`codegraph_coverage.rs` 的 `bundled_node_candidates` 按 `codegraph_platform_target()` 映射的平台名覆盖三种布局（nested / hoisted / 上游自愈缓存），node 不可用时映射为 `codegraph_sdk_incompatible` 并回退 CLI 清单验收；提交 `f9c776d8`；红测试 `bundled_node_resolves_nested_platform_layout` / `bundled_node_resolves_hoisted_platform_layout` / `bundled_node_resolves_self_heal_cache_layout`（契约归属 REQ-IND-01）
- [x] 13.6 能力卡父级 MUST NOT 因「已安装但未核验」而卸载「暴露到终端 PATH」入口（否则 UI 主路径不可达）—— **已落地**：能力卡按 `cli_available && off_path` 渲染**行级** expose 入口（与安装块解耦 ⇒ 已装未核验行也可达），安装块内同义按钮由共享开关 `exposeEnabled={false}` 关闭；提交 `f165474e` / `60b2e064`；红测试 `web/src/components/lifecycle/ProviderCapabilityCard.test.tsx`（契约归属 REQ-PCI-11）
- [x] 13.7 `off_path` 判定与 expose 的 CLI 发现范围 MUST 为**宿主 PATH ∪ 平台登记目录（overlay）**，MUST NOT 漏掉「仅宿主 PATH 可见」的 CLI —— **已落地**：handler 的 `install_search_dirs` = 服务进程生效 PATH（overlay 合并后）∪ 登录 shell PATH ∪ 已知落点（`~/.npm-global/bin`、`/usr/local/bin`），`expose.rs` 的二进制查找同口径；提交 `ada50bca`；红测试 `off_path_counts_service_process_path_as_platform_visible` / `expose_finds_binary_from_service_process_path`（契约归属 REQ-PCI-08 / REQ-PCI-11）
- [x] 13.8 Kimi Code 的更新前版本快照 MUST 同样取快照并比对、未变时如实报告「命令仍是旧版」—— **已落地**：`runner.rs` 的 `version_before_update` 对脚本类（Kimi）取更新前版本快照，`before == after` ⇒ 作业判失败并报「命令仍是旧版」（不得静默成功）；提交 `0f69fe90`；红测试 `kimi_update_reports_still_old_version_when_script_keeps_same_version` / `kimi_update_completes_when_script_upgrades_version`（契约归属 REQ-PCI-10）
  - **补充（终审 R10 新发现）**：更新**前**与更新**后**的版本取数 MUST **同源同口径**（同一探测通道、同一解析口径）—— 否则「版本未变」恒不成立，「命令仍是旧版」判定在生产上永不触发。该口径收口已在本 change 的**修复波**落地（提交待补），并补一条**不注入探针桩、走真实取数路径**的用例（同版本 ⇒ 失败「命令仍是旧版」；版本升级 ⇒ completed）
- [x] 13.9 前端观察器 P2：dirty 补跑的等待期内再次收到普通唤醒时，MUST NOT 把 1s 最短间隔改写成 250ms —— **已落地**：`web/src/hooks/useWorkspaceSessionObservers.ts` 给 dirty 补跑单列 `dirtyTimer` 槽位（`DIRTY_RERUN_MIN_DELAY_MS = 1_000`），等待期内的新唤醒并入已排补跑、不另排 250ms 档；提交 `ea1434b1`；红测试 `useWorkspaceSessionObservers.polling.test.tsx` 的「keeps the dirty rerun at its minimum gap when another wake-up arrives (F9)」
  - **能力归属**：该行为属前端观察器目录轮询，其**能力边界归属仍待 owner 裁决**；本 change **不为此新立 capability**（只按终审要求把已落地的行为如实记录）
- [x] 13.10 终审「补充建议」（非必修）已随修复波逐条落地：① Node helper 以 `CodeGraph.open(root, { readOnly: true })` 打开索引（提交 `84675998`；`codegraph_coverage_helper.mjs`）；② 测试用线程级存储与生产进程级全局的跨线程差异已有独立的生产路径测试（提交 `55296a8f`；`path_overlay.rs` 的 `process_global_registry_is_shared_across_threads`）；③ `off_path` 的「PATH 分支」夹具已实际建出 CLI，否定分支真被测到（提交 `ada50bca`；`off_path_marks_installed_target_missing_from_login_shell_path`）；④ 被删除的两条既有 handler 断言已恢复（提交 `ada50bca`；`install_targets_endpoint_lists_five_targets_with_actions`）
