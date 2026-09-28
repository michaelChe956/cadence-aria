# Tasks

## 1. coding run 韧性底座：断连产物、最小互斥、abort→restart、人工继续（REQ-CRO-01～REQ-CRO-04、REQ-CRO-06）

- [ ] 1.1 将 manager／runner 完成收尾路径改为“source／artifact／completion checkpoint 先写既有持久面，再发观察事件”：attach／detach／supersede 只影响观察，`spawn_provider_run_claiming_idle` 的不取消语义贯穿返修完成路径；完成状态不明时落地“完成状态待确认”等待项并通知，不自动重启。
- [ ] 1.2 在 coding 启动 admission（`ensure_provider_run_allowed` 临界区）接入租约三态判别：复用 C1 `classify_worktree_lease`／`confirm_takeover`，活跃拒绝并通知“已在运行”、已死经用户确认接管（展示最后活动时间与节点证据，接管后旧持有者迟到写入被拒）、未知停等；同 `command_id` 重复 kick 幂等返回首次结果。
- [ ] 1.3 为 `coding_run_registry` 的退役状态增加显式 restart 产品动作：确认无活跃编码进程后清除退役标记并走既有终态重开 admission；restart 仅在 Aborted／Failed 且无活跃进程时可见，携带 attempt 身份与版本，旧 run 迟到回执被拒，不依赖重启服务。
- [ ] 1.4 把 `manual_continue`、允许的质量绕过与验证处理结论加入 `should_resume_runner_after_gate_response` 续跑白名单：门结果先写既有 gate 记录再唤回原 runner 从该阶段之后继续；复用已持久化 review 结论不重跑；重复提交幂等；续跑不依赖提交动作的连接。

**验收映射：** A06 断连／双 kick（provider 只启动一次、已完成 artifact 保留、重复操作幂等）；A08 runner 恢复（manual_continue 后关 socket 续跑、abort 后同进程 restart、旧回执被拒）；A09 coding 部分（活跃冲突停等不抢、确认接管后续跑）。

## 2. 证据区间与指令单次消费（REQ-GCE-C2-INSTR、coder-owned-work-item-commit／coding-workspace-completion／group-final-review-triage deltas）

- [ ] 2.1 execution 区间按事实派生：重试 execution 创建时重置 `start_commit`；新 execution 在 provider 认领前记录工作树真实 `HEAD`；`group_completion` 删除基线回填，改为起点缺失诊断停等；零提交记录空区间，重连不改写 start，最终 scope 以各 execution 真实区间聚合，人工 WIP 不倒算，越界提交仍被既有终态检查拒绝。
- [ ] 2.2 统一 spawn 与 rework 的返修指令／context note 消费口径：先渲染完整 prompt 与执行上下文，再一次可重放原子写入完成“认领→绑定渲染结果→标记消费”，最后 spawn；渲染失败（含空渲染）不消费；中断后用户点继续以同一认领与同一上下文 hash 启动，指令不被消费第二次，旧 hash 不覆盖；复用 C1 命令账本幂等模式。

**验收映射：** A09（带人工 WIP 的零提交重跑可 FinalConfirm、越界仍拒）；A15（send_to_coder 在 prepare／bind／consume／spawn 窗口注入中断后，用户操作恢复、指令在真实 prompt 中且只消费一次、不改旧 hash）。

## 3. 验证分诊与受限政策读取（REQ-CVT-01～REQ-CVT-05、REQ-ENV-C2-POLICY、coding-code-review-triage delta）

- [ ] 3.1 验证类等待面并列展示计划 check 命令与 coder 实际执行命令、cwd、exit、环境摘要（缺失显示“未记录”），标注“实际执行命令与计划不一致”而非计划缺陷；提供“重跑原计划命令”（走既有 rework 路径、计划字面命令入 prompt、`command_id`＋gate／check 身份、幂等）；#2／#9 错误文案按“实际不一致／计划未声明／计划路径不可执行”三类口径区分，不建议升级运行时、泛化重试或忽略 finding。
- [ ] 3.2 落地独立验证处理／受限豁免入口与持久记录：入口在门呈现面旁、不进原门动作集合（`coding_output_human_triage` 两动作与三个 Code Review 门四动作不变）；记录绑定 finding／check／plan revision／原与替代命令／cwd／结果／测试量／环境／scope／expiry；三类结论（批准计划修订经既有 amendment 链、接受可信等价证据、限域环境例外）全部需用户明确批准；零测试、错 revision、过宽 scope、重复转入、证据不完整均拒绝；findings 不清空只标注覆盖，FinalConfirm 仍人工且可见全部记录。
- [ ] 3.3 blocked／rework 页面提供“读取政策／重新授权”：经唯一 LC authority resolver（C4 交付）解析 attempt 冻结 envelope 的 policy_id／revision／digest 返回同 digest 正文；重新授权绑 attempt＋role＋digest、仅该返修 run 运行态有效；错 role、过期、错 attempt、digest 不匹配、越界成员范围拒绝；resolver 不可唯一解析 fail-closed 停等，不回落备用路径，不向 note 写绝对路径。

**验收映射：** A11 验证分诊（计划合法但 coder 误跑→重跑计划且不误判；计划字面命令不可满足→受控进入唯一验证面；原门动作集合不变；修订／等价证据／限域豁免均需人批准，finding 不被清空）；A15 政策部分（blocked／rework 页面产品操作读取同 digest policy；错误角色／过期对象拒绝）。

## 4. reviewer 三值、SC 完整预算与统一收尾（REQ-CRO-05、REQ-CG-03、REQ-CRO-06 收口）

- [ ] 4.1 根除 reviewer 毒默认：删除 `provider_config.rs` 的 author 回填与 `plan.rs` 的 Codex 回填；持久化并按原值重建 effective／provisional／enabled 三值（消费既有 `provisional_reviewer_provider`／`reviewer_enabled_at_start` 并补齐 coding attempt 侧）；缺失时落“reviewer 配置缺失”等待项＋配置／重试操作，disabled 保持 disabled，不以 author 顶替、不视为自动通过。
- [ ] 4.2 SC 人工返修完整预算与传输：扣回合前计算完整候选＋固定合同＋feedback＋上下文与 provider 真实预算；超 inline 经既有 artifact 读取或完整有序分块传输并记录组装 digest，候选不截断不摘要；不可读／缺块／超硬限在 turn CAS 前通知停等，门与预算不变，用户点击“分段返修／重试”后才开新回合。
- [ ] 4.3 把全部 C2 等待项（完成状态待确认、已在运行、确认接管、活性未知、restart、reviewer 配置缺失、验证处理、政策核验、消费中断、大候选停等）接入统一通知／操作结果投影（复用 C1WaitingItemDto 驾驶舱只读模式）：展示原因／身份／已完成步骤／副作用／操作边界／下一阶段，每操作带稳定 `command_id` 与 expected 版本，REST 与页面同一应用服务，通知失败可补读；完成定向回归与故障注入记录，覆盖“错误→通知→用户操作→原链继续”真实观察链。

**验收映射：** A10 reviewer 部分（重启后 null 仍停等、不意外启动 reviewer、无 author 回填）；A14 大候选返修（28,695B 及更大支持候选完整返修不截断；artifact 不可读／缺块／超硬限在 CAS 前停等且门与预算不变）；A06／A08／A09／A11／A15 的通知与操作结果链。
