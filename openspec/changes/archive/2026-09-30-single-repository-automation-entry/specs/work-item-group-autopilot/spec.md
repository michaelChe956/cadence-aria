# Spec Delta

## MODIFIED Requirements

### Requirement: 人工 design 确认后的显式授权与范围（REQ-WIGA-01）

系统 SHALL 保持 story/design 的生成和确认由人完成，并仅在 design 经人工确认后提供自动化模式选择；默认 SHALL 为不自动化。自动化 enrollment SHALL 将 target、plan、session、source、provider 和绑定版本作为同一条精确身份合同持久化；target SHALL 明确区分单仓（Single-Repository）与逻辑代码库载体，且 SHALL 是 enrollment 载体身份的唯一权威：单仓 target MUST 携带与 issue 记录所属物理仓一致的真实 repository 身份，逻辑代码库 target MUST 同时携带 logical codebase 与 logical repository 身份；单仓 enrollment MUST NOT 携带或生成任何 logical repository 替身身份。用户执行“重新绑定／换代”时 MUST 明确提供目标 plan/session、source、target 和 provider，并携带当前 expected binding/policy version。成功操作 SHALL 写入新的绑定版本并使后续动作只接受当前版本；旧绑定、旧 child 与旧代事件 SHALL 保持只读可追溯。系统 MUST NOT 按最新 session、旧 issue 扫描或后台失败发现猜测接管，也 MUST NOT 将 Manual 已认领 attempt 升级为自动。未选自动化、旧数据缺少 enrollment、或单 target 身份不明确时 SHALL 保持既有手动流程；零/多 target、跨载体或身份不一致 SHALL fail-closed 拒绝自动授权。

#### Scenario: design 确认前不出现自动授权

- **WHEN** story 或 design 尚未由人确认
- **THEN** 不创建 enrollment、不自动准备 plan，原有人工生成与确认入口继续可用

#### Scenario: 默认不自动化

- **WHEN** 用户在 design 确认后选择不自动化或旧 issue 不含 enrollment
- **THEN** plan 准备、生成、advance 和 coding 启动仍按原有手动行为执行，不因自动化补偿扫描产生动作

#### Scenario: 单 target 精确绑定及重复提交

- **WHEN** 已确认 source 只对应一个精确 target（一个单仓物理仓，或逻辑代码库选择中的一个 logical repository），用户用相同选择键重复开启自动化
- **THEN** 系统仅返回同一 enrollment、同一绑定 plan/session 与策略修订；不同 source 或 provider/options 的冲突提交明确拒绝且不改绑

#### Scenario: 单仓 target 可被授权

- **WHEN** 单仓 issue 的记录指向一个已登记的物理仓，用户在 design 确认后选择自动化并提交服务端投影的单仓 target
- **THEN** 系统创建 enrollment，其 target 为该物理仓身份、不含任何 logical repository 身份，绑定版本 1 建立且后续自动链可以启动

#### Scenario: 错仓或跨载体 target 被拒

- **WHEN** Enable 携带的单仓 repository 身份与 issue 记录所属物理仓不一致、该物理仓不存在，或对单仓 issue 提交逻辑代码库 target、对逻辑代码库 issue 提交单仓 target
- **THEN** 系统返回 422 与需重新选择目标的提示，不写 enrollment、不启动 provider

#### Scenario: 多 target 不被自动纳入

- **WHEN** 已确认 source 或冻结的 plan/attempt 涉及多个目标仓库、目标不明确或与 enrollment target 不符
- **THEN** 系统拒绝自动授权或自动启动，绝不循环启动多个 attempt；既有逐 target 手工准入仍可用

#### Scenario: 显式重绑创建新绑定版本

- **WHEN** 当前 enrollment 处于等待恢复且用户提交匹配 expected version 的 plan、session、source、target、provider
- **THEN** 系统写入新的绑定版本，返回当前 enrollment 身份与下一步操作；旧绑定保持只读，后续编排只消费新版本

#### Scenario: 绑定版本过期拒绝

- **WHEN** 用户以过期 expected binding/policy version 或不匹配的 plan/session/target 提交重绑
- **THEN** 系统返回版本冲突/需刷新错误，不覆盖当前绑定、不启动 provider、不改变旧代历史

#### Scenario: 同一命令重放幂等

- **WHEN** 客户端以相同 command_id 和相同绑定负载重复提交同一重绑/换代操作
- **THEN** 系统返回首次操作结果，不创建第二绑定版本、不重复触发后续编排；同一命令异负载被拒绝

## ADDED Requirements

### Requirement: Enable 前完整角色链预检（REQ-WIGA-C5-PREFLIGHT）

自动化 target 投影与 Enable SHALL 在写入 enrollment 前，按该 enrollment 在真实链上将启动的角色逐一核验：plan author、plan reviewer、coder、code reviewer 与 internal reviewer，每个角色的 provider SHALL 按真实启动路径的同一派生规则确定，而非另设独立配置。逻辑代码库载体下，任一角色的 provider 属于 provider gateway 确定性不支持的组合（无 gateway 启动能力，或该 provider 被 gateway 路由策略静态阻断）时，系统 SHALL 以 HTTP 422 与稳定错误码 `automation_role_chain_unsupported` 拒绝，错误 SHALL 列出每个不支持的角色、provider 与原因，并提示用户更换 provider 配置或更换目标；拒绝 MUST NOT 写入或修改 enrollment、MUST NOT 启动任何 provider。单仓载体 MUST NOT 施加逻辑代码库 gateway 约束：provider 可用性校验通过的合法组合 SHALL 被授权。预检只判定确定性静态不支持，MUST NOT 探活或把账号池/网关的动态不可用判为静态不支持；Enable 之后运行期的动态不可用（503）SHALL 进入既有通知停等，系统 MUST NOT 自动切换 provider 或盲目重试。GET 投影与 Enable SHALL 对同一输入给出同一判定。

#### Scenario: 逻辑代码库不支持的 coder 在授权前被拒

- **WHEN** 逻辑代码库单 target issue 的 enrollment options 使 coder 派生为 gateway 无启动能力的 provider（如 Pi），而 reviewer 合法
- **THEN** 投影与 Enable 均返回 422 `automation_role_chain_unsupported`，错误指明 coder 角色与 provider，enrollment 未写入，provider 启动计数为 0

#### Scenario: 逻辑代码库被路由阻断的 provider 被拒

- **WHEN** 逻辑代码库 issue 任一角色派生为被 gateway 路由策略静态阻断的 provider
- **THEN** 系统在授权前返回 422 并列出该角色与阻断原因，不写 enrollment

#### Scenario: 单仓合法组合不误拒

- **WHEN** 单仓 issue 的 author/coder/reviewer 派生为逻辑代码库 gateway 不支持但本机可用的 provider
- **THEN** 投影与 Enable 成功，enrollment 写入，后续 plan 与 coding 按单仓直连路径启动

#### Scenario: 运行期动态不可用进入停等

- **WHEN** Enable 已成功，之后某角色启动时账号池或网关返回 503
- **THEN** 系统保留该失败事实并通过既有通知停等，展示恢复后重试操作；不自动切换 provider、不盲重、不回滚 enrollment

### Requirement: 单仓载体的无页面自动全链（REQ-WIGA-C5-CHAIN）

对单仓 target 的有效 enrollment，服务端 SHALL 沿单仓既有路径完成与逻辑代码库同等的自动链：幂等准备唯一 plan/session、启动一次生成、在 choice 与人工计划门停等、确认后独立 advance 到唯一 attempt Ready、经共用 typed StartCoding 单发首启、coding 运行至 `WaitingForHuman ∧ FinalConfirm` 并发出“待最终确认”信息；关闭所有页面与 coding socket 不影响推进。单仓链 MUST NOT 注入逻辑代码库 gateway、MUST NOT 创建逻辑代码库 manifest/selection/snapshot。prepare、Enrolled advance 与 StartCoding SHALL 各自复核 enrollment target 与当前权威载体一致：单仓 target 的 repository 身份 MUST 等于 issue 所属物理仓、plan 的全部工作项 MUST 无 logical target 归属、attempt MUST 无逻辑代码库 target 快照；逻辑代码库 target 的 attempt 唯一 target MUST 等于 enrollment 的 logical repository。任一不符 SHALL fail-closed 零副作用，并以既有等待项呈现需重新绑定。会话 owner 投影、驾驶舱等待项、plan 已确认与待最终确认信息 SHALL 对两种载体使用同一派生与展示，并显示当前 target 身份。

#### Scenario: 单仓关闭页面后自动跑到最终确认

- **WHEN** 单仓 issue 经人确认 design 后明确 Enable 自动化，用户随即关闭全部页面与 socket，并仅在 choice/计划门出现时从驾驶舱处理
- **THEN** 系统自动生成 plan、在人工门停等、批准后自动 advance 到 Ready 并 typed 首启，coding 完成后停在 FinalConfirm 并出现一次“待最终确认”信息；全程 provider 首启恰一次

#### Scenario: 单仓链拒绝错仓与旧绑定

- **WHEN** 单仓 enrollment 的 target 与 issue 当前所属物理仓不一致，或 advance/StartCoding 回执携带旧绑定版本
- **THEN** 该动作零启动、零写入 attempt，驾驶舱出现需重新绑定的等待项，当前绑定不被覆盖

#### Scenario: 逻辑代码库 attempt target 与 enrollment 不符

- **WHEN** 逻辑代码库 enrollment 的 plan 唯一 target 或 attempt 快照指向与 enrollment target 不同的 logical repository
- **THEN** Enrolled advance 或自动 StartCoding fail-closed 拒绝，不启动 provider

#### Scenario: 手动与关闭模式不被接管

- **WHEN** 单仓 issue 选择手动或关闭 enrollment 后后台补偿扫描运行
- **THEN** 补偿扫描不准备 plan、不 advance、不启动 coding，手动链行为与本 change 前一致
