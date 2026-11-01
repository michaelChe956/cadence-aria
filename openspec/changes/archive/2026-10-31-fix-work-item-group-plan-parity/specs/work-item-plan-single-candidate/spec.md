# 规范增量：环诊断带来源与尝试可序列观测

## ADDED Requirements

### Requirement: 依赖环诊断必须携带边来源并结构化教学（REQ-WSC-24）

依赖环失败 MUST 以 typed witness 表达，逐跳标注来源（canonical_input 或 change_order）及涉及的仓序位次。平台 MUST 据此生成结构化教学指令交原授权 author 重组，指令 MUST 含保留全部 Inputs 与验收标准、不得改仓序的保护项。同一结构指纹重复出现时 MUST 升级为业务取舍并一次问清。

#### Scenario: 失败面带逐跳来源

- **WHEN** Final Compile 因依赖环失败
- **THEN** 失败记录 MUST 含逐跳 from、to、sources 与被逆的两个仓序位次，MUST NOT 只留自由文本

#### Scenario: 同一结构环重复则升级

- **WHEN** 重写后产出结构指纹相同的环
- **THEN** 平台 MUST 停止自动重写并升级为业务取舍，指纹判据 MUST 沿用 REQ-WSC-22

### Requirement: 候选生成尝试可序列观测（REQ-WSC-25）

平台 MUST 以追加写记录每次候选生成与编译尝试，MUST NOT 只保留最近一次。每条记录 MUST 含尝试序号、source revision 引用、阶段、稳定码、typed 种类、环结构指纹、已应用的自动修复与等价规范化清单、处置结论。由此 MUST 可计算首次通过率、机械自修收敛率与模型重写次数。

#### Scenario: 连续失败可逐次追溯

- **WHEN** 同一 issue 连续发生多次候选失败
- **THEN** 平台 MUST 能列出每次失败所在阶段、稳定码与应用的自动修复，MUST NOT 只剩最后一次

#### Scenario: 记录读写失败不阻断生成

- **WHEN** 追加记录时发生写入错误
- **THEN** 平台 MUST 把该错误记入运行日志，MUST NOT 因此使候选生成或确认失败
