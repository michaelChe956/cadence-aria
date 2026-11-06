## MODIFIED Requirements

### Requirement: 人工 design 确认后的显式授权与范围（REQ-WIGA-01）
系统 SHALL 保持 story/design 的生成和确认由人完成，并仅在 design 经人工确认后提供自动化模式选择。**「工作项组生成路径」SHALL 指：用户在已确认 design 上经 Work Item Plan 配置弹窗提交的工作项组生成。design 尚未确认时该弹窗不提供任何生成模式选择（既无「自动化」亦无「手动」），其提交不属于工作项组生成路径。在工作项组生成路径上 SHALL NOT 提供手动模式选择——已确认 design 的 Work Item Plan 配置弹窗 SHALL NOT 呈现「手动」选项，该路径的提交 SHALL 恒走托管自动化，其产物 SHALL 为托管自动化的工作项组。工作项组生成路径之外的既有行为 SHALL 逐字不变。**

#### Scenario: 未选自动化默认关闭

- **WHEN** design 经人工确认后用户在**工作项组生成路径之外**未选择自动化模式
- **THEN** 默认 SHALL 为不自动化。**在工作项组生成路径上 SHALL NOT 出现「未选自动化」：生成模式 SHALL 恒为托管自动化。**

#### Scenario: 工作项组生成路径不提供手动模式

- **WHEN** 用户在已确认 design 上经 Work Item Plan 配置弹窗提交一次工作项组生成
- **THEN** 界面 SHALL NOT 呈现「手动」模式选项
- **AND** 该次提交 SHALL 走托管自动化路径：SHALL 创建自动化 enrollment 并绑定其 plan/session，工作项组 SHALL 由服务端编排推进（REQ-WIGA-03），界面 SHALL 如实呈现托管事实
- **AND** 该次提交 SHALL NOT 改为人工推进的 workspace 会话，SHALL NOT 静默降级为手动生成
- **AND** 提交形态 SHALL 为「启用自动化」，SHALL NOT 出现「创建并打开 Workspace」

#### Scenario: 托管目标不可得时不得静默降级

- **WHEN** 用户在工作项组生成路径上提交，但托管自动化所需的授权目标不可得（目标投影读取失败或授权载体不受支持）
- **THEN** 界面 SHALL 如实告知该事实并给出可执行的下一步
- **AND** SHALL NOT 静默改为手动生成，SHALL NOT 伪造已启用自动化的状态
- **AND** 提交 SHALL 保持禁用；目标投影读取失败时 SHALL 提供重试；弹窗 SHALL NOT 提供任何以人工推进 workspace 会话创建工作项组的替代提交（含「创建并打开 Workspace」），且 SHALL NOT 发起人工 prepare 请求；用户 SHALL 可取消退出

#### Scenario: 工作项组生成路径之外的既有行为逐字不变

- **WHEN** 某次生成不经已确认 design 的 Work Item Plan 配置弹窗（story/design 的人工生成与人工确认入口、既有人工推进的工作项组上的确认／换代／重绑），**或 design 尚未确认因而该弹窗不呈现「生成模式」选择**
- **THEN** 既有手动生成与人工确认流程 SHALL 逐字不变（入口、文案与提交载荷）
- **AND** 托管化后的人工门、AI choice 与最终确认 SHALL 仍然保留（REQ-WIGA-03 / REQ-WIGA-05），SHALL NOT 因本 requirement 被取消

#### Scenario: 身份合同精确持久化

- **WHEN** 创建自动化 enrollment
- **THEN** 自动化 enrollment SHALL 将 target、plan、session、source、provider 和绑定版本作为同一条精确身份合同持久化。

#### Scenario: target 三类身份区分

- **WHEN** 解析并持久化 enrollment 的 target
- **THEN** target SHALL 是 enrollment 载体身份的唯一权威，并 SHALL 区分三类：单仓（携带与 issue 记录所属物理仓一致的真实 repository 身份，MUST NOT 携带或生成任何 logical repository 替身身份）、逻辑代码库单成员、逻辑代码库成员集合（`logical_codebase_members`：logical codebase 身份＋logical repository 成员集合）。

#### Scenario: 集合 target 规范化

- **WHEN** 反序列化原始 REST JSON 或读取磁盘中的集合 target 并比较身份
- **THEN** 集合 target SHALL 经统一反序列化规范化入口处理原始 REST JSON 与磁盘读取：非 UUID 或空集 SHALL 拒绝，乱序及重复输入 SHALL 规范化为升序唯一集合，单元素 SHALL 折叠为既有单成员 kind，旧合法 JSON/history 不迁移；digest、selection_key 与 bound.target 比较一律在规范化之后。

#### Scenario: 授权覆盖有效成员

- **WHEN** 为 issue explicit selection 建立自动化授权
- **THEN** 自动化授权 SHALL 覆盖 issue explicit selection 的全部有效成员（manifest∩(included−excluded)，排序去重），MUST NOT 要求逐仓勾选，MUST NOT 提供逐仓授权/启动 API；集合用于 enrollment 及其 binding/generation intent/child 授权身份，单 WI intent 仍单仓、MUST NOT 把任一集合成员当作全部 WI 归属。

#### Scenario: 实际桶与 attempt

- **WHEN** 生成 plan 实际桶集合 P 并创建 attempt
- **THEN** plan 实际桶集合 P SHALL 满足 ∅≠P⊆A（A＝授权成员集合）且无未归属 WI；attempt 只按 P 创建，A−P 的授权仓 MUST NOT 造空 attempt/空组。

#### Scenario: 重绑显式且不扩权

- **WHEN** 用户执行重新绑定／换代
- **THEN** 用户执行"重新绑定／换代"时 MUST 明确提供目标 plan/session、source、target 和 provider，并携带当前 expected binding/policy version；新旧 logical 走同一路径、规范化后全等，rebind MUST NOT 借道扩权，current/previous 保留全集，旧单成员 MUST NOT 随 selection 扩仓。

#### Scenario: 新绑定生效旧版只读

- **WHEN** 重新绑定操作成功
- **THEN** 成功操作 SHALL 写入新的绑定版本并使后续动作只接受当前版本；旧绑定、旧 child 与旧代事件 SHALL 保持只读可追溯。

#### Scenario: 禁止猜测接管

- **WHEN** 系统处理既有 session、issue 与 attempt 归属时
- **THEN** 系统 MUST NOT 按最新 session、旧 issue 扫描或后台失败发现猜测接管，也 MUST NOT 将 Manual 已认领 attempt 升级为自动。

#### Scenario: 手动缺省与非法拒绝

- **WHEN** 未选自动化、旧数据缺少 enrollment、单成员身份不明确或 target 非法
- **THEN** 未选自动化、旧数据缺少 enrollment 或单成员身份不明确时 SHALL 保持既有手动流程；零 target、空集、非法 UUID、错 LC、跨载体或身份不一致 SHALL fail-closed 拒绝自动授权。**「未选自动化 ⇒ 保持既有手动流程」这一条在工作项组生成路径上 SHALL NOT 适用——该路径不存在未选自动化的手动入口；旧数据缺少 enrollment、单成员身份不明确等其余情形 SHALL 保持既有手动流程不变。**

#### Scenario: design 确认前不出现自动授权

- **WHEN** story 或 design 尚未由人确认
- **THEN** 不创建 enrollment、不自动准备 plan，原有人工生成与确认入口继续可用

#### Scenario: 默认不自动化

- **WHEN** 用户在 design 确认后**在工作项组生成路径之外**选择不自动化，或旧 issue 不含 enrollment
- **THEN** plan 准备、生成、advance 和 coding 启动仍按原有手动行为执行，不因自动化补偿扫描产生动作。**工作项组生成路径上 SHALL NOT 出现「选择不自动化」这一选项。**

#### Scenario: 集合授权覆盖全部有效成员

- **WHEN** 已确认 design 的 issue explicit selection 覆盖同一 logical codebase 的四个成员仓，用户选择自动化
- **THEN** 系统创建一个 enrollment，其集合 target 规范化覆盖全部四个成员；入口不出现逐仓勾选，回传原样集合

#### Scenario: 实际桶 P⊆A 且无空组

- **WHEN** 授权集合 A 含四仓而 plan 只把 WI 归属到其中三仓
- **THEN** 系统只为三个实际桶创建 attempt，第四个授权仓不产生空 attempt/空组；存在未归属 WI 时拒绝并要求修正

#### Scenario: 乱序重复输入规范化后比较

- **WHEN** 客户端或磁盘 JSON 提交乱序、含重复 UUID 的成员集合
- **THEN** 统一规范化入口输出升序唯一集合后 digest/selection_key/bound.target 相等判定成立；空集或非法 UUID 被拒绝

#### Scenario: 单元素折叠与旧 wire 兼容

- **WHEN** 集合 target 只含一个成员
- **THEN** 系统折叠为既有单成员 kind 写出；旧单成员 JSON/history 原样可读，不迁移旧 digest

#### Scenario: 错仓或跨载体 target 被拒

- **WHEN** Enable 携带的单仓 repository 身份与 issue 记录所属物理仓不一致、集合含不属于该 logical codebase 的成员，或对单仓 issue 提交逻辑代码库 target、跨载体混用
- **THEN** 系统返回 422 与需重新选择目标的提示，不写 enrollment、不启动 provider

#### Scenario: 显式重绑创建新绑定版本

- **WHEN** 当前 enrollment 处于等待恢复且用户提交匹配 expected version 的 plan、session、source、target、provider
- **THEN** 系统写入新的绑定版本，返回当前 enrollment 身份与下一步操作；旧绑定保持只读，后续编排只消费新版本

#### Scenario: 单 target 精确绑定及重复提交

- **WHEN** 已确认 source 只对应一个精确 target（一个单仓物理仓，或逻辑代码库选择中的一个 logical repository），用户用相同选择键重复开启自动化
- **THEN** 系统仅返回同一 enrollment、同一绑定 plan/session 与策略修订；不同 source 或 provider/options 的冲突提交明确拒绝且不改绑

#### Scenario: 单仓 target 可被授权

- **WHEN** 单仓 issue 的记录指向一个已登记的物理仓，用户在 design 确认后选择自动化并提交服务端投影的单仓 target
- **THEN** 系统创建 enrollment，其 target 为该物理仓身份、不含任何 logical repository 身份，绑定版本 1 建立且后续自动链可以启动

#### Scenario: 多 target 不被自动纳入

- **WHEN** 无成员集合授权而已确认 source 或冻结的 plan/attempt 涉及多个目标仓库、目标不明确或与 enrollment target 不符
- **THEN** 系统拒绝自动授权或自动启动，绝不循环启动多个 attempt；既有逐 target 手工准入仍可用

#### Scenario: 绑定版本过期拒绝

- **WHEN** 用户以过期 expected binding/policy version 或不匹配的 plan/session/target 提交重绑
- **THEN** 系统返回版本冲突/需刷新错误，不覆盖当前绑定、不启动 provider、不改变旧代历史

#### Scenario: 同一命令重放幂等

- **WHEN** 客户端以相同 command_id 和相同绑定负载重复提交同一重绑/换代操作
- **THEN** 系统返回首次操作结果，不创建第二绑定版本、不重复触发后续编排；同一命令异负载被拒绝
