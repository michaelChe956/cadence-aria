# provider-cli-installation Specification

## Purpose
为四家 provider CLI（Claude Code、Codex、Pi、Kimi Code）与 codegraph 提供平台内的安装与更新能力，让「未安装 / 版本太低」这一冷启动最常见的用户侧失败成因能在界面内闭环处理。安装入口放在能力探针处，由探针结果与本地状态共同决定显示「安装」还是「更新」；npm 类对象（含 codegraph）走镜像源可配置的 npm 包，Kimi Code 走执行前明示来源的官方脚本；安装以异步 durable 作业执行、完成后重新探测版本与可用性；失败按成因分类并给手动命令兜底，平台不自动提权；安装成功不等于能力已核验，仍需既有核验通道确认。

## Requirements

### Requirement: 安装与更新的适用对象与安装源（REQ-PCI-01）

平台 SHALL 支持对四家 provider 与 codegraph 提供安装 / 更新能力：Claude Code、Codex、Pi、codegraph 走 **npm 包**，**Kimi Code 走官方安装脚本**（非 npm 包）。各对象的安装源与命令名 SHALL 明确且可查。

对**执行远程 shell 脚本**的对象（Kimi Code），界面 MUST 在执行前明确告知「将执行来自 code.kimi.com 的安装脚本」；MUST NOT 让用户在不知来源的情况下触发远程脚本执行。该告知与用户确认 SHALL 覆盖对该对象的**安装与更新**两种触发。

#### Scenario: 五类对象的安装源各自明确

- **WHEN** 用户查看任一对象的安装入口
- **THEN** 系统 SHALL 给出该对象对应的安装源与命令名：Claude Code → npm 包 `@anthropic-ai/claude-code` / 命令 `claude`；Codex → `@openai/codex` / `codex`；Pi → `@earendil-works/pi-coding-agent` / `pi`；codegraph → `@colbymchenry/codegraph` / `codegraph`；Kimi Code → 官方脚本 `https://code.kimi.com/kimi-code/install.sh` / `kimi`；MUST NOT 让 Kimi Code 走 npm 包安装路径

#### Scenario: Kimi 执行前告知脚本来源

- **WHEN** 用户即将触发 Kimi Code 的安装**或更新**
- **THEN** 界面 SHALL 明确告知「将执行来自 code.kimi.com 的安装脚本」；MUST NOT 静默执行远程脚本

### Requirement: 按钮显示规则与本地状态探测（REQ-PCI-02）

安装 / 更新按钮 SHALL 出现在**能力探针那块**（能力卡 / 探针结果面板），其显示 SHALL 由「探针结果」与「本地状态」共同决定：

| 探针结果 | 本地状态 | 显示 |
|---|---|---|
未安装（命令缺失） | 无 | **【安装】**按钮 + 手动命令 |
已安装，探针失败（能力不支持 / 未核验） | 有 | 不显示安装按钮；显示探针失败原因 + 「核验」按钮（若已核验则显示「更新」） |
已安装，版本落后 | 有 | **【更新】**按钮 + 当前版本 vs 最新版本 |
已安装，版本最新 | 有 | 不显示任何按钮 |

「最新版本」判定 SHALL 复用既有 provider health 的命令可用性与版本探测事实（安装完成后 SHALL 重新探测该对象的版本与可用性）。

「探针结果」的口径 SHALL 为 **宿主 PATH ∪ 平台登记目录**（平台自身命令可见 PATH）：平台在判定某对象的命令可用性时，MUST 把**平台登记目录**纳入命令查找范围。装完的 CLI 即使不在宿主 PATH 上、只要落在平台登记目录内，MUST 视为「已安装、探针通过」；MUST NOT 仅以宿主 PATH 作为唯一判据。

#### Scenario: 未安装显示安装按钮

- **WHEN** 某对象的命令在本机不可用
- **THEN** 界面 SHALL 显示「安装」按钮与对应的手动安装命令；MUST NOT 显示「更新」

#### Scenario: 已安装但探针失败不显示安装

- **WHEN** 某对象已安装但探针未通过
- **THEN** 界面 MUST NOT 显示「安装」按钮；SHALL 显示探针失败原因与「核验」按钮；已核验过则显示「更新」

#### Scenario: 版本落后显示更新按钮

- **WHEN** 某对象已安装且已核验，但本地版本低于可查到的最新版本
- **THEN** 界面 SHALL 显示「更新」按钮，并同时显示当前版本与最新版本

#### Scenario: 版本最新不显示按钮

- **WHEN** 某对象已安装且版本已是最新
- **THEN** 界面 MUST NOT 显示安装或更新按钮

#### Scenario: 探针口径含平台登记目录

- **WHEN** 某对象的命令不在宿主 PATH 上、但落在平台登记目录（平台自身命令可见 PATH）内
- **THEN** 平台 SHALL 判该对象「已安装」；命令可用性探测 SHALL 以宿主 PATH ∪ 平台登记目录为查找范围；MUST NOT 仅因宿主 PATH 上找不到该命令就判「未安装」

#### Scenario: 装完落在平台登记目录内即算装上

- **WHEN** 某对象的安装把 CLI 装进了平台登记目录、而宿主 PATH 上没有该命令
- **THEN** 界面 SHALL 按「已安装」呈现；MUST NOT 仍显示「未安装」或仍显示「安装」按钮

### Requirement: npm 镜像源与安装执行形态（REQ-PCI-03）

npm 安装 SHALL 默认使用阿里镜像 `https://registry.npmmirror.com`，并 SHALL 支持通过环境变量 `NPM_REGISTRY` 覆盖（供海外用户切换到官方 registry）。

npm 安装命令 SHALL 采用已验证形态：全局安装、`--prefix <可写前缀>`（避开 sudo）、`--omit=optional`、`--no-audit --no-fund`、`--allow-scripts=<pkg>`（让目标包的安装脚本在受支持的 npm 版本下真正执行）。

装后探测失败（装上但真实命令跑不起来）时，平台 MAY 以 `--include=optional` **重试一次**；`--include=optional` 与 `--omit=optional` 互斥，MUST NOT 在同一次调用里同时出现。

**一次安装 MUST 只包含一个包**：MUST NOT 把多个包合并进同一次 `npm install -g` 调用。

「`--prefix <可写前缀>`」中的「可写」SHALL 按**真实写入探测**判定：MUST 先按权限位预筛（目录对当前用户没有任何写位 ⇒ 直接判不可写），再在该目录内实际创建并删除一个唯一命名的临时文件以确认可写；MUST NOT 只看权限位即判「可写」。系统级前缀若真实写入探测失败，MUST NOT 被选为安装落点；平台 MUST NOT 为写入系统级前缀而提权（沿用 REQ-PCI-06 的「不自动提权」）。

#### Scenario: 默认阿里镜像且可覆盖

- **WHEN** 执行 npm 安装
- **THEN** 安装 SHALL 使用阿里镜像作为 registry；环境变量 `NPM_REGISTRY` 有值时 SHALL 以其为准；MUST NOT 写死单一镜像

#### Scenario: 安装参数形态固定

- **WHEN** 平台执行 npm 安装
- **THEN** 命令 SHALL 含全局安装、`--prefix <可写前缀>`、`--omit=optional`、`--no-audit --no-fund`、`--allow-scripts=<pkg>`；MUST NOT 使用需要提权的默认全局前缀

#### Scenario: 探测失败时以可选依赖重试一次

- **WHEN** 首次安装完成、但装后探测失败（装上但真实命令跑不起来）
- **THEN** 平台 MAY 以 `--include=optional` 重试一次；该次调用 MUST NOT 同时带 `--omit=optional`；重试仍失败 SHALL 给出可复制的修复命令

#### Scenario: 一个包一次安装

- **WHEN** 需要安装多个包
- **THEN** 平台 SHALL 为每个包单独发起一次安装；MUST NOT 把多个包合并进同一次 `npm install -g`

#### Scenario: 可写性按真实写入探测判定

- **WHEN** 某候选前缀的权限位显示可写、但真实写入探测失败
- **THEN** 平台 MUST 判该前缀不可写；MUST NOT 把它选为 `--prefix` 的落点

#### Scenario: 系统级不可写前缀不得被选中

- **WHEN** 系统级全局前缀的真实写入探测失败
- **THEN** 平台 MUST 改选真实可写的用户级前缀；MUST NOT 选中该系统级前缀

#### Scenario: 判定可写不得提权

- **WHEN** 平台判定候选前缀是否可写
- **THEN** 平台 MUST NOT 以提权方式写入该目录、MUST NOT 自动执行 sudo 重试

### Requirement: 异步安装作业与装完重新探测（REQ-PCI-05）

安装 SHALL 以**异步作业**执行，MUST NOT 阻塞 HTTP 请求。发起安装 SHALL 立即返回作业标识；查询接口 SHALL 轮询作业进度与结果。

作业状态 SHALL 是 **durable** 的：页面刷新或进程重启后 MUST NOT 丢失进度。

作业流程 SHALL 为：记录 `running` → 执行安装命令（npm 对象走 npm 安装；Kimi Code 走官方脚本）→ 安装成功后**重新探测**该对象的版本与可用性 → 记录 `completed` 与结果。

**Kimi Code 的更新 SHALL 复用官方脚本通道**：重新执行官方安装脚本（脚本自行解析最新版本并原地替换本机二进制），MUST NOT 让 Kimi Code 走 npm 安装路径，且 SHALL 满足 REQ-PCI-01 的来源告知与用户确认。

安装成功 MUST NOT 被当作能力已核验：安装后仍 SHALL 通过既有能力核验通道确认能力。

装后重新探测的判定口径 SHALL 为**平台可见即判成功**：装后探测的命令查找范围为宿主 PATH ∪ 平台登记目录；装完的 CLI 只要在平台登记目录内，SHALL 判为安装成功并刷新其版本与可用性，MUST NOT 在装完后仍显示「未安装」。

安装提示（`path_hint`）SHALL 以**完整句子**原样呈现：前端 MUST NOT 对后端已给出的提示句做二次包裹或改写；后端未给出提示时 MUST NOT 渲染提示节点。

装后验证 SHALL 为**三重验证**：① 版本读取**优先读 npm 包的 `package.json`**（读不到才执行 `--version`）；② MUST 跑一个**真实命令**证明可执行（`--version` 在平台包缺失时仍可能成功，MUST NOT 作为唯一依据）；③ 失败时 SHALL 区分「没装上 / 装上但缺平台包 / 装上但不在 PATH」三种语义并给出可复制的修复命令。

宿主 PATH 上找不到该命令时，平台 SHALL 以**绝对路径**（`<bin>/<command>`）做一次兜底探测；该探测 MUST 经平台的命令执行通道（否则平台登记目录不生效），且其结果**只填已安装版本**，MUST NOT 绕过 REQ-PCI-02「已装但探针未过」的语义（能力核验仍走既有通道）。

Kimi Code 的官方脚本安装目录 SHALL 可被平台识别：平台 SHALL 能在装后把该安装目录纳入平台命令可见范围（平台登记目录），并 SHALL 用它做装后探测、给出安装提示；MUST NOT 因该对象不是 npm 包就既不给提示也不做装后探测。

#### Scenario: 发起安装立即返回

- **WHEN** 用户触发某对象的安装
- **THEN** 请求 SHALL 立即返回作业标识，MUST NOT 等待安装完成才返回

#### Scenario: 轮询可得进度与结果

- **WHEN** 用户按作业标识查询
- **THEN** 系统 SHALL 返回该作业的当前状态与结果（运行中 / 已完成 / 已失败）；MUST NOT 只返回「已提交」

#### Scenario: 刷新页面不丢进度

- **WHEN** 安装进行中用户刷新页面
- **THEN** 作业状态 SHALL 仍可查询；MUST NOT 因刷新而丢失或重置进度

#### Scenario: 装完重新探测版本与可用性

- **WHEN** 某对象的安装成功
- **THEN** 系统 SHALL 重新探测该对象的版本与可用性并刷新其状态；MUST NOT 在装完后仍显示「未安装」

#### Scenario: Kimi 更新走官方脚本

- **WHEN** 用户对已安装的 Kimi Code 触发更新
- **THEN** 平台 SHALL 重新执行官方安装脚本完成原地升级并重新探测版本；MUST NOT 走 npm 安装路径，且 MUST NOT 在用户未确认来源时执行远程脚本

#### Scenario: 安装不等于核验

- **WHEN** 某对象安装成功
- **THEN** 该对象的能力状态 SHALL 仍需通过既有显式核验通道确认；MUST NOT 因安装成功而自动标记能力已确认，也 MUST NOT 自动触发核验探针

#### Scenario: 装完在平台登记目录内即判成功

- **WHEN** 安装把 CLI 装进了平台登记目录、而宿主 PATH 上没有该命令
- **THEN** 系统 SHALL 以宿主 PATH ∪ 平台登记目录为口径重探，判该对象已安装并记录其版本；MUST NOT 在装完后仍显示「未安装」

#### Scenario: 安装提示句原样呈现

- **WHEN** 安装结果带有安装提示句
- **THEN** 界面 SHALL 逐字原样呈现该提示句；MUST NOT 对其做二次包裹、改写或拼接成新句子；后端未给出提示时 MUST NOT 渲染提示节点

#### Scenario: Kimi 安装目录被识别并用于探测与提示

- **WHEN** Kimi Code 经官方脚本安装完成、安装目录落在平台可识别位置
- **THEN** 平台 SHALL 把该安装目录纳入平台命令可见范围并用它完成装后探测；SHALL 给出指向该安装目录的安装提示；MUST NOT 因该对象不是 npm 包就既不给提示也不做装后探测

#### Scenario: 装后验证是三重验证

- **WHEN** 某对象安装完成
- **THEN** 平台 SHALL 优先读 npm 包 `package.json` 取版本（读不到才执行 `--version`），并跑一个真实命令证明可执行；MUST NOT 只以 `--version` 成功作为「装上可用」的依据

#### Scenario: 装后失败区分三种语义

- **WHEN** 装后验证失败
- **THEN** 平台 SHALL 区分「没装上 / 装上但缺平台包 / 装上但不在 PATH」三种语义并给出可复制的修复命令；MUST NOT 用同一条含糊文案覆盖三种原因

#### Scenario: PATH 找不到时用绝对路径兜底探测

- **WHEN** 宿主 PATH 上找不到该对象的命令、但平台登记目录内有其可执行文件
- **THEN** 平台 SHALL 以绝对路径做一次兜底探测并把探测到的版本填入已安装版本；MUST NOT 因此把该对象判为能力已核验

### Requirement: 安装失败分类与手动命令兜底（REQ-PCI-06）

安装失败 SHALL 按成因分类并给出对应的中文提示与**手动命令**：

| 失败类型 | 判定 | 提示要点 |
|---|---|---|
权限不足 | 输出含权限拒绝类信号 | 「装不上（权限不足）」并给出需管理员权限运行的命令 |
网络 / 镜像不可达 | 输出含超时 / 连接失败 / 域名解析失败 / registry 404 类信号 | 「装不上（网络问题）」并给出手动命令与换镜像的提示 |
包名不存在 | registry 返回 404 且响应体表明包未找到 | 「装不上（找不到这个包）」并给出手动安装命令 |
其它 | 其它非零退出 | 「安装失败」+ stderr 摘要 |

平台 SHALL **不自动提权**（MUST NOT 自动执行 sudo）；手动命令文本 SHALL 交由用户自行决定是否以管理员权限运行。npm 类对象的手动命令 SHALL 带 registry 参数；Kimi Code 的手动命令 SHALL 为官方脚本命令。

#### Scenario: 权限不足给管理员命令

- **WHEN** 安装因权限不足失败
- **THEN** 提示 SHALL 说明权限不足并给出需管理员权限运行的命令；平台 MUST NOT 自动提权重试

#### Scenario: 网络问题给手动命令与换镜像提示

- **WHEN** 安装因网络或镜像不可达失败
- **THEN** 提示 SHALL 说明是网络问题并给出可手动运行的命令与更换镜像的提示

#### Scenario: 包不存在给手动命令

- **WHEN** registry 返回该包不存在
- **THEN** 提示 SHALL 说明找不到这个包并给出手动安装命令

#### Scenario: 其它失败给 stderr 摘要

- **WHEN** 安装以其它非零退出失败
- **THEN** 提示 SHALL 给出「安装失败」与 stderr 摘要

#### Scenario: 手动命令按安装源给出

- **WHEN** 系统给出手动安装命令
- **THEN** npm 类对象 SHALL 给出带 registry 参数的 npm 全局安装命令；Kimi Code SHALL 给出官方脚本命令；MUST NOT 给 npm 对象错误的脚本命令或反之

#### Scenario: 平台不自动 sudo

- **WHEN** 安装因权限不足需要提权
- **THEN** 平台 MUST NOT 自动以提权方式重试或自动执行 sudo；提权与否由用户自行决定

### Requirement: 最新版本判定与 durable 缓存（版本来源与安装来源分离）（REQ-PCI-07）

「有新版」的判定 SHALL 以**该对象的版本来源**为数据来源，并以**精确版本比较**（`major.minor.patch` + 可选预发布后缀）比对本地版本；预发布版本 SHALL 视为**低于**对应的正式版本。

版本来源 SHALL 与**安装来源**相互独立：npm 类对象的版本来源为其 npm 包；**Kimi Code 的版本来源为 npm 包 `@moonshot-ai/kimi-code`**（官方以该包名发布，其 `dist-tags.latest` 与官方 `https://code.kimi.com/kimi-code/latest` 端点给出同一版本号），而 Kimi Code 的**安装来源仍为官方脚本**。Kimi Code SHALL 参与「有新版」判定，MUST NOT 因「非 npm 安装源」而豁免版本判定。

查询结果 SHALL 缓存 **24 小时**且缓存 SHALL 是 **durable**（进程重启后仍可复用）。缓存条目 SHALL 至少记录**版本来源包名**、最新版本与查询时间戳。

#### Scenario: 用 registry latest 与本地版本精确比较

- **WHEN** 判定某对象是否有新版
- **THEN** 系统 SHALL 取该对象版本来源的 `dist-tags.latest` 并与本地版本做精确比较；MUST NOT 用字符串前缀或浮点近似比较

#### Scenario: 预发布版本低于正式版

- **WHEN** 本地版本是预发布版而最新版本是对应的正式版
- **THEN** 判定 SHALL 结论为「有新版」；预发布 MUST NOT 被视为不低于正式版

#### Scenario: 查询结果 24 小时内复用

- **WHEN** 距上次成功查询未超过 24 小时
- **THEN** 系统 SHALL 直接使用缓存的版本结论，不发起新的 registry 查询

#### Scenario: 缓存跨进程重启仍在

- **WHEN** 进程重启后再次判定是否有新版
- **THEN** 系统 SHALL 仍能取到 24 小时内的缓存结论；MUST NOT 因重启而丢失缓存

#### Scenario: Kimi Code 也做版本判定

- **WHEN** 判定 Kimi Code 是否有新版
- **THEN** 系统 SHALL 取 npm 包 `@moonshot-ai/kimi-code` 的 `dist-tags.latest` 与本地版本做精确比较并给出结论；MUST NOT 因 Kimi 走官方脚本安装而跳过版本判定

### Requirement: 「装了但不在终端 PATH 上」的显式状态（REQ-PCI-08）

平台 SHALL 为每个安装目标暴露一个显式的「不在终端 PATH 上」状态：当且仅当**平台能找到**该对象的 CLI（宿主 PATH ∪ 平台登记目录），而**该 CLI 所在 bin 目录不在用户终端（登录 shell）PATH 上**时为真。界面 SHALL 显式呈现该事实，MUST NOT 呈现成「未安装」。

平台取得用户终端 PATH 的事实取自对用户登录 shell PATH 的探测；该事实取不到时，MUST NOT 判该状态为真。

平台 MUST NOT 为让 CLI 在用户终端可见而**自动**改写用户的 shell profile（MUST NOT 写 `.bashrc` / `.zshrc` / 等价文件，MUST NOT 自动创建 symlink）；处置内容 SHALL 以可复制的形式给出。该状态 MUST NOT 阻断该对象的安装、探测或使用。用户**显式触发**的暴露动作按 REQ-PCI-11 执行。

#### Scenario: 平台可见但终端 PATH 没有时状态为真

- **WHEN** 平台在宿主 PATH ∪ 平台登记目录中能找到某对象的 CLI、而该 CLI 的 bin 目录不在用户终端 PATH 上
- **THEN** 该目标的「不在终端 PATH 上」状态 SHALL 为真，界面 SHALL 显式呈现；MUST NOT 把它呈现成「未安装」

#### Scenario: 终端 PATH 可见时状态为假

- **WHEN** 某对象 CLI 的 bin 目录已在用户终端 PATH 上
- **THEN** 该目标的「不在终端 PATH 上」状态 SHALL 为假

#### Scenario: 平台不自动改用户 shell

- **WHEN** 平台判定某对象处于「不在终端 PATH 上」状态
- **THEN** 平台 MUST NOT 自动改写用户的 shell profile、MUST NOT 自动创建 symlink；MUST NOT 因此阻断该对象的安装或使用

#### Scenario: 显式暴露动作受 REQ-PCI-11 约束

- **WHEN** 用户显式触发「暴露到终端 PATH」动作
- **THEN** 该动作 SHALL 按 REQ-PCI-11 的边界执行；MUST NOT 改写用户的 shell 配置文件

### Requirement: 更新落点与被更新对象一致（REQ-PCI-09）

更新 SHALL 复用与安装同一条异步作业链（作业标识、轮询、durable 状态的语义见 REQ-PCI-05），但**落点**必须与被更新对象当前所在的位置一致。

更新 SHALL 把 CLI 装到**被更新对象当前所在的前缀**：已安装且其命令在宿主 PATH 上时，更新 SHALL 装回**同一个前缀**；已安装但其命令不在宿主 PATH 上时，平台 SHOULD 从**已安装包的真实路径反推 npm 前缀**（形如 `<prefix>/lib/node_modules/<pkg>` ⇒ 其 `lib` 目录的父目录即该前缀），MUST NOT 把更新改装到别的前缀（MUST NOT 造成「更新装到 A、用户跑的还是 B」）。前缀候选与可写性的判定口径见 REQ-PCI-03。

#### Scenario: 已安装且在 PATH 上时更新装回同一前缀

- **WHEN** 用户对某对象触发更新、且该对象的命令当前在宿主 PATH 上
- **THEN** 平台 SHALL 把该对象更新到**它当前所在的前缀**；MUST NOT 改装到另一个前缀

#### Scenario: 已安装但不在 PATH 上时从已安装包真实路径反推前缀

- **WHEN** 用户对某对象触发更新、且该对象已安装但其命令不在宿主 PATH 上
- **THEN** 平台 SHOULD 从该对象已安装包的真实路径反推其 npm 前缀并把更新装到该前缀；MUST NOT 因为「PATH 上找不到该命令」就改装到别的前缀

#### Scenario: 更新不得改装到别的前缀

- **WHEN** 平台执行某对象的更新
- **THEN** 平台 MUST NOT 把更新装到与被更新对象当前所在位置不同的前缀；MUST NOT 造成「更新装到 A、用户实际运行的仍是 B」的结果

### Requirement: 更新后版本校验与 Kimi 更新（REQ-PCI-10）

更新完成后，若该命令的版本仍与更新前相同，平台 SHALL 明确报告「命令仍是旧版」（诊断 SHALL 说明更新前后版本相同并给出可行动提示），MUST NOT 静默判成功、MUST NOT 把「版本未变」呈现为更新完成。

Kimi Code 的更新 SHALL 沿用官方安装脚本的**原地替换**通道（见 REQ-PCI-05），MUST NOT 让 Kimi Code 走 npm 安装路径；且 SHALL 经装后验证（见 REQ-PCI-05 的装后重新探测，以及 REQ-PCI-01 的来源告知与用户确认）确认更新后的命令真实可用。

#### Scenario: 更新后版本未变须明确报告

- **WHEN** 更新执行完成、但探测到的命令版本与更新前相同
- **THEN** 平台 SHALL 明确报告「命令仍是旧版」并给出可行动诊断；MUST NOT 静默判成功、MUST NOT 把该结果呈现为更新已完成

#### Scenario: Kimi 更新沿用官方脚本并经装后验证

- **WHEN** 用户对已安装的 Kimi Code 触发更新
- **THEN** 平台 SHALL 重新执行官方安装脚本完成原地替换，MUST NOT 走 npm 安装路径；SHALL 经装后验证确认更新后的命令真实可用；MUST NOT 在用户未确认来源时执行远程脚本

### Requirement: 显式把已装 CLI 暴露到用户终端 PATH（REQ-PCI-11）

平台 SHALL 提供一个**由用户显式触发**的动作，把已安装的 CLI 暴露到用户终端（登录 shell）的 PATH 上，使新开的终端里该命令可直接运行。该动作 MUST 仅在用户明确触发时执行，MUST NOT 自动发生；MUST NOT 修改用户的 shell 配置文件（MUST NOT 写 `.bashrc` / `.zshrc` / 等价文件，MUST NOT 改写 PATH 环境变量）。

该动作 MUST 只在**已存在、真实可写、且确实在用户终端 PATH 上**的目录里创建**单个**软链（指向已安装二进制）；MUST NOT 在用户终端 PATH 之外的目录创建软链；目录内已有**同名但指向别处**的残留时 MUST 跳过该目录，MUST NOT 覆盖用户已有文件。

创建软链后 SHALL **重验**该命令是否真的落在用户终端 PATH 上且软链可解析到目标；重验失败 MUST 清理本次新建的软链并**如实报告**失败原因。未安装的对象 MUST 被明确拒绝；全部候选目录不合格时 MUST 报错并给出各候选目录的不合格原因，MUST NOT 静默成功。

#### Scenario: 显式触发才动作

- **WHEN** 用户触发「暴露到终端 PATH」动作
- **THEN** 平台 SHALL 执行暴露动作并如实呈现结果；MUST NOT 在用户未触发时自动创建软链或改写任何用户文件

#### Scenario: 不改用户 shell 配置文件

- **WHEN** 平台执行暴露动作
- **THEN** 平台 MUST NOT 写 `.bashrc` / `.zshrc` / 等价文件，MUST NOT 改写 PATH 环境变量；只 MAY 在合格候选目录里创建单个软链

#### Scenario: 只在终端 PATH 上的可写目录建软链

- **WHEN** 某候选目录不存在、真实不可写、或不在用户终端 PATH 上
- **THEN** 平台 MUST 跳过该目录并考察下一个候选；MUST NOT 在用户终端 PATH 之外的目录创建软链；全部候选不合格时 MUST 报错并给出各候选的不合格原因

#### Scenario: 同名残留不覆盖

- **WHEN** 候选目录内已有同名文件 / 软链、但它不指向本次安装目标
- **THEN** 平台 MUST 跳过该目录并说明原因；MUST NOT 覆盖用户已有文件

#### Scenario: 暴露后重验

- **WHEN** 软链已创建
- **THEN** 平台 SHALL 重验该命令是否真的落在用户终端 PATH 上且软链可解析到目标；重验失败 MUST 清理本次新建的软链并报错；MUST NOT 在重验失败时报成功

#### Scenario: 未安装对象不可暴露

- **WHEN** 用户对未安装的对象触发暴露动作
- **THEN** 平台 MUST 明确拒绝并说明先安装再暴露；MUST NOT 创建任何软链

### Requirement: 登录 shell 探测到的 agent 专属环境变量补进子进程（REQ-PCI-12）

平台 SHALL 在启动期以**交互式登录 shell** 探测 agent 专属环境变量（最少覆盖 `PI_CODING_AGENT_DIR`、`CODEX_HOME`、`CLAUDE_CONFIG_DIR`），并只在**缺失时**把它们补进平台发起的子进程 env：调用方显式覆盖层与平台进程自身 env 优先，MUST NOT 用探测值覆盖它们。探测 SHALL 有 **2s 硬上限**且在后台异步执行，超时 / 失败 / 不可用 MUST 不阻塞启动，MUST NOT 让安装、装后探测、终端 PATH 判定或索引验收失败。注入 SHALL 在两条命令汇聚点都生效，且只影响子进程环境。

#### Scenario: 启动期探测不阻塞启动

- **WHEN** 平台启动并登记启动期 PATH overlay
- **THEN** 登录 shell 环境探测 SHALL 在后台异步执行，超过 2s 硬上限即放弃并使用空结果；MUST NOT 阻塞启动流程，也 MUST NOT 让登记调用等待探测完成

#### Scenario: 不假定用户 shell 变量对平台可见

- **WHEN** 平台进程不是登录 shell，而用户在 shell 配置里设定了 `PI_CODING_AGENT_DIR` / `CODEX_HOME` / `CLAUDE_CONFIG_DIR`
- **THEN** 平台 MUST NOT 假定这些变量可见；被拉起的 `pi` / `codex` / `claude` 若拿不到这些变量会去读默认配置位置 ⇒ 平台 SHALL 按本 requirement 的优先级补齐，MUST NOT 杜撰或猜测配置目录

#### Scenario: 缺失的 agent 变量被补进子进程

- **WHEN** 平台即将拉起子进程，调用方覆盖层与平台进程 env 都不含 `PI_CODING_AGENT_DIR`，而登录 shell 探测到了该变量
- **THEN** 该子进程 env SHALL 含 `PI_CODING_AGENT_DIR` 及其探测值

#### Scenario: 平台已有值优先

- **WHEN** 平台进程自身 env 已含 `CODEX_HOME`，登录 shell 探测也返回了该变量
- **THEN** 子进程 SHALL 使用平台进程已有的值；MUST NOT 用探测值覆盖它

#### Scenario: 调用方显式覆盖优先

- **WHEN** 调用方为某次子进程显式传入 `CLAUDE_CONFIG_DIR` 覆盖值
- **THEN** 子进程 SHALL 使用调用方给出的值；MUST NOT 用探测值覆盖它

#### Scenario: 探测失败不阻塞任何流程

- **WHEN** 登录 shell 探测超时、失败或不可用（未设 `SHELL` / shell 不可执行 / 输出不含标记行）
- **THEN** 平台 MUST NOT 注入任何 agent 专属变量、MUST NOT 生成空值变量；安装、装后探测、终端 PATH 判定与索引验收 SHALL 照常进行，MUST NOT 因探测失败而失败；MUST NOT 阻塞启动

#### Scenario: 两条命令汇聚点都生效

- **WHEN** 命令分别经进程拉起通道与同步捕获通道发起
- **THEN** 两条通道的子进程 SHALL 都得到同样的 agent 专属变量注入；MUST NOT 只在其中一条通道生效

#### Scenario: 注入不改变终端可见性语义

- **WHEN** 平台把 agent 专属变量补进子进程 env
- **THEN** 该注入 MUST NOT 被当作「把 CLI 暴露到用户终端 PATH」；MUST NOT 改变「不在终端 PATH 上」的判定，MUST NOT 触发显式暴露动作，也 MUST NOT 改写任何用户 shell 配置文件
