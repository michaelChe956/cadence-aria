# 设计：provider 能力核验机器级化（一次核验，整机可用）

> 本文件是 OpenSpec 层的**决策记录**：背景 / 决定 / 理由 / 代价 / 冲突裁决。所有落点均已在本仓（`.worktrees/bugfix-b-11-02`）核对过行号与语义；未核实的假设一律标注为「假设 + 核验方式」。
> 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`；本文件不写实施步骤。

## 0. 现状事实（决策的事实基础，全部已核对）

| 事实 | 位置 |
|---|---|
capability store 有两个构造器：`new(paths)`（项目级）与 `for_lc(paths, lc_id)`（LC 子树） | `src/product/logical_codebase/provider_capability_store.rs:744` / `:749` |
落盘路径由 `lc_scope_root(paths, project_id, &lc_id)` 派生：`None` ⇒ `projects/{p}/logical-codebase/`；`Some(lc_id)` 且非 legacy 别名 ⇒ `projects/{p}/logical-codebases/{lc_id}/` | `provider_capability_store.rs:883-888` + `src/product/logical_codebase/store.rs:97-110` |
store 模块文档自述落盘位置就是 `.aria/projects/{project_id}/logical-codebase/capabilities.json` | `provider_capability_store.rs:3-4` |
`ensure_bootstrap` 幂等：文件已存在即跳过、不覆盖（写两条 ClaudeCode + Codex bootstrap 占位行，矩阵全 Unknown） | `provider_capability_store.rs:867-882` |
provider 选择门 L2 读的是 **per-LC** store | `src/product/logical_codebase/provider_selection.rs:236-240`（`ProviderCapabilityStore::for_lc(paths.clone(), lc_id)` → `StoreBackedProviderCapabilitySource::with_store`） |
L2 与 gateway 共用同一只读判定函数 `launch_cell_state` | `provider_selection.rs:119-160`；`production_policy_resolvers.rs` 的 `require_supported` 同源调用 |
聚合初始化的 provider 选择门在创建期被调用，不通过 ⇒ **不创建 operation** | `src/web/handlers/aggregate_initialization/handlers.inc.rs:135-138`（`select_aggregate_provider_blocking`）→ `:139-142`（`provider_selection_api_error`） |
创建期还有一条更早的旁路：聚合根具备 `AGENTS.md` + `.claude/rules/language.md` ⇒ 判定「本链零 provider turn」⇒ 免过整个选择门 | `src/product/logical_codebase/aggregate_initialization.rs:229-230`（两个判定文件常量）、`:311`（`detect_skills_initialization`）、`:346-401`（`resolve_aggregate_provider_requirement`）；接入点 `handlers.inc.rs:104-121` |
gateway 准入读的是 **per-LC** capability source | `src/web/gateway_factory.rs:181-190`（`StoreBackedProviderCapabilitySource::for_lc`） |
gateway 的 durable probe writer 也写 **per-LC** | `src/web/gateway_factory.rs:237-243`（`durable_probe_writer_for_lc`） |
`StoreBackedProviderCapabilitySource::for_lc` 同时建立**两件事**：per-LC capability store + **per-LC** 的 root-recipe 凭据重核验通道（`AggregateInitializationOperationStore::for_lc`） | `src/product/logical_codebase/production_policy_resolvers.rs:232-247`（`for_lc`）、`:197-207`（结构体字段） |
重验证服务的 `lc_id` 承担**三件事**：记录作用域、证据根作用域、探针执行上下文 | `src/product/logical_codebase/provider_capability_revalidate.rs:245`（记录）、`:447-459`（`capability_evidence_root`）、`:264-282`（`ProbeScope.lc_id` → 探针 host scope） |
HTTP 路由带 `lc_id`，且 `lc_id` 承担 404 守卫职责 | `src/web/app.rs:187-199`；`src/web/handlers/logical_codebase_capabilities.rs`、`logical_codebase_capability_batch.rs` |
界面文案「逐 LC 核验：会真实跑一次探测，可能几分钟，消耗 provider 额度。」 | `web/src/components/lifecycle/ProviderCapabilityBatch.tsx`（`data-testid="capability-batch-scope"` 的那段；⚠ 该文件正被并行任务改动，行号会位移 ⇒ **按文案内容定位**） |

## 决策 1：能力记录的作用域唯一化 —— 项目级全局

> ⚠️ **本决策的作用域部分已被 owner 2026-11-05 的决策 13 取代**（项目级全局 → **机器级（整机）**）。本决策保留为历史记录；「单一作用域」「无第二条读路径」「clean cutover」三条原则**继续有效**，只是作用域边界外扩。

**决定**：capability 记录**只有一个作用域：项目级全局**。

- 写：全部落 `projects/{project_id}/logical-codebase/capabilities.json`（`ProviderCapabilityStore::new(paths)` 的既有落点）。
- 读：全部读同一份。
- **SHALL NOT** 存在第二条读路径（per-LC 子树）、**SHALL NOT** 存在「全局没有就去找某个 LC 的旧记录」这类回退/提升分支。

**理由**：

1. **事实的归属就是项目级**：capability 记录描述的是「本机上某家 provider CLI 能不能跑这个 action」，与某个逻辑代码库的代码内容**无关**。per-LC 副本是同一事实的多份拷贝，天然会漂移。
2. **成本不可接受**：一次真实验证要起 provider 进程、跑分钟级探针、消耗额度。逐 LC 复制 = 把同一份结论重跑 N 遍。
3. **门 L2 的联动收益**（决策 6）：只有单一作用域，L2 才能变成「全局已核验」，聚合初始化才能在全局核验一次后自动开始。

**代价**：

- 某 LC 的用户核验会改变其它 LC 的能力状态 —— 这是**要的效果**，不是副作用。
- provider 换代/版本漂移会让**所有** LC 同时需要重验 —— 同样是要的效果（本来就是同一份事实）。

## 决策 2：`ProviderCapabilityStore` 收敛为单一作用域构造器（clean cutover）

**决定**：

- `ProviderCapabilityStore` 删掉 `lc_id: Option<String>` 字段与 `for_lc(paths, lc_id)` 构造器，只保留 `new(paths)`。
- `capabilities_path` 固定为 `paths.logical_codebase_root(project_id).join("capabilities.json")`（= `projects/{p}/logical-codebase/capabilities.json`），**不再**经 `lc_scope_root` 派生。
- `lc_scope_root` 本身**不动** —— 它还被 manifest / policy / trust / receipt / operation / checkout 等一堆 LC 作用域 store 使用。

**理由**：留下 `for_lc` 就等于留下第二条作用域（哪怕没人调用），下一个改这块的人会再次得出「能力按 LC 分开存」。这是本仓反复出现的教训（`cadence/notes/2026-11-03_Legacy改写_产品侧_v1.0.md` 记录过「`for_lc` 与未作用域 store 按 doc 字节级等价」的等价性陷阱）。删除构造器 = 用类型系统把第二种作用域**编译期消灭**。

**代价**：全仓 `ProviderCapabilityStore::for_lc` 的调用点（产品 + 夹具 + 集成测试，逐一清点在决策 10 与 tasks 第 6 节）必须一次性迁完，**不留兼容别名、不留「deprecated 别名」**。

## 决策 3：写入 scope 逐点裁决

| # | 写入点（已核对） | 处置 |
|---|---|---|
W1 | `provider_capability_revalidate.rs:404-406`（`import_outcome` → `ProviderCapabilityProbeService::with_durable_writer`） | 改用项目级 store ⇒ 探针结果导入全局记录 |
W2 | `provider_capability_revalidate.rs:245`（`revalidate` 读既有记录判「已核验/缺哪几行」） | 改读全局记录（**读**点，但决定是否再跑探针 ⇒ 属写路径的幂等前置） |
W3 | `gateway_factory.rs:144-150`（`build_for_lc_material_prep` 的 `capabilities.ensure_bootstrap`） | 改项目级 ⇒ 自举占位行只写全局一份 |
W4 | `gateway_factory.rs:237-243`（`durable_probe_writer_for_lc`） | 改项目级 ⇒ 与 gateway 读的作用域同源 |
W5 | `coordinator_provider_turn.inc.rs:955`（聚合初始化执行期 `ensure_bootstrap`，**测试夹具**内） | 改项目级（该处属测试夹具，不是生产写路径） |

**`ensure_bootstrap` 的语义不动**：仍然「文件已存在即跳过、不覆盖」（`provider_capability_store.rs:867-882`）。全局化后它的效果是「项目级自举占位行只存在一份」，且仍然**不冒充真实证据**（矩阵全 Unknown）。

## 决策 4：读取 scope 逐点裁决

| # | 读取点（已核对） | 处置 |
|---|---|---|
R1 | `provider_selection.rs:236-240`（**L2**） | 读全局 ⇒ L2 语义从「本 LC 已核验」变为「全局已核验」 |
R2 | `gateway_factory.rs:181-190`（gateway 的 capability source） | 读全局（见决策 5 的职责拆分） |
R3 | `gateway_factory.rs:237-243`（durable probe writer 的读侧同源） | 与 W4 同 |
R4 | `provider_capability_revalidate.rs:352-353`（只读状态投影 `capability_snapshot`） | 读全局 ⇒ 状态面投影的是全局事实 |
R5 | `production_policy_resolvers.rs:215`（`StoreBackedProviderCapabilitySource::new`） | 已经是项目级 ⇒ **不动**（这是本次改造的「正确形态样板」） |
R6 | `production_policy_resolvers.rs:235`（`StoreBackedProviderCapabilitySource::for_lc`） | 见决策 5 |

**注**：读侧**只有**上述这些点。全仓不存在第二套 capability 判定（`launch_cell_state` 是选择器与 gateway 共用的唯一只读判定函数，`provider_selection.rs:119-160`），因此**无需**新增判定、不存在「选择器读全局、gateway 读 per-LC」这类漂移面。

## 决策 5：`StoreBackedProviderCapabilitySource` 的职责拆分（最容易漏的一处）

**背景**：`for_lc(paths, project_id, lc_id)`（`production_policy_resolvers.rs:232-247`）**同时**建立两件事：

1. capability store（**要**改成全局）
2. root-recipe 凭据的 durable Running 重核验通道 —— 内部是 `AggregateInitializationOperationStore::for_lc(paths, lc_id)`（**不能**改成全局：聚合初始化 operation 是 per-LC 的事实）

若只把 (1) 改掉而保留 `for_lc` 的名字与签名，会得到「能力读全局、凭据仍按 LC」的**半改状态**：名字继续骗人，下一个改这块的人仍会以为整件事是 per-LC 的。

**决定**：

- 删除 `StoreBackedProviderCapabilitySource::for_lc`。
- 新增一个职责单一、名字自证的构造器（建议名）：

  ```rust
  impl StoreBackedProviderCapabilitySource {
      /// 全局 capability 记录 + **按 LC** 的 root-recipe 凭据重核验通道。
      pub fn global_capability_with_lc_recipe_recheck(
          paths: ProductAppPaths,
          project_id: String,
          lc_id: String,
      ) -> Self
  }
  ```

- `new(paths, project_id)`（无 root-recipe 通道 ⇒ root-recipe 相位 fail-closed）与 `with_store(store, project_id)` 保持既有语义不变。

**理由**：把「哪个作用域」写进名字，是防止下一次作用域漂移的最低成本手段。root-recipe 通道**必须**留在 LC 作用域：它重验的是「本 LC 的聚合初始化 operation 是否处于 Running」，operation 本身就是 per-LC 的。

**代价**：调用点从「一个 `for_lc`」变成「一个名字更长但语义准确的构造器」；`gateway_factory.rs:181-190` 的 `match lc_id { Some => for_lc, None => with_store }` 两个分支**都要**走新的全局构造器（`None` 分支同样需要补上 root-recipe 通道吗？—— **不需要**：`None` 分支历史上就没有该通道，保持原语义 ⇒ `None` 分支仍用 `with_store` / `new`）。

## 决策 6：门 L2 联动 —— 「全局已核验 ⇒ 聚合初始化自动开始」

**背景（已核对的链路）**：

```
冷启动「一键推进」→ bootstrap/advance → 推进到 rules_policy
   → 准备派发 → start_lc_aggregate_initialization（handlers.inc.rs:87-104）
      ├─ 创建期判定 resolve_aggregate_provider_requirement（handlers.inc.rs:104-121）
      │    ├─ 聚合根有 AGENTS.md + .claude/rules/language.md ⇒ NotRequired
      │    │      ⇒ 免过整个 provider 选择门 ⇒ 自动创建 operation ⇒ 零 provider turn
      │    └─ 否则 ⇒ Required ⇒ 进入 provider 选择门
      │           → select_aggregate_provider_blocking（handlers.inc.rs:135-138）
      │              ├─ L1 命令可用（provider health 快照）
      │              └─ L2 能力已核验 ← 本次改的就是这一层（provider_selection.rs:236-240）
      │                   不通过 ⇒ provider_selection_api_error ⇒ **不创建 operation**（handlers.inc.rs:139-142）
      └─ 通过 ⇒ begin ⇒ spawn ⇒ 聚合初始化自动开始
```

**决定**：L2 读**项目级全局**能力记录（决策 4 / R1）。

**用户面收益（必须写进 proposal 的 Why 与 tasks）**：

- 四家 provider 执行一次「一键核验全部」（即全局核验一次）⇒ 之后**任意** LC 的聚合初始化，其 L2 都通过 ⇒ 冷启动推进到聚合初始化时 **operation 自动被创建并开始**。
- ⇒ **用户不再需要**往聚合根里放 `AGENTS.md` + `.claude/rules/language.md` 这两个判定文件才能让聚合初始化自动开始。

**必须如实声明的边界**（写进 spec 的场景与 proposal 的口径澄清，避免夸大）：

| 路径 | 触发条件 | 是否消耗 provider 额度 | 本次是否受影响 |
|---|---|---|---|
路径甲：零 provider turn | 聚合根有两个判定文件 | **不消耗**（不跑四条命令） | **不受影响**（判定文件与全局化正交，继续保留） |
路径乙：全局核验后自动开始 | 至少一家候选命令可用**且**其全局能力已 `Confirmed` | **消耗**（真跑四条无中断命令） | **本新增的可达路径** |

**理由**：这正是 owner 裁定的直接推论 —— 能力核验一次全局可用 ⇒ L2 只看全局 ⇒ 全局核验一次即解锁所有 LC 的聚合初始化自动开始。

**代价 / 风险**：路径乙会花掉所选 provider 的额度（跑四条命令）。这是「聚合初始化真跑」本身的固有成本，与能力核验无关；**SHALL NOT** 为了「省额度」把路径乙悄悄改成路径甲（那是判定前移那条变更的语义，本变更不碰）。

## 决策 7：探针证据根随记录一起全局化

**背景**：探针证据（`capability-evidence/boundary`）当前落在 **LC 子树**，与 `capabilities.json` 同一 `lc_scope_root`（`provider_capability_revalidate.rs:447-459` 的注释明写这一点）。记录里的 `probe_artifact_ref` 指向这些工件。

**决定**：证据根改为**项目级**（与全局记录同根）⇒ 一次全局核验的证据与它的记录**同处一个作用域**、互相可指。

**理由**：证据是记录的附属事实。记录全局、证据 per-LC，会出现「全局记录引用某个 LC 子树里的工件」这种跨作用域指针，审计时说不清证据到底属于谁。

**代价**：既有 per-LC 证据目录不再产生（**无历史数据 ⇒ 无需迁移**）。

## 决策 8：界面文案口径

**决定**：`ProviderCapabilityBatch.tsx:305` 的

```
逐 LC 核验：会真实跑一次探测，可能几分钟，消耗 provider 额度。
```

改为**全局口径**，且必须保留三项既有成本告知（真实探测 / 几分钟 / 消耗额度）：

```
全局核验：一次核验全项目可用；会真实跑一次探测，可能几分钟，消耗 provider 额度。
```

**理由**：REQ-PCR-06 要求按钮区域说明「核验是逐逻辑代码库的 / 真实探测 / 消耗额度 / 可能几分钟」。前三项保留，作用域那一项从「逐逻辑代码库」改为「全局（一次核验全项目可用）」。

**连带**：`ProviderCapabilityBatch.tsx:296-297` 的注释、三个前端测试断言（`ProviderCapabilityBatch.test.tsx:402`、`ProviderCapabilityCard.test.tsx:618`、`LogicalCodebaseManagementPanel.test.tsx:117`）同步改口径。

**MUST NOT** 顺手改这块按钮的其它行为（按钮位置、逐项进度、失败项重试、安装/更新引导 —— 均属块2/块3 已冻结的契约）。

## 决策 9：无历史数据 ⇒ 直接切，无迁移、无提升、无兼容分支

> ⚠️ 本决策在 2026-11-05 由 owner 再次确认（决策 16）：**旧的项目都不用管，没有旧的** ⇒ 不写任何存量分支。

**决定**：

- **不**写迁移脚本、**不**做读时提升（扫描各 LC 旧记录取该 provider 最新一条提升为全局）、**不**保留「全局没有就回退读 per-LC」的兼容分支、**不**保留任何 `for_lc` 别名。
- 存量 per-LC 能力记录：**既不迁移也不读取**。

**理由**（owner 2026-11-04 裁定）：本产品没有历史数据 ⇒ 任何迁移/提升/兼容分支都是**为不存在的问题**写代码，反而制造第二种作用域（提升分支要判定「哪条更新」，引入时间戳比较与并发写冲突，是纯负担）。

**代价**：一旦本次变更上线，per-LC 子树里**永远不会再产生** capability 记录；`logical-codebases/{lc_id}/capabilities.json` 成为「不应存在的路径」，出现即为缺陷。

## 决策 10：路由与 API 形状不变

> ⚠️ **本决策已被 owner 2026-11-05 的决策 14 取代**：capability 面改为**机器级路由**（不带 `project_id` / `logical_codebase_id`），旧路由删除、不留兼容 shim。本决策保留为历史记录。

**决定**：`POST /api/projects/{p}/logical-codebases/{lc_id}/capability-revalidate`、`GET .../capabilities`、`POST .../capabilities/revalidate-all` 等**路由形状不变**。

**理由**：`lc_id` 在这些路由里承担**三件与作用域无关**的职责：

1. 「哪个 LC 的面板发起的」；
2. `require_logical_codebase` 的 404 守卫（未知 LC ⇒ 404，不因能力全局化而放行未知 LC 的请求）；
3. 探针的**执行上下文**（`ProbeScope.lc_id` ⇒ 探针在哪个 LC 的世界里跑、审计归属哪个 LC）。

**代价**：路由带 `lc_id` 但记录作用域与它无关 —— 这是一个**必须写进 spec 的事实**，否则下一个改这块的人会误以为「lc_id 决定记录作用域」而把 store 又改回 per-LC。

## 决策 11：与既有契约的冲突点逐条裁决

| # | 既有契约（位置） | 冲突 | 裁决 |
|---|---|---|---|
C1 | `provider-capability-revalidation` 主 spec「导入写入的 capability store SHALL 与 gateway/admission 消费的作用域同一 LC 子树」 | 作用域从 LC 子树变项目级 | **MODIFIED**：改「同一项目级全局记录」 |
C2 | 同 spec「SHALL 提供 **per-LC** 的只读 capability 状态 GET 面」 | 投影内容变全局 | **MODIFIED**：GET 面形状不变，投影内容为项目级全局记录 |
C3 | 同 spec REQ-PCR-03「SHALL 提供 **per-LC** 的批量核验入口」 | 入口形状不变但效果全局 | **MODIFIED**：入口形状不变；核验结果写项目级全局记录 |
C4 | 同 spec REQ-PCR-06「按钮区域 SHALL 说明：核验是**逐逻辑代码库**的」 | 文案口径 | **MODIFIED**：改为全局口径，三项成本告知保留 |
C5 | `non-interrupt-repository-bootstrap` REQ-BOOT-07「L2 **本 LC** 的 `PlanningReadOnly.launch` 为 `Confirmed`」 | 判定作用域 | **MODIFIED**：改「项目级全局能力记录中该 provider 的 …」 |
C6 | 同 spec REQ-BOOT-07 场景「各自在**本 LC** 的 `PlanningReadOnly` 能力均为 `Confirmed`」 | 场景口径 | **MODIFIED** |
C7 | 同 spec REQ-BOOT-07 场景「但没有任何一家**在本 LC** 核验过能力」 | 场景口径 | **MODIFIED** |
C8 | 同 spec REQ-BOOT-03 场景「选中第一个命令可用且**本 LC** 能力已核验的 provider」 | 场景口径 | **MODIFIED** |
C9 | `lc-gateway-multi-provider` 讲 capability 证据但**不讲作用域** | 无冲突 | 不动 |
C10 | `logical-codebase-registration` REQ-REG-12 提「capability/gateway 条件」但**不讲作用域** | 无冲突 | 不动 |
| C11 | `provider-cli-installation`「安装成功 MUST NOT 被当作能力已核验」 | 无冲突（判定对象变了，规则没变） | 不动 |
| C12 | `store.rs:1106-1109`：legacy 判定**只看 `manifest.json`**，`capabilities.json` 不构成 legacy 代码库 | 全局化后 `capabilities.json` 落在 legacy 项目根 | **已核实无影响**：不需改 |
| C13 | 既有断言 `src/web/gateway_factory.rs:524` `build_for_lc_scopes_policy_and_capability_to_lc_subtree` 断言 `lc_root/capabilities.json` 存在且 `legacy_root/capabilities.json` 不存在 | 语义变更后必然红 | **改写断言**（policy / manifest 的按 LC 断言**保持不变**） |
| C14 | `tests/it_web/web_lc_capability_revalidate.rs:198-206`（拒绝路径不得写 capability 记录）、`:423-426`（播种旧 v2 字节） | 断言/播种指向 LC 子树 | **改指项目级全局路径**，断言语义（拒绝不写 / 旧字节不被迁移）不变 |

## 决策 12：失败处理与 fail-closed 保持

- 能力门强度**不变**：仍然只有 `Confirmed` 放行；`Denied` / `Unknown` / 记录缺失 / snapshot 漂移一律 fail-closed（`production_policy_resolvers.rs` 的 `require_supported` 语义一字不动）。
- 错误码与文案形态**不变**：`provider_capability_launch_not_confirmed: …`、`provider_capability_snapshot_mismatch: …` 等既有形态一字不改（块2 已把文案形态冻结为对外契约）。
- 写入失败仍是 `ProductStoreError` 上抛，**不**静默吞掉。
- 全局记录**读不到**（文件不存在）⇒ 视为「未核验」⇒ L2 不通过 ⇒ 失败面给出核验入口（既有 `aggregate_initialization_provider_needs_verification` 路径）—— 与今天 per-LC 记录缺失的行为**完全一致**。

## 13. 额外影响面（本设计核对代码时发现，超出 owner 给出清单的部分）

> 以下每条都已在本仓核对过行号。**这是本次变更最容易漏的部分**，tasks 必须逐条覆盖。

| # | 发现 | 为什么容易漏 | 处置 |
|---|---|---|---|
X1 | `StoreBackedProviderCapabilitySource::for_lc` **同时**承担 capability store 作用域与 root-recipe operation store 作用域（`production_policy_resolvers.rs:232-247`） | 名字只像「能力按 LC」，看不出它还挂着聚合初始化 operation store | 决策 5：职责拆分 + 改名自证 |
| X2 | `ProviderCapabilityRevalidateService::revalidate(project_id, lc_id, provider)` 的 `lc_id` **还**用于探针执行上下文与证据根（`provider_capability_revalidate.rs:245` / `:447-459` / `:264-282`） | 直觉上「能力全局化了 ⇒ 参数 lc_id 该删」 | 保留参数，只改记录与证据的落盘作用域（决策 7） |
| X3 | `ProductionPolicyTargetResolver::for_lc`（`production_policy_resolvers.rs:53`，**同一个文件里另一个 impl**）**不是** capability 作用域 | 名字与 `StoreBackedProviderCapabilitySource::for_lc`（同文件 `:233`）撞车，且同在 `production_policy_resolvers.rs`，容易一起改 | **不动**：checkout 目标解析必须按 LC（`gateway_factory.rs:191-196` 的 R9 fix 语义） |
| X4 | `gateway_factory.rs:144-147` 与 `:237-240` 各有一个 `match lc_id { Some => for_lc, None => new }` | 两处分号不同、看起来像重复代码 | 两处**都**收敛为全局；`None` 分支保留「无 root-recipe 通道」原语义 |
| X5 | `ensure_bootstrap` 是**写**路径但也决定「记录是否存在」⇒ 读侧把「文件不存在」当未核验 | 写路径与读路径耦合在同一文件的存在性上 | 语义不变（决策 12）；只改落点 |
| X6 | `src/web/handlers/logical_codebase_capabilities.rs` / `logical_codebase_capability_batch.rs` 的路由带 `lc_id` 且有 `require_logical_codebase` 404 守卫 | 全局化后容易想「路由也该去 lc_id」 | 路由保留（决策 10） |
| X7 | `src/web/handlers/aggregate_initialization/handlers.inc.rs:888-905` `capability_revalidate_entry` 生成的 `details.revalidate.path` 带 `lc_id` | 失败面文案来源，容易漏改 | 路径保留（与 R6 一致） |
| X8 | `src/product/logical_codebase/provider_capability_store.rs:3-4` 模块文档已写「存于 `.aria/projects/{project_id}/logical-codebase/capabilities.json`」 | 文档反而是**正确**的，per-LC 才是漂移的那一半 | 文档保持；`for_lc` 删除后文档自动成为唯一事实 |
| X9 | `src/product/logical_codebase/store.rs:1106-1109` legacy 判定只看 `manifest.json` | 全局 `capabilities.json` 落在 legacy 项目根，容易被误读成「多了一个 legacy 代码库」 | **已核实无影响**，不改 |
| X10 | `src/product/logical_codebase/provider_admission_preflight_tests_policy_body.inc.rs:781` 往 `scope_root.join("capabilities.json")` 写旧记录 | 作用域随 store 走，不 grep 字面量找不到 | 夹具迁移（决策 10 / tasks 第 6 节） |
| X11 | `src/product/logical_codebase/provider_gateway_tests/audit.inc.rs:174-176` 读 LC 子树 `capabilities.json` 的字节做「导入不得改写 durable 字节」断言 | 断言的是**路径**，不是语义 | 改指全局路径，断言语义不变 |
| X12 | `src/web/workspace_ws_handler/tests/gateway_start_parts/author_launch_cases.inc.rs:649-652` 删 `capabilities.json` 以确定性触发 validate 失败 | 靠删文件触发失败，路径改了就删不到 | 改指全局路径，触发语义不变 |
| X13 | `src/web/handlers/automation_enrollment_test_support.rs:859-860` 的 `match lc_id { Some => for_lc, None => new }` | 与 X4 同形，分散在 web handlers 里 | 夹具迁移 |
| X14 | `src/web/wiga_gate_fixture.rs:252` / `:507` 两处 `for_lc` 播种 | 同一文件两处，容易只改一处 | 夹具迁移，**两处同改** |
| X15 | `src/web/test_controls/plan_repair/provider_matrix.rs:431-435` | test_controls 目录容易被漏 | 夹具迁移 |
| X16 | `src/web/handlers/automation_gateway_preflight.rs:569-573` | 位于 `#[cfg(test)]` 模块内（`:308` 起的 `use` 块），grep 容易误判为生产路径 | 夹具迁移 |
| X17 | `src/web/handlers/automation_target.rs:186`、`automation_enrollment_tests_reenable.inc.rs:627`、`automation_reenable_supersede_tests.inc.rs:647` | 三处同形播种 | 夹具迁移，**三处同改** |
| X18 | `src/product/logical_codebase/provider_selection.rs:561`（测试播种 `write_records`）、`:900-903`、`:928-931` | 选择器的测试用**不同 lc_id** 造「同源一致 / snapshot 漂移」两种场景 | 夹具迁移：作用域改全局，但**两种场景的区分方式必须重做**（不能靠不同 lc_id 区分了） |
| X19 | `src/product/logical_codebase/provider_capability_revalidate.rs:718-719` / `:762-763` / `:796-797` / `:824-825` / `:875-876` | 单测里五处 `for_lc` 播种/读回 | 夹具迁移 |
| X20 | `src/product/logical_codebase/provider_boundary_probe.rs:2366-2371` | 外部入口导入测试 | 夹具迁移 |
| X21 | `src/product/logical_codebase/provider_admission_preflight_tests_policy_body.inc.rs:812-816` | 读回 LC 子树记录做 legacy 对比 | 夹具迁移 |
| X22 | `src/product/logical_codebase/coordinator_provider_turn.inc.rs:955` | 聚合初始化执行期的 `ensure_bootstrap`（测试夹具内） | 夹具迁移 |
| X23 | `web/src/components/lifecycle/ProviderCapabilityBatch.test.tsx:8`（文件头注释）、`ProviderCapabilityCard.test.tsx:618`、`LogicalCodebaseManagementPanel.test.tsx:117` | 三处断言同一句文案 | 文案同改 |
| X24 | `src/product/logical_codebase/provider_capability_store.rs:1155-1159` / `:1629-1634` 的旧记录夹具 | 用 `new` + 项目级路径写旧 v1/v2 字节 | **本来就是项目级** ⇒ 不用改（可作为正确样板） |
| X25 | `web/src/components/lifecycle/ProviderCapabilityBatch.tsx` 与 `ProviderCapabilityCard.tsx` 正被并行任务改动（codegraph 移出能力面板）⇒ 本变更要改的文案所在文件会位移 | 按行号定位会打到别人的改动上 | 文案定位改用**内容 + `data-testid="capability-batch-scope"`**；写盘前重读该文件 |

## 14. 明确不做的事（防越界）

- 不改 capability 记录结构、版本钉定、三态、`record_verified_probe` 导入语义
- 不改 provider 优先级、安装/更新机制、探针文案
- 不改「不点击不运行探针」契约
- 不改任何 **capability 以外** 的 per-LC 作用域（manifest / policy / operation / trust / receipt / checkout / index）
- ⚠️ **不改路由形状** —— 已被决策 14 取代（capability 面改机器级路由）；**不改 DTO 形状**仍有效（除新增 root-recipe 事实的独立载体）
- 不新增 capability 判定函数（`launch_cell_state` 仍是选择器与 gateway 的唯一共用判定）
- 不做迁移脚本、不做读时提升、不留兼容读取分支（决策 9）

## 15. 2026-11-05 追加决策（机器级化）

> 触发：owner 指出「provider 不是项目的附属，是平台的公用能力」⇒ 作用域不应停在项目级。下列事实全部已在本仓核对（行号为准）。

### 15.1 新增事实（机器级化的证据基础）

| # | 事实 | 位置 |
|---|---|---|
P1 | 能力探针在 `std::env::temp_dir()` 下的**临时目录**执行，跑完 `remove_dir_all` 清理；fixture 为**自造材料**（假 `AGENTS.md`「# lc root」、假 `.mcp.json`、假 `.aria/state.json`、假 `root/member-a/member-b/home/target-wt`、`git init`） | `src/product/logical_codebase/provider_capability_revalidate.rs:475-485`、`:268`、`:291`；`src/product/logical_codebase/provider_boundary_probe.rs:348-452` |
P2 | 探针**不读**真实聚合根、成员仓、项目 policy artifact、root-recipe、trust registry；唯一真实输入是宿主 PATH 上该 CLI 的存在与版本 | `provider_boundary_probe.rs:365-452`、`:1515-1533`（projection 用固定占位 digest 直构） |
P3 | capability 记录字段**无任何** project_id / lc_id / 路径字段；绑定只发生在**文件位置层** | `src/product/logical_codebase/provider_capability_store.rs:541-561`、`:878-886` |
P4 | 记录中唯一项目相关的字段是 `root_recipe_evidence`（绑某 LC 的 root recipe 交付 + 凭据按 per-LC operation Running 重核验） | `provider_capability_store.rs:205-208`；`src/product/logical_codebase/production_policy_resolvers.rs:361-419`、`:382-402` |
P5 | `probe_artifact_ref` 指向**项目级**证据目录 `capability-evidence/boundary` | `provider_capability_revalidate.rs:462-471` |
P6 | 同仓 provider 健康与安装面**已是机器级**：路由无 `{project_id}` 段，handler 内 `project_id` / `projects_root` grep **零命中** | `src/web/app.rs:24-27`；`src/web/handlers/providers.rs`、`src/web/handlers/provider_installation.rs` |
P7 | capability 的 4 条路由全部带 `{project_id}` + `{lc_id}`；`lc_id` 现仅承担 404 守卫与探针工件归属，**不进快照** | `src/web/app.rs:188-198`；`src/web/handlers/logical_codebase_capabilities.rs:75-77`、`:101-102` |
P8 | 批量作业是**唯一**按 `lc_scope_root` 分目录的 capability 面；job 结构固化 `project_id` + `logical_codebase_id` 并双校验归属 | `src/product/logical_codebase/capability_revalidate_batch.rs:174-184`、`:221-230`、`:432-442`、`:456-461` |
P9 | 前端入口按钮被 `disabled={!selectedProjectId}` 门控；面板 props 传 `projectId` + `logicalCodebaseId`，任一为 null 则整卡不渲染 | `web/src/components/lifecycle/ProjectSidebar.tsx:242`；`web/src/components/lifecycle/ProviderManagementPanel.tsx:10-30`；`web/src/components/lifecycle/ProviderCapabilityCard.tsx:177/202/222/232` |
P10 | 前端取 id 的逻辑自述「端点仍按逻辑代码库定位（承载 404 守卫），内容与逻辑代码库无关」 | `web/src/components/lifecycle/IssueLifecycleWorkbenchView.tsx:128-132` |
P11 | 共享验收实例实测**没有**任何 capability 记录文件（核验从未成功）⇒ 存量面近乎为零 | `.aria/projects/project_0001/logical-codebase/` 实测无 `capabilities.json` |

### 决策 13：作用域唯一化 —— 机器级（整机）

**决定**：capability 记录**只有一个作用域：机器级（整机）**。

- 写：全部落机器级文件（`.aria/provider-capabilities/capabilities.json`）。
- 读：全部读同一份。
- **SHALL NOT** 存在第二条读路径（项目级 / per-LC 子树）、**SHALL NOT** 存在回退/提升分支。

**理由**：

1. **事实的归属是整机**：探针跑在临时目录的自造 fixture 里（P1/P2），记录字段无一绑定项目或 LC（P3）⇒ 「这台机器上某家 provider CLI 能不能跑这个 action」就是**整机事实**。
2. **仓库已有同构先例**：provider 健康与安装面早就是机器级（P6）⇒ capability 面是**唯一的例外**，这个例外没有技术理由，只是历史包袱。
3. **成本**：一次真实验证起 provider 进程、跑分钟级探针、烧额度。按项目/逻辑代码库复制 = 同一份结论重跑 N 遍。
4. **可用性**：现状下「没有选中项目 ⇒ 能力面板不可用」（P9）⇒ 平台公用的能力被一个项目绑住，用户在别的项目里用不了同一台机器已具备的能力。

**代价**：

- 用户在任意项目里做的核验会改变**整机**的能力状态 —— 这是**要的效果**，不是副作用（owner 明确：核验只核验一次，是机器级的）。
- provider 换代/版本漂移会让**整机**同时需要重验 —— 同样是要的效果（本来就是同一份事实）。

### 决策 14：能力面改机器级路由（clean cutover）

**决定**：

- 新路由：`GET /api/providers/capabilities`、`POST /api/providers/capabilities/revalidate`、`POST /api/providers/capabilities/revalidate-all`、`GET /api/providers/capabilities/revalidate-all/{job_id}`。
- 旧路由（`/api/projects/{p}/logical-codebases/{lc_id}/capabilities*` 四条）**删除**，不留兼容别名、不留转发 shim、不留双写。
- 批量作业落盘改为机器级（不再按 `lc_scope_root` 分目录）；job 结构不再固化 `project_id` / `logical_codebase_id`，归属校验相应取消。
- 前端：能力卡与面板**不再**以「选中项目 / 存在逻辑代码库」为渲染或发起的前置条件；「Provider 管理」入口在无选中项目时也可用。

**理由**：

- 机器级事实用项目级路由承载，是「作用域与路由形状错配」的典型形态（决策 10 已被取代）；留着会让下一个改这块的人重新得出「lc_id 决定记录作用域」。
- 仓库铁律要求**干净切换**：迁走所有调用方、删除旧路径，不留 shim。
- 入口的门控（P9）在机器级语义下是**纯 bug**：平台公用能力被一个项目绑住。

**代价**：前端 4 处 + 后端 4 个文件 + 夹具与断言需一次性迁完（P7/P8/P10 的调用点清单见 tasks 第 8 节）。

### 决策 15：两层分离 —— root-recipe 事实拆出为 per-LC

**决定**：

- `root_recipe_evidence`（某 LC 的 root recipe 是否交付、凭据是否仍有效）**从 capability 记录拆出**，改为 **per-LC 独立事实**（存放于该 LC 的作用域）。
- 它继续按 per-LC durable operation 的 Running 状态重核验（`AggregateInitializationOperationStore::for_lc` 不动）。
- 机器级 capability 记录 SHALL NOT 含任何 project/lc/路径字段。
- 准入强度与结论**均不变**。

**理由**：`root_recipe_evidence` 的效力依赖某个具体 LC 的 durable operation 状态（P4）⇒ 它是 per-LC 事实。若塞进整机记录，就把「某个 LC 的状态」写死成「整机的事实」—— 别的项目会读到与自己无关的 root-recipe 结论。反之，通用能力证据（版本 / 方言 / 三格矩阵 / 执行语义）本就不带项目信息（P3），升机器级无损。

**代价**：多一个 per-LC 事实载体；写入者（root recipe 交付路径）与读取者（`require_root_recipe_supported`）都要改落点/读点，但判定逻辑不变。

### 决策 16：存量数据 —— 没有旧的，不写任何存量分支

**决定**：**不**迁移存量 capability 记录、**不**做读时提升、**不**保留兼容读取分支、**不**写任何「若未来出现存量」的兜底逻辑。

**理由**：owner 2026-11-05 再次确认「**旧的项目都不用管，没有旧的**」；owner 2026-11-04 已裁定本产品无历史数据；共享验收实例实测亦无任何 capability 记录文件（P11）⇒ 迁移代码是为不存在的问题写代码，反而制造第二种作用域。
