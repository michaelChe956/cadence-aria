## MODIFIED Requirements

### Requirement: root recipe turn 零变更必须显式失败（REQ-BOOT-17）

root recipe 的 provider turn（PreCheck / RuleAndMcpConfig / OpenspecAndExamples）结束时，系统 SHALL 复用既有 receipt auditor 的变更事实，不新增第二套有无变更判定。工具报告结论 MUST 只取工具结果原文，不采信模型自述。明确 overall=failed MUST 优先于文件变化：步骤失败、后续步骤不推进、审计receipt仍保留，cause=root_recipe_script_failed；诊断携带provider、step和真实输出，不以zero_change冒充脚本原因。

没有失败报告时，零变更且工具报告overall=success SHALL 幂等成功；零变更且没有报告 SHALL 失败；有变更且没有明确失败报告 SHALL 保持既有成功语义，不放宽其他审计门禁。

零变更失败诊断 SHALL 同时携带失败事实（provider、step、零变更）和provider真实输出尾部。provider没有输出时只保留失败事实，不伪造兜底输出。turn输出超预算时 SHALL 保留头部和尾部，不得仅保留头部而丢失收尾报告。

#### Scenario: 零变更的 recipe turn 判失败
- **WHEN** receipt判为零动作且没有成功报告
- **THEN** step失败，operation记录诊断，后续步骤保持未启动；若有明确failed报告优先记脚本失败

#### Scenario: 零变更但脚本报告成功判成功
- **WHEN** receipt判为零动作且工具报告overall=success，没有failed
- **THEN** step成功，不以幂等零变更拒绝

#### Scenario: 脚本报告失败仍判失败
- **WHEN** 工具结果有overall=failed，无论是否产生文件变化
- **THEN** step失败、后续Pending、cause=root_recipe_script_failed，保留receipt和真实失败输出

#### Scenario: 有变更的 recipe turn 照旧成功
- **WHEN** receipt有实际变更且没有明确failed报告
- **THEN** step按既有成功语义处理，不放宽任何其他审计判据

#### Scenario: 既有成功判据不被改写
- **WHEN** 复核receipt动作判定
- **THEN** 复用既有auditor，仅新增明确脚本失败优先例外，不新增第二套变更判定

#### Scenario: 零变更诊断携带 provider 的真实输出
- **WHEN** 零变更失败turn产出真实输出
- **THEN** 诊断保留零变更事实和输出尾部，不只展示zero_change

#### Scenario: provider 无输出时诊断只保留失败事实
- **WHEN** 失败turn没有任何输出
- **THEN** 只保留失败事实，不生成无信息量或虚构输出

#### Scenario: 超长 turn 输出保留收尾
- **WHEN** turn输出超出诊断预算
- **THEN** 保留头部与尾部，不能整段丢失收尾报告

## ADDED Requirements

### Requirement: LC 配置阶段与索引构建分离

固定四条skill命令文本 SHALL 不变。LC追加上下文 MUST 要求rule-config关联脚本dry-run/apply透传--defer-codegraph-index；不得自行追加索引构建或等待。真实索引归既有后续aggregate_index，配置成功不得当作PlanningReady。

#### Scenario: 新聚合根配置
- **WHEN** 聚合根没有.codegraph
- **THEN** 先完成规则/MCP配置，不在provider turn内初始化索引；成功后交给后续索引步骤

#### Scenario: 旧Skills不认识参数
- **WHEN** 关联脚本拒绝--defer-codegraph-index
- **THEN** provider保留真实错误并停止，不退回会触发索引的无参数调用；平台仅对可识别的failed报告作明确失败判定，不宣称本次已重写缺报告兼容路径
