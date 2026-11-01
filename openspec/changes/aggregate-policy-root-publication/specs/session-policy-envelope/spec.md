# Spec Delta

## ADDED Requirements

### Requirement: 权威根政策正文与冻结引用一致（REQ-ENV-12）

逻辑代码库 recipe 发布的每个最终 `AggregatePolicyArtifact` SHALL 在 manifest 的 canonical `provider_context_root` 下，以 `policy_id` 的安全相对路径提供与 artifact.policy_text 字节一致的正文；原始 UTF-8 字节 SHA-256 SHALL 等于 artifact digest，revision SHALL 与 `policy_id` 一致。发布不完整时 MUST NOT 完成最终 receipt；下游 SHALL 继续消费同一权威 policy 引用，不引入成员仓、内部 JSON 路径或桩正文的替代入口。

#### Scenario: envelope 引用可读取的权威根正文

- **WHEN** 逻辑代码库 recipe 已完成并请求 planning、coding、review 或 revision provider run
- **THEN** authority resolver 返回的 `policy_id`/revision/digest SHALL 与 scoped policy artifact 一致，provider 通过 canonical root 下 `policy_id` 可读取同一原始正文；envelope、gateway、admission 与 receipt 消费契约 SHALL 保持不变，readiness 仅增加 REQ-BOOT-06 定义的桩识别

#### Scenario: 发布目标缺失、冲突或摘要不一致

- **WHEN** recipe 最终发布时无法安全建立 canonical root 下的政策路径，路径存在不同正文、symlink/越界，或落盘正文与 artifact/digest/revision 不一致
- **THEN** 系统 SHALL 在签发最终 receipt 前失败关闭并留下可重试诊断，不替换用户文件、不从成员仓或旧桩正文补齐；下游现有身份与 digest 漂移拒绝行为 SHALL 保持不变

#### Scenario: 政策升级与历史冻结引用

- **WHEN** 用户显式重跑 recipe 并发布更高 revision 的最终政策
- **THEN** 新正文 SHALL 以新 `policy_id` 原样落盘且旧正文保留；新 envelope SHALL 使用当前权威 artifact 的 revision/digest，旧 envelope 的 resume SHALL 按既有指纹一致性契约判定，不把旧引用改写为新正文

#### Scenario: 单仓策略路径保持不变

- **WHEN** 传统单仓登记创建或启动 provider session
- **THEN** 既有单仓 policy、cwd、四命令和 `git_finalize` 契约 SHALL 保持不变，不要求 canonical LC root 的政策文件
