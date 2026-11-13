# Spec Delta

## MODIFIED Requirements

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

npm 安装命令 SHALL 采用已验证形态：全局安装、`--prefix <可写前缀>`（避开 sudo）、`--omit=optional`、`--no-audit --no-fund`。

**一次安装 MUST 只包含一个包**：MUST NOT 把多个包合并进同一次 `npm install -g` 调用。

「`--prefix <可写前缀>`」中的「可写」SHALL 按**真实写入探测**判定：MUST 先按权限位预筛（目录对当前用户没有任何写位 ⇒ 直接判不可写），再在该目录内实际创建并删除一个唯一命名的临时文件以确认可写；MUST NOT 只看权限位即判「可写」。系统级前缀若真实写入探测失败，MUST NOT 被选为安装落点；平台 MUST NOT 为写入系统级前缀而提权（沿用 REQ-PCI-06 的「不自动提权」）。

#### Scenario: 默认阿里镜像且可覆盖

- **WHEN** 执行 npm 安装
- **THEN** 安装 SHALL 使用阿里镜像作为 registry；环境变量 `NPM_REGISTRY` 有值时 SHALL 以其为准；MUST NOT 写死单一镜像

#### Scenario: 安装参数形态固定

- **WHEN** 平台执行 npm 安装
- **THEN** 命令 SHALL 含全局安装、`--prefix <可写前缀>`、`--omit=optional`、`--no-audit --no-fund`；MUST NOT 使用需要提权的默认全局前缀

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

## ADDED Requirements

### Requirement: 「装了但不在终端 PATH 上」的显式状态（REQ-PCI-08）

平台 SHALL 为每个安装目标暴露一个显式的「不在终端 PATH 上」状态：该状态为真，当且仅当**平台能找到**该对象的 CLI（宿主 PATH ∪ 平台登记目录），而**该 CLI 所在 bin 目录不在用户终端（登录 shell）PATH 上**。该状态下界面 SHALL 显式呈现「平台可用、但用户终端 PATH 上没有」这一事实，MUST NOT 仅给一句自由文本让用户自行推断，也 MUST NOT 把它呈现成「未安装」。

平台取得用户终端 PATH 的事实取自对用户登录 shell PATH 的探测；该事实取不到时，MUST NOT 判该状态为真（不得给出无法证实的断言）。

平台 MUST NOT 为让 CLI 在用户终端可见而自动改写用户的 shell profile（MUST NOT 写 `.bashrc` / `.zshrc` / 等价文件，MUST NOT 创建 symlink）；处置内容 SHALL 以可复制的形式给出。该状态 MUST NOT 阻断该对象的安装、探测或使用。

#### Scenario: 平台可见但终端 PATH 没有时状态为真

- **WHEN** 平台在宿主 PATH ∪ 平台登记目录中能找到某对象的 CLI、而该 CLI 的 bin 目录不在用户终端 PATH 上
- **THEN** 该目标的「不在终端 PATH 上」状态 SHALL 为真，界面 SHALL 显式呈现；MUST NOT 把它呈现成「未安装」

#### Scenario: 终端 PATH 可见时状态为假

- **WHEN** 某对象 CLI 的 bin 目录已在用户终端 PATH 上
- **THEN** 该目标的「不在终端 PATH 上」状态 SHALL 为假

#### Scenario: 平台不自动改用户 shell

- **WHEN** 平台判定某对象处于「不在终端 PATH 上」状态
- **THEN** 平台 MUST NOT 自动改写用户的 shell profile、MUST NOT 创建 symlink；MUST NOT 因此阻断该对象的安装或使用

### Requirement: 更新落点与被更新对象一致（REQ-PCI-09）

更新 SHALL 复用与安装同一条异步作业链（作业标识、轮询、durable 状态的语义见 REQ-PCI-05），但**落点**必须与被更新对象当前所在的位置一致。

更新 SHALL 把 CLI 装到**被更新对象当前所在的前缀**：已安装且其命令在宿主 PATH 上时，更新 SHALL 装回**同一个前缀**；已安装但其命令不在宿主 PATH 上时，平台 SHOULD 从**二进制的真实路径反推 npm 前缀**（形如 `.../node_modules/<pkg>/...` ⇒ 其 `lib` 目录的父目录即该前缀），MUST NOT 把更新改装到别的前缀（MUST NOT 造成「更新装到 A、用户跑的还是 B」）。前缀候选与可写性的判定口径见 REQ-PCI-03。

#### Scenario: 已安装且在 PATH 上时更新装回同一前缀

- **WHEN** 用户对某对象触发更新、且该对象的命令当前在宿主 PATH 上
- **THEN** 平台 SHALL 把该对象更新到**它当前所在的前缀**；MUST NOT 改装到另一个前缀

#### Scenario: 已安装但不在 PATH 上时从二进制真实路径反推前缀

- **WHEN** 用户对某对象触发更新、且该对象已安装但其命令不在宿主 PATH 上
- **THEN** 平台 SHOULD 从该二进制的真实路径反推其 npm 前缀并把更新装到该前缀；MUST NOT 因为「PATH 上找不到该命令」就改装到别的前缀

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

