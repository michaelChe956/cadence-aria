# kimi×重（levels×双脚本）收官证据——3.6 矩阵 8/9

- 收官序列：**rep35 / rep36 / rep37 三连 workitem 全过（C2 达成，2026-09-12 晨）**
- 判据口径：C1-重=计划 Confirmed+advance Ready（编码 completed 为加分项）；SOP=levels+双 request-change+60min 预算
- 通过线逐项（三跑全绿）：digest 锚 levels+completed+confirmed=1+advance completed+attempt_id 非空+ledger before==after+handoff confirmed+**durable group-initializations admission_kind=sc_advance**

## 三跑明细

| 跑 | issue | 时长 | 判据 |
|---|---|---|---|
| rep35 | issue_0247 | 3034s | 全绿（50min 长链收网） |
| rep36 | issue_0248 | 1827s | 全绿 |
| rep37 | issue_0249 | 3162s | 全绿；coding 加分项=miss（见下） |

二进制注记：rep1-26 在 v15（92c6ee9b），rep27+ 在 v16（4d0cd0be）；v16 仅含 coder 完成报告契约教学（c106a308），workitem 路径零重叠（k3 核验），计数续跑。

## coding 加分项谱系（全 miss，如实入档）

1. v14 rep1（预修复数据点）：InvalidRepairTarget（route OperationalGate×CurrentWorkItem）
2. v15 coding1b：unknown variant `OperationalGate`（大写枚举）
3. v15 coding2：90min 硬超时（coder 停滞）
4. v15 coding4：InvalidRepairTarget（同族）
5. **v16 coding37（收官跑）：结构合法✓（route×target 教学后首个零 InvalidRepairTarget finding），但内容=幻觉环境误报（「文件系统只读挂载」——实测 rw/盘 24%/touch 成功）→分诊门正确拦**

## 收官前的完整谱系（37 跑，17过/20悬）

0199 契约网缺口（compile 拒）×13 为主 + 语法族×4 + 60min 硬超时×1 + 上游空输出×2（通道坏期不计）。缺口五跑比对=纯游走（错误体/静态托管/stop 生命周期/相对路径/404/unlocked 各不相同）——无固定教学点，纯 flash 概率；三连达成靠重跑收敛（C2 纪律）。

## 模型/环境

- 模型基线：glm-5.3-flash 经 kimi CLI 0.41.0；服务器 v15→v16
- kimi 特征复现：不自行 commit（v16 收官跑改为幻觉误报而非违规）；terminal cwd 绝对路径容忍（935665f3）持续有效
