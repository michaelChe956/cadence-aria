# 提案：SC 终局失败修复包与完整失败源自含教学

> 阶段：设计契约，尚未实施。Change：`deliver-sc-failure-repair-package`。本轮仅创建契约与设计文档，不修改 Rust、既有测试或已归档 change。

## Why

SC 作者链目前只保留第一轮失败摘要和最后一轮消息，缺少历轮完整阻断原因，也没有失败轮与 provider 原始输出的耐久绑定。compile／IR 预检失败尚未生成 source revision，结构门失败虽已存 source，却没有把原文交回教学 prompt；依赖 provider 会话记忆的返修因此不可自证。终局失败也没有可回读的修复包，使用者无法完整还原「哪版 source、哪些阻断、发过什么 prompt」。本 change 承接已归档 `repair-wig-fact-projections` 明确未闭合的 A4、A3：先建立可核验的逐轮证据与终局工件，再以完整失败 source 构造自含教学；缺项、读失败、超预算或落盘失败均显式可见，不再以日志或广播冒充耐久成功。

## What Changes

- **A4 优先**：建立 SC 专用的轮次—split run—原始输出绑定和逐轮证据日志；完整留存每轮 `blocking_reasons`，不以首／末轮摘要或展示截断面替代。
- 新增 `ArtifactPayload` 的 SC 失败修复包变体，沿现有版本化 artifact／`ArtifactRef`／`ArtifactUpdate` 体系发布。A4以同一版本文件/现有锁集中协调，写入、回读逐字校验、引用与发布回执写读成功后才广播；未发布耐久条目不作为公开已发布工件。两store公读保raw，A4非current，不替换普通工件；普通写者只经集中save窄保留A4、分配与save同锁，避免pending被stale整表覆盖，不新增高域overlay。
- **A3 依赖 A4**：compile／IR 预检类从所绑定 split run 的 `structured_output.json` 装载原始字符串，经既有确定性归一化恢复失败 source；结构门类按失败自身 `source_revision_ref` 回读。三类 prompt 均携带完整 `[previous_source]` 围栏块及可复验摘要。
- 最终教学 prompt 按 UTF-8 字节执行现有 `WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES = 262_144` 硬兜底，超限拒绝重驱，不截断 source、不省略注入、不新增启动。
- `failure_source_ref` 从恒 `None` 的死字段转为真实来源定位符；生产代码装载并核验其内容。历史缺项如实标记，不伪造 source revision。
- 两项已获集成 owner 明确批准的窄例外：**仅 SC** 为非 LC 直连作者轮也分配 split 身份；**仅 SC 修复包**使用严格持久化发布，不将现有 `update_artifact` 的返回值当作成功证明。

## 影响面

- 能力：仅 `work-item-plan-single-candidate`，新增 REQ-WSC-33／34／35；现有 REQ-WSC-06／12／32 不重复修改。
- 运行面：SC 作者原轮、教学重驱轮、三类失败出口和终局失败工件。归一化、typed 资格、教学窗口 claim／真实启动分账、现有失败码与尝试身份保持既有口径。
- 持久化面：SC 逐轮证据与不可变 prompt 正文；新增 artifact wire 载荷；原始输出继续使用既有 split run 文件。`candidate-attempts.jsonl` 保持原格式与统计身份，不承担证据全文。
- 客户端：新增载荷须同步工件类型与只读呈现分支，不新增恢复动作、下载接口或决策协议。

## 取代关系

- 承接 `openspec/changes/archive/2026-11-13-repair-wig-fact-projections/design.md` §9 的 **A4 终局失败修复包、A3 完整失败 source 自含教学**，实施顺序固定为 A4 证据底座 → A4 严格工件 → A3 自含教学。
- 不重做该归档件已闭合的 A1 parse 五字段与 32,000 B 事实块、B 建议窄域投影、C 成员事实保底、D reviewer 无损紧凑化；不重开归档 change、不整体合并更早的 `--skip-specs` 提案。
- A1 的「parse 事实块省略不新增失败面」保持原义；本 change 的完整 source 装载及**最终 prompt**硬闸属于新增、独立的 A3 前置条件，不以事实块预算冒充全 prompt 预算。

## 非目标

- 本轮不实施代码；后续实施仍须契约审阅和已确认 Plan。
- 不新增模型调用、教学次数、修复预算、运行时开关、存储依赖或自动恢复；不改变 reviewer／人工门／Confirmed／advance 的语义。
- 不扩大非 SC legacy 逐段流的 split 记录行为；已有 LC 审计路径不被删除。
- 不承诺成功率、首 token 延迟或人工介入率改善；不补 IR 全量 typed 审计、命名域、多成员基线或前端 disposition 过滤等其它未闭合项。
