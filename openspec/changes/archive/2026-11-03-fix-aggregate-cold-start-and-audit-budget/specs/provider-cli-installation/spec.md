## ADDED Requirements

### Requirement: 安装与更新的适用对象与安装源（REQ-PCI-01）

平台 SHALL 支持对四家 provider 与 codegraph 提供安装 / 更新能力：Claude Code、Codex、Pi、codegraph 走 **npm 包**，**Kimi Code 走官方安装脚本**（非 npm 包）。各对象的安装源与命令名 SHALL 明确且可查。

对**执行远程 shell 脚本**的对象（Kimi Code），界面 MUST 在执行前明确告知「将执行来自 code.kimi.com 的安装脚本」；MUST NOT 让用户在不知来源的情况下触发远程脚本执行。

#### Scenario: 五类对象的安装源各自明确

- **WHEN** 用户查看任一对象的安装入口
- **THEN** 系统 SHALL 给出该对象对应的安装源与命令名：Claude Code → npm 包 `@anthropic-ai/claude-code` / 命令 `claude`；Codex → `@openai/codex` / `codex`；Pi → `@earendil-works/pi-coding-agent` / `pi`；codegraph → `@colbymchenry/codegraph` / `codegraph`；Kimi Code → 官方脚本 `https://code.kimi.com/kimi-code/install.sh` / `kimi`；MUST NOT 让 Kimi Code 走 npm 包路径

#### Scenario: Kimi 执行前告知脚本来源

- **WHEN** 用户即将触发 Kimi Code 的安装
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

### Requirement: npm 镜像源与安装执行形态（REQ-PCI-03）

npm 安装 SHALL 默认使用阿里镜像 `https://registry.npmmirror.com`，并 SHALL 支持通过环境变量 `NPM_REGISTRY` 覆盖（供海外用户切换到官方 registry）。

npm 安装命令 SHALL 采用已验证形态：全局安装、`--prefix <可写前缀>`（避开 sudo）、`--omit=optional`、`--no-audit --no-fund`。

**一次安装 MUST 只包含一个包**：MUST NOT 把多个包合并进同一次 `npm install -g` 调用。

#### Scenario: 默认阿里镜像且可覆盖

- **WHEN** 执行 npm 安装
- **THEN** 安装 SHALL 使用阿里镜像作为 registry；环境变量 `NPM_REGISTRY` 有值时 SHALL 以其为准；MUST NOT 写死单一镜像

#### Scenario: 安装参数形态固定

- **WHEN** 平台执行 npm 安装
- **THEN** 命令 SHALL 含全局安装、`--prefix <可写前缀>`、`--omit=optional`、`--no-audit --no-fund`；MUST NOT 使用需要提权的默认全局前缀

#### Scenario: 一个包一次安装

- **WHEN** 需要安装多个包
- **THEN** 平台 SHALL 为每个包单独发起一次安装；MUST NOT 把多个包合并进同一次 `npm install -g`

### Requirement: 最新版本判定与 durable 缓存（REQ-PCI-04）

「有新版」的判定 SHALL 以 npm registry 的 `dist-tags.latest` 为数据来源，并以**精确版本比较**（`major.minor.patch` + 可选预发布后缀）比对本地版本；预发布版本 SHALL 视为**低于**对应的正式版本。

查询结果 SHALL 缓存 **24 小时**且缓存 SHALL 是 **durable**（进程重启后仍可复用）。缓存条目 SHALL 至少记录包名、最新版本与查询时间戳。

非 npm 包（Kimi Code）SHALL NOT 依赖 registry 判定「有新版」。

#### Scenario: 用 registry latest 与本地版本精确比较

- **WHEN** 判定某 npm 对象是否有新版
- **THEN** 系统 SHALL 取 registry 的 `dist-tags.latest` 并与本地版本做精确比较；MUST NOT 用字符串前缀或浮点近似比较

#### Scenario: 预发布版本低于正式版

- **WHEN** 本地版本是预发布版而最新版本是对应的正式版
- **THEN** 判定 SHALL 结论为「有新版」；预发布 MUST NOT 被视为不低于正式版

#### Scenario: 查询结果 24 小时内复用

- **WHEN** 距上次成功查询未超过 24 小时
- **THEN** 系统 SHALL 直接使用缓存的版本结论，不发起新的 registry 查询

#### Scenario: 缓存跨进程重启仍在

- **WHEN** 进程重启后再次判定是否有新版
- **THEN** 系统 SHALL 仍能取到 24 小时内的缓存结论；MUST NOT 因重启而丢失缓存

#### Scenario: Kimi 不做版本判定

- **WHEN** 判定 Kimi Code 是否有新版
- **THEN** 系统 SHALL NOT 依赖 registry 作出版本判定；该对象 SHALL 只在未安装时提供「安装」入口

### Requirement: 异步安装作业与装完重新探测（REQ-PCI-05）

安装 SHALL 以**异步作业**执行，MUST NOT 阻塞 HTTP 请求。发起安装 SHALL 立即返回作业标识；查询接口 SHALL 轮询作业进度与结果。

作业状态 SHALL 是 **durable** 的：页面刷新或进程重启后 MUST NOT 丢失进度。

作业流程 SHALL 为：记录 `running` → 执行安装命令（npm 对象走 npm 安装；Kimi Code 走官方脚本）→ 安装成功后**重新探测**该对象的版本与可用性 → 记录 `completed` 与结果。

安装成功 MUST NOT 被当作能力已核验：安装后仍 SHALL 通过既有能力核验通道确认能力。

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

#### Scenario: 安装不等于核验

- **WHEN** 某对象安装成功
- **THEN** 该对象的能力状态 SHALL 仍需通过既有显式核验通道确认；MUST NOT 因安装成功而自动标记能力已确认，也 MUST NOT 自动触发核验探针

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
