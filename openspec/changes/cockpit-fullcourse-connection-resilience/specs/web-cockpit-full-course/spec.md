## Purpose

把 3.7 自动执行驾驶舱从「对话门后半程」推到**全程**：前半程（生成发起 / Provider 配置 / 生成过程流式可视）补全进 cockpit，类型感知路由终态化（默认表演化），并为快照门投影加引擎终态守卫。本 capability 只覆盖前端呈现层与操作层（`web/src/`），引擎零改动（延续 3.7-C1 安全面）；跨会话批量确认为 P2 条件项（依赖 `workspace-session-connection-resilience` 的角色仲裁与广播）。

## ADDED Requirements

### Requirement: 生成入口与发起流（REQ-CFC-01）

cockpit SHALL 对 story/design 会话（及 plan 未开始态会话）提供「开始生成」入口与发起流：入口复用既有生成命令发送链（经既有 `useWorkspaceWs` 的 `start_generation`），并 MUST 维持 legacy 发起的全部语义——仅可发起阶段可达（stage 守卫）、连接未建立时禁发（连接状态守卫）、存在可恢复运行时走提示路径（recoverable run 语义）、发起后产生乐观条目（optimistic entry 语义）。系统 MUST NOT 另建第二条生成 WebSocket 连接、MUST NOT 引入第二份生成状态机（流式投影 SHALL 复用既有 workspace store 的 `stream_chunk`/`message_complete`/`stage_change` 投影链）。

#### Scenario: cockpit 内发起 story 生成

- **WHEN** 用户在 cockpit 打开一个未开始的 story 会话并点击「开始生成」（已满足 stage 与连接守卫）
- **THEN** 前端经既有 `start_generation` 发送链发起生成，产生乐观条目，生成开始后流式内容与阶段推进在既有三区投影中可见

#### Scenario: 不可发起阶段入口不可达

- **WHEN** 会话处于不可发起阶段（如生成已开始或门等待中）
- **THEN** 「开始生成」入口不可达或禁用，不提供绕过 stage 守卫的发送路径

#### Scenario: 连接未建立时禁发

- **WHEN** workspace WS 尚未建立连接时用户尝试发起生成
- **THEN** 发起动作被连接状态守卫拦截，不产生发送，界面呈现连接状态而非静默失败

#### Scenario: 不新建第二套生成链路

- **WHEN** 检查 cockpit 生成发起的实现
- **THEN** 不存在第二条生成专用 WebSocket，也不存在与既有 workspace store 并行的第二份流式/生成状态投影

### Requirement: Provider 配置入口（REQ-CFC-02）

cockpit SHALL 在会话创建/发起入口处提供 Provider 选择（含默认 Provider 记忆，存储于前端本地、不进引擎），并在会话可编辑阶段提供配置面，消除「建错只能另改」。配置面 SHALL 复用既有 `ProviderConfigPanel` 与既有 store 语义（author/reviewer 选择、reviewer 开关、permission、rounds）——单一状态源，MUST NOT 在 cockpit 侧维护第二份 provider 状态拷贝。会话进入运行后配置 SHALL 只读呈现当前 Provider，MUST NOT 伪装可修改（运行中改配置为引擎不支持语义）。

#### Scenario: 发起前选择 Provider

- **WHEN** 用户在 cockpit 发起生成前打开 Provider 配置
- **THEN** 可选择 author/reviewer Provider 及相关开关，选择结果随发起生效，全程无需离开 cockpit

#### Scenario: 默认 Provider 记忆

- **WHEN** 用户设置默认 Provider 后新建会话
- **THEN** 发起入口默认呈现该 Provider，减少「建错只能另改」的发生面

#### Scenario: 运行中配置只读

- **WHEN** 会话已进入生成运行阶段
- **THEN** 配置面只读呈现当前 Provider 与参数，不提供运行中修改入口

#### Scenario: 单一状态源

- **WHEN** 检查 cockpit 的 provider 配置实现
- **THEN** 配置读写均经既有 store，不存在第二份 cockpit 专用 provider 状态拷贝

### Requirement: 生成过程流式可视（REQ-CFC-03）

生成中的会话在 cockpit SHALL 有可辨识的运行态呈现：复用既有②执行流（timeline 阶段推进）与③下钻对话流（流式渲染、既有虚拟化）投影，并补齐**生成中运行态**——运行中状态标识（provider/阶段/已耗时）、空会话边界（尚无产物 ≠ 故障）、失败边界（provider 失败可辨识呈现，不静默）、完成边界（产物/门可达）。该呈现 MUST 仅消费既有 workspace WS 事件面，MUST NOT 私自扩展后端事件面；若既有事件面被证实不足，该子项 SHALL 移入 P2 契约重估而非在 P1 内私扩。

#### Scenario: 生成中流式可见

- **WHEN** 会话处于生成运行中且用户在 cockpit 观察
- **THEN** 流式内容持续渲染、阶段推进可见、运行中状态标识（含 provider 与已耗时）可辨识，用户无需切换 legacy

#### Scenario: 空会话边界不误报

- **WHEN** 会话已建立但尚未开始生成
- **THEN** 呈现空态引导（发起入口可达）而非故障态

#### Scenario: 失败可辨识

- **WHEN** provider 失败或生成中止
- **THEN** 失败在 cockpit 可辨识呈现（失败边界），不静默消失也不伪装进行中

#### Scenario: 不扩后端事件面

- **WHEN** 检查本 requirement 的实现
- **THEN** 流式与进度呈现只消费既有 WS 事件，未新增后端事件或字段

### Requirement: 形态路由终态化与默认表演化（REQ-CFC-04）

形态路由 SHALL 以类型感知路由机制为终态，判定优先级固定为：①显式设置 `aria.chat.cockpit = legacy` → legacy（C4 回滚通道）；②显式设置 `= cockpit` → cockpit；③未设置 + 已知类型（story / design / plan，**含未开始态**）→ **cockpit**（默认表演化——本条是对 2026-09-16 用户裁决行为面的变更，MUST 作为明示点呈报用户）；④未设置 + 未知/缺失 `workspaceType` → legacy（安全兜底）。以下守卫 MUST 全部保留：「空 plan 必须 legacy」的前提删除但其安全性守卫不删；跨会话归属守卫（仅当 store 的 `sessionId === 路由 sessionId` 时读取 `workspaceType`/timeline，前一会话首帧不得把当前会话错入另一形态）；未设置会话 MUST 首帧即定形态，MUST NOT 出现「先以 legacy 建立连接、收到状态后 unmount 并 close(1000) 重连」的中途形态切换。legacy 页面 MUST NOT 删除（legacy 退役属阶段 4 退役门）。按默认 legacy 编写的既有测试 MUST 随默认表演化同步迁移断言，MUST NOT 靠删除用例过关。

#### Scenario: 未设置 story 默认进 cockpit

- **WHEN** 用户未设置形态开关并打开 story/design 会话
- **THEN** 默认进入 cockpit，且生成入口与 Provider 配置可达

#### Scenario: 未设置空 plan 默认进 cockpit 且开始按钮可达

- **WHEN** 用户未设置形态开关并打开 timeline 为空（未开始）的 plan 会话
- **THEN** 默认进入 cockpit，「开始生成」入口可达（不再回退 legacy 找开始按钮）

#### Scenario: 显式 legacy 被尊重

- **WHEN** 用户显式设置 `aria.chat.cockpit = legacy`
- **THEN** 会话进入 legacy 页面，原流程可用（C4 回滚通道不受默认表演化影响）

#### Scenario: 未知类型安全兜底

- **WHEN** `workspaceType` 缺失或未知
- **THEN** 路由落到 legacy，不抖动、不误入 cockpit

#### Scenario: 跨会话首帧不串形态

- **WHEN** 用户从会话 A（plan）切换到会话 B（story），会话 B 的 `workspaceType` 尚未到达
- **THEN** 会话 B 不因 store 中残留会话 A 的类型/时间线数据而错误进入或闪烁另一形态

#### Scenario: 首帧定形态无中途重连

- **WHEN** 未设置用户打开未开始的 plan 会话并随后开始生成
- **THEN** 页面首帧即进入 cockpit，开始生成不触发页面形态切换导致的连接 unmount/close(1000)/重连

### Requirement: 快照门投影终态守卫（REQ-CFC-05）

投影层 SHALL 对引擎 stage/相位做终态守卫：当引擎 stage 已处于终态（如 completed/failed 等非门等待态）或门相位与快照投影失配时，快照门控件 MUST 呈现为不可操作（锁定呈现 + 原因说明），MUST NOT 提供 confirm/反馈的可达发送路径——前端 MUST NOT 再以「投影存在未关闭门」为判据放行可操作门。该守卫是前端投影行为：MUST NOT 改变引擎拒绝语义（引擎照旧拒绝），其作用是不再制造用户可撞的协议错误路径。引擎真实门开（stage 为门等待态且相位一致）MUST NOT 被守卫误伤。

#### Scenario: 终态会话的门锁定（0017 形态）

- **WHEN** 引擎 stage=completed 的停摆会话，其前端快照门投影仍存在
- **THEN** 门控件锁定不可操作并附原因说明，不存在可发出并撞 `INVALID_MESSAGE_FOR_STAGE` 的路径

#### Scenario: 相位失配锁定

- **WHEN** 快照投影呈现门等待但引擎相位与 human_confirm 消息判定不一致
- **THEN** 门控件锁定呈现，不提供 confirm 发送路径

#### Scenario: 真实门开不受误伤

- **WHEN** 引擎真实处于门等待（stage=human_confirm 且相位一致）
- **THEN** 门控件正常可操作，守卫不拦截

### Requirement: 跨会话批量确认（P2 条件项）（REQ-CFC-06）

收件箱跨会话多选批量确认 SHALL 作为**条件交付项**：仅当 `workspace-session-connection-resilience` 的角色仲裁与 session 级广播两机制落地后才在本 change 内交付；若 P2 主体滑出 3.8 窗口，本 requirement SHALL 整体 defer 至阶段 4 前夜并登记 defer-ledger（呈报用户可见），不构成 3.8 关闸前置。交付时：收件箱 SHALL 支持跨会话多选批量确认；其安全性 MUST 依赖服务端显式角色仲裁（批量连接不得与用户已开连接形成无差别可命令竞态）与 session 级广播（批量结果对其他连接可见）；每会话批量动作 MUST 恰好生效一次（幂等）。危险动作 MUST NOT 参与批量（沿用 3.7 Phase 4 批量语义）。

#### Scenario: 跨会话批量各自恰一次

- **WHEN** 两个真实会话各有开态门，用户跨会话多选执行批量确认
- **THEN** 每个会话的门各自恰好关闭一次，无重复动作、无 supersede 竞态

#### Scenario: 批量结果广播可见

- **WHEN** 批量确认在某一连接上执行完成
- **THEN** 其他已连接的 cockpit 连接可经 session 级广播观察到各会话门的关闭，无需刷新

#### Scenario: 条件不满足则 defer

- **WHEN** P2 主体未在 3.8 窗口内落地
- **THEN** 本 requirement 整体 defer 至阶段 4 前夜并登记 defer-ledger（呈报可见），3.8 不因它挂起关闸
