# Spec Delta

> 基线：`openspec/specs/coding-run-operability/spec.md` 现有 REQ-CRO-01 至 REQ-CRO-06。本 delta 只新增 REQ-CRO-07 与 REQ-CRO-08，不修改既有 requirement；既有 REQ-CRO-06 的等待项枚举不扩展，AMR 的恢复入口由 REQ-CRO-08 独立承接。

## ADDED Requirements

### Requirement: 转入人工恢复时的运行事实收尾（REQ-CRO-07）

coding runner 因运行期错误退出、attempt 被转入 awaiting_manual_recovery 时，系统 SHALL 在同一次转入中把该 attempt 名下仍处于运行中的 role run 记为失败，并携带与 attempt 相同的稳定原因码。收尾失败 SHALL 作为可见告警记录，MUST NOT 吞没，也 MUST NOT 阻止 attempt 转入人工恢复。收尾 MUST NOT 修改 unit 状态、启动 claim、active 指针或已落盘的完成事实。

#### Scenario: runner 死亡后 role run 不再显示运行中

- **WHEN** coding runner 在某 role run 运行期间因错误退出，attempt 被转入 awaiting_manual_recovery
- **THEN** 该 role run 的状态为失败并带有 attempt 的稳定原因码；运行历史面板不再将其显示为运行中；unit 与启动 claim 保持原值

#### Scenario: 收尾失败不阻止转入人工恢复

- **WHEN** 转入 awaiting_manual_recovery 时收尾 role run 本身失败
- **THEN** attempt 仍处于 awaiting_manual_recovery，系统留下可见告警，其余后续处理照常进行

### Requirement: 显式恢复入口与残留清理（REQ-CRO-08）

系统 SHALL 在 coding 驾驶舱为处于 awaiting_manual_recovery 的 attempt 提供可见、可操作的恢复动作，展示人工恢复原因及该动作会做什么；恢复动作 SHALL 复用既有显式恢复链路并携带当前 attempt 身份，对不处于 awaiting_manual_recovery 的 attempt 重复发出 SHALL 被拒绝且无副作用。显式恢复成功时，系统 SHALL 将该 attempt 下仍为运行中的残留 role run 记为已被取代，使其不再被展示为运行中，恢复后由重启的 runner 创建新的 role run，MUST NOT 复用被取代的 run；该清理对本 change 之前已遗留脏数据的 attempt 同样有效。恢复 MUST NOT 自动触发：服务重启、页面重连、自动续跑与依赖桥接均不得把 awaiting_manual_recovery 的 attempt 放回 Running。

#### Scenario: 驾驶舱提供恢复动作

- **WHEN** 用户打开一个 awaiting_manual_recovery 的 attempt 的 coding 页面
- **THEN** 页面显示人工恢复原因与恢复动作；点击后沿既有恢复链路回到 Running，页面不要求用户手工构造消息或重启服务

#### Scenario: 恢复清理残留运行中 role run

- **WHEN** 用户对一个 awaiting_manual_recovery 的 attempt 发出恢复动作，且该 attempt 下存在本修复前遗留的运行中 role run
- **THEN** 残留 role run 被记为已取代，attempt 回到 Running 并由新的 runner 创建新的 role run，运行历史中不存在永久运行中的旧条目

#### Scenario: 重复或错对象的恢复被拒

- **WHEN** 恢复动作发往已处于 Running、已完成或其他非 awaiting_manual_recovery 状态的 attempt
- **THEN** 系统拒绝该动作并不产生状态变化或新的 runner

#### Scenario: 无自动放行

- **WHEN** 服务重启、页面重连、自动续跑或跨 target 依赖桥接发生时存在 awaiting_manual_recovery 的 attempt
- **THEN** 该 attempt 仍保持 awaiting_manual_recovery，直到用户显式恢复
