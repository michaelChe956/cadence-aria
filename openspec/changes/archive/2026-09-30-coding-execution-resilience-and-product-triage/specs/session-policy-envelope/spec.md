# Spec Delta

## ADDED Requirements

### Requirement: blocked／rework 下的受限政策读取与重新授权（REQ-ENV-C2-POLICY）

当逻辑代码库 coding attempt 处于 blocked 或 rework 等待面时，产品面 SHALL 提供“读取政策”与“重新授权”操作。读取政策 SHALL 仅通过唯一 LC authority resolver 解析该 attempt 当前 SessionPolicyEnvelope 冻结的 policy_id、revision 与 digest，返回与 digest 一致的政策正文；resolver 无法唯一解析、digest 不一致或材料缺失时 SHALL fail-closed 并返回可诊断等待项，MUST NOT 回落到成员仓路径、项目级历史布局或绝对路径猜测。重新授权 SHALL 仅在用户确认后为该 attempt 的下一次 Coder／Reviewer 返修 run 签发 evidence 授权，授权 SHALL 绑定 attempt、role 与 policy digest，并仅在该 run 处于运行态期间有效。错误 role、过期或已替换的授权、错 attempt、digest 不匹配或越出 manifest 成员范围的读取 SHALL 继续被拒绝。系统 MUST NOT 把政策绝对路径或正文写入 context note 作为授权手段。

#### Scenario: 返修 coder 读取同 digest 政策

- **WHEN** 用户在 blocked 页面点击“重新授权”后发起返修，返修 Coder 在运行中读取政策
- **THEN** Coder 读到的政策正文 digest 与 envelope 冻结的 digest 一致，返修继续

#### Scenario: 页面读取政策

- **WHEN** 用户在 rework 等待面点击“读取政策”
- **THEN** 页面显示 resolver 返回的政策正文及其 policy_id、revision、digest，不暴露宿主绝对路径

#### Scenario: 错 role 或过期授权被拒

- **WHEN** 非被授权 role 的 run 或授权已过期／已被新授权替换的 run 发起读取
- **THEN** 读取被拒绝，attempt 保持停等并提示重新授权

#### Scenario: resolver 无法解析时停等

- **WHEN** resolver 无法唯一解析 policy 或 digest 与 envelope 不一致
- **THEN** 读取与重新授权均 fail-closed，系统落地核验等待项，不回落任何备用路径
