# 提案：聚合冷启动可用化与审计预算口径

> 状态：草案 —— owner 决策已全部拍板（见文末）。技术方案见 `cadence/designs/2026-11-02_技术方案_块2_聚合冷启动与审计预算_v1.0.md`（owner 2026-11-02 审阅通过，2026-11-03 补决策）。
> 覆盖缺陷：B-1 provider 写死 / B-2 能力未核验无指引 / B-3 stderr 恒空 / B-4 codegraph 卡死 / K-1 累计记成单文件 / K-2 `.git/` 被计入 / K-3 预算偏小且调不动

## 背景

owner 2026-11-02 在**新机器**上连测多仓链路，聚合冷启动三步全红：

```
规则/政策     失败  aggregate_pre_check_failed: stage aggregate_initialization: no stderr summary
成员索引      失败  aggregate_pre_check_failed: stage aggregate_initialization: no stderr summary
聚合索引激活  失败  aggregate_index_failed:aggregate_index_degraded:codegraph_missing: codegraph: provider command not found: codegraph
```

同时 provider 准入被拒：`provider command not found: claude` —— 该机可能装了 pi / kimi / codex，产品一次都不试。

审计预算也把冷启动卡死：`audit snapshot budget exceeded at .../target/pms/log/error-2026-07-11.log: 68205791 cumulative bytes > max_bytes 67108864`。

**这七条同属「聚合冷启动」一条链，互相咬合**（provider 选谁 → 能力核验 → stderr → codegraph → 审计预算），一次做完最省。

## 目标

| # | 目标 |
|---|---|
1 | **provider 自动选择**：按 owner 顺序 `pi > kimi code > claude code > codex` 逐个探测，选第一个可用的；**全程只用一个**，结果落盘可查 |
2 | **能力未核验可行动**：区分「未核验」与「真实拒绝」，给出中文文案与可点击的核验入口 |
3 | **失败面不空洞**：任何失败面必须说清「是什么问题 + 怎么操作」，机器码降级到次要行 |
4 | **codegraph 缺失只报一次**：冷启动三连红 → 一条中文提示 + 安装命令 |
5 | **审计预算不再被存量体量压垮**：跳过 `.git/`，预算按本次变更量计，老仓可用 |
6 | **审计预算文案可行动**：中文 + 点名成员仓与大项 + 两条操作出路 |

## 非目标

- 不改 provider 适配层、网关信封、流式输入结构、工具策略
- 不自动触发 capability 核验（与「不点击不运行任何探针」契约冲突）
- 不改「运行中不许改 payload」的 fail-closed 语义（属块1）
- 不做 provider CLI 的自动安装（属块5 提案）
- 不做一键核验全部（属块3 票3）
- 不做 B-5 的 R1（重新初始化预览+备份）与 R2（workbench 透出 details）—— **owner 裁定不做，保持原样**
- 不改「已初始化」判定文件（owner 裁定：用户自己决定）
- 不做账本合并、快照实时化、inotify 事件驱动

## 范围

| 能力 | 变更 |
|---|---|
`non-interrupt-repository-bootstrap` | MODIFIED：聚合初始化 provider 选择；失败面字段化与文案单一来源；codegraph 前置探测与单次报错；审计预算口径与文案 |
`logical-codebase-aggregate-index` | MODIFIED：明确「无 LKG + 工具缺失 → `codegraph_missing` 致命且在前置探测阶段阻断；有 LKG 的 rebuild/sync 才走 Degraded」 |
`provider-capability-revalidation` | MODIFIED：能力状态可机读；失败面按三态分流 |

## 风险与取舍

| 风险 | 处置 |
|---|---|
选择器与 gateway 各写一套能力判定 → 迟早漂移 | 抽出**同一只读方法**（`launch_cell_state`），gateway 与选择器共用；准入**不放宽一个字节** |
provider 选择若放进 driver 字段 → 跨 LC 串味 | driver 是 state 级单例 → provider **必须走 request**，从 durable 记录读 → 「重试不换人」成为结构性保证 |
自动触发核验会阻塞分钟级且花 provider 额度 | **不做自动触发**；失败面引导用户显式点核验（与「不点击不运行任何探针」契约一致） |
`CasResolution` 等中间层丢原因 | 判别位穿透必须覆盖 `resolve()` 之前的全部层 |
`.git/` 整目录跳过 → 平台看不到成员仓 git 状态 | **owner 已确认接受**；既有守卫测试（断言 `.git/HEAD` 可观测）按新口径改写 |
元数据差分法理论上可能漏「大小与 mtime 都没变」的改动 | mtime 为纳秒精度，实际不发生；且**不用** inotify（跨平台成本 + 可能漏事件 → 削弱审计保证） |
流式读改造触及快照核心 | 与 K-3 在同一处改；先补单测再改 |
新增 `ProductStoreError` 变体造成底层反向依赖 | 把审计预算失败结构放到**独立小模块**（`json_store` 不反向依赖 `logical_codebase`） |

## owner 决策记录（2026-11-02 / 2026-11-03 已全部拍板）

| # | 决策 | 结论 |
|---|---|---|
1 | K-2 `.git/` 跳过范围 | **整个 `.git/` 跳过**；确认接受代价（平台看不到成员仓 git 状态；一个既有测试按新口径改写） |
2 | B-4 codegraph 致命 vs 降级 | **维持致命 + 前置探测 + 只报一次**（spec:99 早有规定） |
3 | Pi 优先的 skills 假成功 | **无需改动** —— `Cadence-skills` 的 `install.sh:401-404` 明确 `pi` 与 `Kimi Code` 都用 `~/.agents/skills`（= Aria `paths.rs:22` 的 `shared_skills_root`），**pi/kimi 读得到** |
4 | K-3 预算模型 | **立项 + 与本批一起实施**；实现方式 = **元数据差分法**（扫描两遍只记元数据，只对变更文件读内容算哈希；保留全仓两遍 → 「写到 allowlist 外」的检测能力完整保留） |
5 | 「已初始化」判定文件 | **不改** |
6 | 超大文件先读后计费 | **改**（流式读） |
7 | B-5 残项（R1/R2） | **都不做，保持原样** |
8 | B-2 失败面指向的按钮 | **单个「核验 X」**；一键核验全部留在块3 |
