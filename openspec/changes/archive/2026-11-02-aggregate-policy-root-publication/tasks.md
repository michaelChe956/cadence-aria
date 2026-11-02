# Tasks

本文件只记录可跟踪的高层工作包；精确文件归属与接口见 [design.md](design.md) §8，具体步骤/命令由契约获批后的 `writing-plans` 写入 `cadence/plans/`。实施尚未开始，所有工作包保持未勾选。合计 **8–16 小时（1–2 人日）**，单包不超过 4 小时。

## 1. 政策事实与显式迁移

- [x] 1.1 完成根规则确定性聚合、原字节 SHA-256/revision、operation-owned 不可变发布输出与 canonical root/policy_id 正文发布，复用现有政策 store 的 identity/successor 契约；随包新增构造、完整正文、相同 operation 重入、新 operation 递增、冲突/symlink/非法 UTF-8/持久化失败的 TDD 测试，并更新既有根初始化设计的发布说明；验收为根正文与 artifact.policy_text 字节一致、digest 重算一致、失败不覆盖用户文件、重入不涨 revision。对应 REQ-BOOT-05、REQ-ENV-12；估时 2.5–4h。
- [x] 1.2 只在现有 readiness 原将完成的路径增加固定自举桩识别，命中后 rules/policy waiting、planning_ready=false、allowed_actions 含 Retry，detail/notice 明示初始化产品 API 与新 idempotency_key；随包以“桩 artifact + 旧有效 receipt + 其余四步完成”观察红→绿，再以真正文证明原判定仍 ready，保留原缺件/漂移原因与 GET 零副作用，并核对迁移说明准确指出 Completed 无 UI 启动按钮、dispatcher/UI 零改动。对应 REQ-BOOT-06；估时 1–2h。
- [x] 1.3 完成 producer-only 的已有根重跑 AggregatePreflight：仅同 LC/canonical root 的旧完成四命令 Allowed receipt 与 AGENTS/CLAUDE 字节归属证明允许复用入口，其余根/成员检查保持不变；随包增加“Completed 桩 LC 新 key 原撞 ownership_conflict→有 receipt 证明可推进”的 TDD，以及无 receipt、错 LC/root、Rejected、摘要不符和首次注册用户文件仍拒的负例，更新相应预检职责说明；验收为新 operation 可到达原 recipe 命令，而不是仅返回 API accepted。对应 REQ-BOOT-06；估时 1–2h。

## 2. 生产五步收口

- [x] 2.1 把最终发布接入 RuleAndMcpConfig 产物流的末命令既有收尾，调用 1.1 返回的最终 artifact digest 冻结 root receipt，发布/receipt 失败传播到原末步 Failed，撤掉重复和 warn-only 发布收口；随包新增生产依赖同构的正负接线及命令审计后/正文后/artifact 后/receipt 后中断恢复测试与必要模块说明，证明前三命令不前移政策、显式 Continue 不重复成功命令或增加同 operation revision、四条审计仍 Allowed/原序、最后一步成功时根正文/artifact/receipt 摘要一致，取消/恢复、索引独立、单仓与成员零写入边界不变。对应 REQ-BOOT-05、REQ-ENV-12；依赖 1.1，估时 2–4h。

## 3. 跨包真实验收

- [x] 3.1 全部实现汇合后统一完成真实 Claude Code fresh-LC 产品 E2E 与 Completed 存量桩 API 迁移：新工作区经登记→五步四命令→产品发布非桩政策→成员/聚合索引就绪→planning_ready=true，禁手工 policy/receipt/index seed 与旧 fixture 政策物化；旧桩先等待，再 POST 现有初始化 API（新 key）实际重跑，证明新 revision/receipt、根原生读取和三方字节 SHA 一致、旧证据不变及成员 Git 基线不漂移；以真实证据追加既有 E2E v1.2 台账的 #8 收口，不改 #10/#13 或冒称四家网关已解锁。对应 REQ-BOOT-05、REQ-BOOT-06、REQ-ENV-12；依赖 1.1/1.2/1.3/2.1，估时 1.5–4h。

1.1 与 1.3 可按独占文件并行；1.1 交付接口后，1.2 与 2.1 可并行。每个实现包自带定向测试和需要的说明；3.1 仅负责跨包系统验收，不接收前包未完成的测试/文档工作。四家 provider 的网关矩阵由 `lc-gateway-multi-provider` 在同一权威政策事实之上完成，不能用 fixture 物化替代本 change 的产品发布。
