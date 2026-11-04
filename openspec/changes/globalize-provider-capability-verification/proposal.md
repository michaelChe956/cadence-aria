# 提案：provider 能力核验全局化（一次核验，全项目可用）

> 状态：草案 —— owner 2026-11-04 已拍板（方案 A：全局；且本产品无历史数据 ⇒ 不做存量处理）。见文末「owner 决策记录」。
> 技术方案与决策记录见同目录 `design.md`；精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`。

## 背景

owner 2026-11-04 裁定：**provider 能力只需要核验一次，核验一次全局可用**。

现状是**逐逻辑代码库（per-LC）核验**：

- 写：`ProviderCapabilityStore::for_lc(paths, lc_id)` ⇒ 落到 `lc_scope_root(project_id, lc_id)/capabilities.json`（每个 LC 一份）
- 读：provider 选择门（聚合初始化的第二道门 L2「本 LC 能力已核验」）、gateway 准入、自动化授权预检、聚合初始化执行期，全部用 `for_lc` 读同一棵子树

痛点有三条，**互相咬合**：

1. **重复消耗**：能力核验要真跑一次现场探针（分钟级、消耗 provider 额度）。一个项目有 N 个逻辑代码库 ⇒ 用户要点 N 次「核验」，跑 N 遍探针，花 N 份额度，**得到的结论完全相同**（探针验的是本机的 provider CLI，不是某个 LC 的代码）。
2. **多仓布局下尤其刺眼**：块3 已经把产品定位成「一个聚合根 + 多个成员仓」的多仓形态，LC 数量天然可能不止一个；每加一个 LC 就多一轮「逐 LC 核验」，用户在能力面板上看到的却是**同一份事实**（本机四家 provider 能不能跑）。
3. **挡住冷启动**：聚合初始化的第二道门 L2 现在判「**本 LC** 能力已核验」。在全局核验之前，这道门对每个 LC 各判一次，且不通过时**不创建 operation** ⇒ 冷启动推进到聚合初始化就停住。

## 目标

| # | 目标 |
|---|---|
1 | **能力核验全局化**：写入一律落项目级全局 `capabilities.json`；读取只读项目级全局。一次核验 ⇒ 全项目（含全部 LC）可用 |
2 | **读时零歧义**：产品 SHALL NOT 维护、读取或提升任何 per-LC 能力记录；不存在「全局没有就去找某个 LC 的旧记录」这类回退分支 |
3 | **界面口径同步**：能力面板的「逐 LC 核验：会真实跑一次探测…」改为全局口径，并说清「一次核验，全项目可用」 |
4 | **门 L2 联动（本次最重要的用户面收益）**：能力核验改全局后，聚合初始化的 L2 自动变成「全局已核验」⇒ **四家 provider 全局核验一次（四家执行一次「一键核验全部」）之后，聚合初始化自动开始** ⇒ **用户不再需要往聚合根里放 `AGENTS.md` + `.claude/rules/language.md`** 这两个判定文件来解锁自动开始 |

> **口径澄清（必须如实，不得夸大）**：
> - 「全局核验后自动开始」= 冷启动推进到聚合初始化时，provider 选择门的 L2 通过 ⇒ operation 被创建并开始执行。这条路径**会真跑四条无中断命令、会消耗所选 provider 的额度**。
> - 聚合根里放 `AGENTS.md` + `.claude/rules/language.md` 仍是**另一条独立路径**（整条链零 provider turn、零额度、零探针）。本次全局化**不取消、不改写**这条路径；它只是不再成为「让聚合初始化自动开始」的**唯一**办法。
> - 「四家都核验过」不是必要条件：L2 只要在 owner 顺序里找到**第一家命令可用且已全局核验**的候选即放行；用户执行一次「一键核验全部」自然覆盖这个条件。

## 非目标

- 不改 capability 记录**内部结构**（`ProviderCapabilityRecord` / `ProviderActionMatrix` / 版本钉定 / 三态 / `record_verified_probe` 导入语义一律不动）
- 不改 capability **门本身**的准入强度：仍然只有 `Confirmed` 放行，`Denied` / `Unknown` 一律 fail-closed
- 不改「不点击不运行任何探针」契约：核验仍只由用户显式触发，聚合初始化失败**不得**自动发起核验
- 不改 capability 记录**以外的**任何 per-LC 作用域：manifest / `aggregate-policy.json` / 聚合初始化 operation / provider trust / root-recipe receipt / checkout / 聚合索引一律**继续按 LC 作用域**
- 不改 HTTP 路由形状（`/logical-codebases/{lc_id}/capability-revalidate` 等仍带 `lc_id`，它继续承担「哪个 LC 的面板发起」「404 守卫」与「探针执行上下文」的职责）
- 不新增 provider、不改 provider 优先级顺序（`Pi > KimiCode > ClaudeCode > Codex`）、不改 provider 安装机制
- 不做「已核验能力随 provider 换代自动失效」之类的新机制（既有版本钉定语义已覆盖）
- 不做存量 per-LC 能力记录的迁移、提升、兼容读取（owner 裁定：本产品无历史数据）

## 范围

| 能力 | 变更 |
|---|---|
`provider-capability-revalidation` | MODIFIED：重验证导入作用域、只读状态投影、批量核验入口的作用域口径、按钮区域说明；ADDED：能力记录的项目级全局作用域 |
`non-interrupt-repository-bootstrap` | MODIFIED：REQ-BOOT-03 / REQ-BOOT-07 中「本 LC 能力已核验」的判定口径改为项目级全局；ADDED：全局核验后聚合初始化自动开始 |

## 关键联动（必须在实施中兑现）

```
能力记录作用域：per-LC ──► 项目级全局
        │
        ├─► provider 选择门 L2：「本 LC 能力已核验」──► 「全局能力已核验」
        │        │
        │        └─► 四家全局核验一次 ⇒ L2 放行 ⇒ 冷启动推进到聚合初始化时
        │            operation 被创建并自动开始
        │            ⇒ 用户不再需要往聚合根放 AGENTS.md + .claude/rules/language.md
        │
        └─► gateway 准入（require_supported）：读同一份全局记录
                ⇒ 准入强度不变（只有 Confirmed 放行），只是作用域变大
```

## 风险与取舍

| 风险 | 处置 |
|---|---|
`StoreBackedProviderCapabilitySource::for_lc` 同时承担两件事（capability store 作用域 + root-recipe 凭据重核验通道的 operation store 作用域）⇒ 只改一半会出现「能力读全局、凭据仍按 LC」的半改状态 | 显式拆分职责：新增「全局 capability + LC 作用域 root-recipe 重核验通道」的构造器，删除旧的 `for_lc`（clean cutover，不留兼容构造器） |
`ProviderCapabilityRevalidateService::revalidate(project_id, lc_id, provider)` 的 `lc_id` 不只用于记录作用域，还用于探针执行上下文与证据根 ⇒ 盲目删参数会破坏探针 | 保留 `lc_id` 参数用于**探针上下文**，只把**记录与证据的落盘作用域**改为项目级；证据根随记录一起全局化（证据与记录同源） |
`for_lc` 被删除后，全仓（产品 + 夹具 + 集成测试）十余处播种/读取点会编译失败 | 逐处改为项目级 store；**只改作用域，不改断言语义**；夹具迁移清单写进 tasks |
既有断言依赖 per-LC 语义（如「capability 落在 LC 子树、legacy 项目级没有」）⇒ 语义变更后必然红 | 逐条改写为「capability 落项目级全局、LC 子树不再有 capability 记录」；policy / manifest 仍按 LC 的断言**保持不变** |
全局记录被多个 LC 共用 ⇒ 某 LC 的用户核验会改变其它 LC 的能力状态 | 这是本次变更**明确要的效果**（能力是本机 provider 的事实，不是某个 LC 的事实）；provider 换代/版本漂移仍由既有版本钉定语义逐家重验 |
全局化后 `capabilities.json` 落在 legacy 项目根（`projects/{project}/logical-codebase/`）⇒ 会不会被误判成「存在一个 legacy 逻辑代码库」 | 已核实：`store.rs:1106-1109` 的 legacy 判定**只看 `manifest.json`**，不因 `capabilities.json` 存在而成立 ⇒ 无影响 |

## owner 决策记录（2026-11-04）

| # | 决策 | 结论 |
|---|---|---|
| 1 | 作用域 | **方案 A：全局（项目级）**。provider 只需要核验一次 ⇒ 全局可用 ⇒ 不再逐 LC 核验 |
| 2 | 存量数据 | **本产品没有历史数据** ⇒ 存量 per-LC 能力记录不需要处理：不迁移、不提升、不做兼容读取分支 |
| 3 | 门 L2 联动 | 能力核验改全局后 L2 自动变成「全局已核验」⇒ 四家全局核验一次 ⇒ 聚合初始化自动开始 ⇒ **不再需要聚合根放 `AGENTS.md` + `.claude/rules/language.md`** |
