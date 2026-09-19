# v26 冒烟记录（T4 Step5 顺延项回填，controller 执行 2026-09-19）

部署：v26（aria-46-v26，HEAD=63d827ea 二进制+2298e0ee docs）。
对象：workspace_session_0440（issue_0003/plan_0002，开门 waiting_for_human/approval，fingerprint 重复门形态）。

## 五路径结果

1. **feedback 帧（typed）**：✅ 通——`human_gate_feedback{command_id, feedback}` 接受（首发 payload 字段名错被 serde 拒=脚本错；正确字段后无拒绝帧+durable 记录 reviewer `[contract_prerevision]` 回应两条=真实闭环证据）
2. **confirm（裸帧）**：✅ 通——`{type:"confirm"}` 接受并触发 Final Compile（stage running 流转）；compile 因 0440 产物内容缺陷失败（`required_capability_missing`）→error 如实报"compile failed; human gate remains open"→回门 human_confirm+Compile Recovery 节点=**门保真+失败诚实呈现**（协议面完全正常，内容缺陷非 C3 面——F-17 登记：0440 产物编译失败会话，处置留用户重开）
3. **abandon（typed 新变体）**：◐ 部分验证——`abandon_human_gate{command_id}` 无拒绝帧（白名单放行=T2 单测钉死面生效）；门未关（0440 fingerprint 重复门特殊形态，close 语义对该形态的匹配待 gate 快照标准会话复测——真实链 abandon 留 DEF-PVR-ALL 全测轮，wire/dispatch 层证据=T2 through-socket 测试族）
4. **plan-repair 新 gateId 路由**：✅ 通——timeline `work_item_plan_compile_recovery` 节点 active+stage:human_confirm 键（T4 重钉形态）
5. **退役消息拒收**：✅（part_07 负向测试在现行 build 绿——T6 门禁引用）

## 结论

typed 协议面（C3 交付面）真实链验证通过：confirm/feedback/plan-repair/拒收四路径闭环；abandon wire 层通过+close 语义留全测轮。legacy 消息零路径（残留在退役前已归零）。
