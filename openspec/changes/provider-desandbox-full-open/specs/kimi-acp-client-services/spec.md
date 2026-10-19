# 规范增量：Kimi 全角色 ACP 宿主能力

<!-- 物理来源：去沙箱设计三条实际位于 openspec/specs/kimi-acp-client-services/spec.md:80-150，另生命周期:37-78；原标题仅为 MODIFIED 匹配锚，正文完整取代旧语义。集成三条配套另见 kimi-code-provider-integration delta。 -->

## MODIFIED Requirements

### Requirement: 终端生命周期与资源边界（SHALL）

terminal SHALL 保留 created→running→(exited|killed)→released、幂等 kill/release、未知 ID 错误、pipe 双流与进程组管理。每会话并发上限默认 4、双流合计输出上限 1048576 字节、超限只标记一次；backpressure 不阻塞 RPC。全角色不再施加独立 120 秒硬上限，有效执行 deadline 不超过受信 invocation 总 deadline；真实活动 terminal 不得被 resume stall 或外层 idle 误杀，资源须登记回收。

#### Scenario: 正常链路
- **WHEN** create→输出流→wait_for_exit→release
- **THEN** 全链路 SHALL 成功，execution event 时间线可见真实命令与输出摘要；运行默认 pipe，只有真实 fixture 证明需要 TTY 才显式使用并审计，不静默换模式

#### Scenario: 清理
- **WHEN** 会话结束/取消时存在运行中终端
- **THEN** 系统 SHALL 幂等终止并回收登记的进程组与本轮资源；未知 detached writer 不得仅因 killpg 返回而宣称已静止，无归属资源如实报告并进入适用恢复停等

#### Scenario: 恰为上限不截断的确定性锁定
- **WHEN** 受控内存流恰好供给 MAX_TERMINAL_OUTPUT_BYTES 字节
- **THEN** 系统 SHALL 完整返回且不标截断，断言不依赖墙钟预算

#### Scenario: 超限截断只标记一次的确定性锁定
- **WHEN** 受控流供给 MAX_TERMINAL_OUTPUT_BYTES + 1 字节
- **THEN** 系统 SHALL 恰保留上限字节并仅标截断一次，断言不依赖机器负载

#### Scenario: 并发流共享预算不超额的确定性锁定
- **WHEN** stdout/stderr 并发供给的合计字节超过上限
- **THEN** 系统 SHALL 在任意交错下合计仅保留上限字节，使用原子共享预算与唯一截断标记

#### Scenario: 真实进程管道 smoke 保留
- **WHEN** 运行真实子进程管道 smoke
- **THEN** create、双流输出、wait_for_exit、release SHALL 全链路成功且 cap 生效，保留真实进程覆盖，不以纯内存断言替代

#### Scenario: 根治不改变行为语义
- **WHEN** 本次执行模型迁移完成
- **THEN** 除显式取消独立 120 秒上限及改为宿主执行外，系统 SHALL 保留并发上限、合计口径、截断原子性与幂等清理的确定性断言，不用加大测试超时或 ignore 掩盖故障

#### Scenario: 长 CHECK 在总预算内执行
- **WHEN** 已登记 terminal 的真实构建/测试超过 120 秒但仍在 invocation 总 deadline 内
- **THEN** 系统 SHALL 允许继续，不被独立 120 秒 timer、resume stall 或外层 provider idle 误杀；总 deadline 到达或用户取消仍终止，无活动工作且无进展时既有 stall 判定继续生效

#### Scenario: 资源边界与预算前置
- **WHEN** 请求第 5 个并发 terminal，或声明的 CHECK 超过 invocation 可用总预算
- **THEN** 系统 SHALL 分别拒绝超并发和在运行前暴露预算配置问题，不改默认并发/输出护栏，不静默将 CHECK 截为 120 秒

### Requirement: 终端执行策略与每命令封闭 grammar（SHALL）

Kimi terminal SHALL 在所有角色接受 ACP command+args 普通 argv 与已有 legacy shell 形式，不再以只读命令白名单、固定解释器或禁止管道/重定向限制执行。子进程 SHALL 继承宿主环境（HOME 为用户宿主 HOME、工具链 PATH 等随宿主可达），平台不构造、不覆盖子进程环境；权限模式只控制审批，不代替执行基线或 author 岗位工具规则。

#### Scenario: 白名单内只读命令
- **WHEN** Auto 请求 git status 或 rg
- **THEN** 系统 SHALL 在有效宿主上下文执行并返回真实结果，不再要求只读隔离根或封闭模板

#### Scenario: 越界与执行型 flag 拒绝
- **WHEN** 请求普通 shell 管道、重定向、解释器、构建、容器，或旧模板曾拒绝的正常 argv/flag
- **THEN** 系统 SHALL 按原生命令语义和当前审批执行，不因旧 grammar 拒绝；危险宿主能力如实归属于全开风险，fs API 的 no-follow 校验不投影为 terminal 沙箱

#### Scenario: 本地 fsmonitor 探针
- **WHEN** 仓库配置了 hooks/fsmonitor 且平台发起受信 Git 操作
- **THEN** 平台 SHALL 禁用这些仓库执行桥；该护栏不得被表述为限制 Kimi 自身可执行的全部命令或隔离其用户配置

#### Scenario: 全角色命令形态一致
- **WHEN** Author/Orchestrator、Splitter/Handoff、Reviewer 或 Coder 发起合法 ACP command+args 或 legacy shell
- **THEN** 系统 SHALL 不因角色套只读 grammar 或仅支持 bash -c；author 仍不配编辑代码文件内建工具，Reviewer 的执行受 B 档窗口采信，不等于撤销 terminal 能力

#### Scenario: 宿主环境可用但不冒充隔离
- **WHEN** 构造 terminal 环境
- **THEN** 系统 SHALL 继承宿主环境保留工具链、网络/容器与 owner 配置发现，不强制 HOME=worktree 或全局 Git 配置 /dev/null；凭据随宿主可达、安全由用户自管，平台不泄露凭据值，不声称同 UID 不可读取 owner 文件

### Requirement: 终端 OS 级隔离（auto 模式）（SHALL）

Kimi 全角色 terminal SHALL 在当前宿主用户下执行，不再使用应用层 bwrap、netns 或 no-new-privs，也不依赖 bwrap 探测成功。Auto/Supervised 只决定审批，不选择隔离路径。系统 SHALL 记录实际宿主非隔离执行，不把逻辑 root 或已验证 cwd 误记为物理只读/target-only；未知部署上下文不得静默回退。

#### Scenario: bwrap 可用
- **WHEN** bwrap 已安装而非 Coder 角色请求 terminal
- **THEN** 系统 SHALL 仍使用统一宿主路径，不挂只读 root、不禁网，Reviewer 不再由角色 policy 恒拒

#### Scenario: bwrap 可用（coding 可写 worktree，F-17）
- **WHEN** Coder 在目标 worktree 执行构建、测试或合法 Git 提交
- **THEN** 系统 SHALL 在宿主完成，不再构造读写 bind-mount/只读宿主组合；保留 cwd 与目标身份校验，但不宣称授权根外不可写

#### Scenario: bwrap 不可用
- **WHEN** 宿主无 bwrap 且满足 CLI/工具链与审批前提
- **THEN** Auto 与 Supervised SHALL 均可按各自审批语义执行，无需切 Supervised 绕过隔离失败，不返回旧 sandbox unavailable

#### Scenario: 规划层同步放开
- **WHEN** Kimi 用于 Planning/Author/Splitter/Handoff
- **THEN** terminal SHALL 同样宿主直跑，不能保留规划 bwrap 双路径或整体拒绝；author 编辑代码文件工具的岗位规则独立保留

### Requirement: 权限与路径沙箱（角色与 fs）（SHALL）

terminal/fs SHALL 接受当前权限审批与受信岗位合同；Reviewer 与非 author 角色不再因角色恒拒 Terminal/FsWrite，author 仅不配编辑代码文件内建工具。fs API SHALL 保留授权根内路径校验与 no-follow 原子语义，拒绝绝对越界、..、symlink 逃逸及新建文件父目录竞态；此 API 边界不限制宿主 terminal/MCP。Coding fs 授权根仍为目标 worktree，Reviewer 运行另满足 B 档窗口。

#### Scenario: reviewer 只读
- **WHEN** Reviewer 的 Kimi 发起 terminal/create 或授权根内 fs/write
- **THEN** 系统 SHALL 按当前审批允许能力，不再默认拒绝；运行依赖有效 B 档窗口，临时验证写入须恢复，Git/平台元数据违规整轮作废，不用只读工具面代替对账

#### Scenario: 越界拒绝
- **WHEN** fs 请求指向授权根外或通过 symlink、..、新建文件父目录竞态逃逸
- **THEN** 系统 SHALL 返回可区分错误且不执行该 fs 操作；terminal 的宿主可达范围不因此被宣称受同一 API 约束

#### Scenario: planning/author 可读不可写
- **WHEN** author 的 Kimi 发起 fs/read 或 terminal
- **THEN** 合法读取与普通 terminal SHALL 依权限执行，不受 bwrap/只读模板；author 不配编辑代码文件内建工具是岗位限制而非宿主只读

#### Scenario: planning/author 禁止写
- **WHEN** author 经 Kimi 编辑代码文件 fs 写服务请求写入
- **THEN** 系统 SHALL 按岗位规则拒绝；不得扩大为禁用 author terminal/MCP/skills，不影响经平台文档通道产出

#### Scenario: Splitter/Handoff 不得整体拒绝
- **WHEN** Kimi 的 Splitter/Handoff 发起读取、普通 terminal 或适用 fs 操作
- **THEN** 系统 SHALL 按实际岗位职责和权限决定，不按角色全拒；承担 author 职责的运行仅保留该岗位代码编辑工具限制
