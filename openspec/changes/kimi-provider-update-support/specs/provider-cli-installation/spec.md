## ADDED Requirements

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

## MODIFIED Requirements

### Requirement: 安装与更新的适用对象与安装源（REQ-PCI-01）

平台 SHALL 支持对四家 provider 与 codegraph 提供安装 / 更新能力：Claude Code、Codex、Pi、codegraph 走 **npm 包**，**Kimi Code 走官方安装脚本**（非 npm 包）。各对象的安装源与命令名 SHALL 明确且可查。

对**执行远程 shell 脚本**的对象（Kimi Code），界面 MUST 在执行前明确告知「将执行来自 code.kimi.com 的安装脚本」；MUST NOT 让用户在不知来源的情况下触发远程脚本执行。该告知与用户确认 SHALL 覆盖对该对象的**安装与更新**两种触发。

#### Scenario: 五类对象的安装源各自明确

- **WHEN** 用户查看任一对象的安装入口
- **THEN** 系统 SHALL 给出该对象对应的安装源与命令名：Claude Code → npm 包 `@anthropic-ai/claude-code` / 命令 `claude`；Codex → `@openai/codex` / `codex`；Pi → `@earendil-works/pi-coding-agent` / `pi`；codegraph → `@colbymchenry/codegraph` / `codegraph`；Kimi Code → 官方脚本 `https://code.kimi.com/kimi-code/install.sh` / `kimi`；MUST NOT 让 Kimi Code 走 npm 包安装路径

#### Scenario: Kimi 执行前告知脚本来源

- **WHEN** 用户即将触发 Kimi Code 的安装**或更新**
- **THEN** 界面 SHALL 明确告知「将执行来自 code.kimi.com 的安装脚本」；MUST NOT 静默执行远程脚本

### Requirement: 异步安装作业与装完重新探测（REQ-PCI-05）

安装 SHALL 以**异步作业**执行，MUST NOT 阻塞 HTTP 请求。发起安装 SHALL 立即返回作业标识；查询接口 SHALL 轮询作业进度与结果。

作业状态 SHALL 是 **durable** 的：页面刷新或进程重启后 MUST NOT 丢失进度。

作业流程 SHALL 为：记录 `running` → 执行安装命令（npm 对象走 npm 安装；Kimi Code 走官方脚本）→ 安装成功后**重新探测**该对象的版本与可用性 → 记录 `completed` 与结果。

**Kimi Code 的更新 SHALL 复用官方脚本通道**：重新执行官方安装脚本（脚本自行解析最新版本并原地替换本机二进制），MUST NOT 让 Kimi Code 走 npm 安装路径，且 SHALL 满足 REQ-PCI-01 的来源告知与用户确认。

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

#### Scenario: Kimi 更新走官方脚本

- **WHEN** 用户对已安装的 Kimi Code 触发更新
- **THEN** 平台 SHALL 重新执行官方安装脚本完成原地升级并重新探测版本；MUST NOT 走 npm 安装路径，且 MUST NOT 在用户未确认来源时执行远程脚本

#### Scenario: 安装不等于核验

- **WHEN** 某对象安装成功
- **THEN** 该对象的能力状态 SHALL 仍需通过既有显式核验通道确认；MUST NOT 因安装成功而自动标记能力已确认，也 MUST NOT 自动触发核验探针

## REMOVED Requirements

### Requirement: 最新版本判定与 durable 缓存（REQ-PCI-04）

- **Rationale**: 原条款中「非 npm 包（Kimi Code）SHALL NOT 依赖 registry 判定『有新版』」与「该对象 SHALL 只在未安装时提供『安装』入口」两条已被 REQ-PCI-07 取代 —— Kimi Code 官方以 npm 包 `@moonshot-ai/kimi-code` 发布（其 `dist-tags.latest` 与官方 `/latest` 端点同版本号），版本来源与安装来源分离后即可参与「有新版」判定。其余条款语义（精确版本比较、预发布低于正式版、24 小时 durable 缓存、缓存条目含来源包名与时间戳）由 REQ-PCI-07 逐条承接，无删减、无放宽。