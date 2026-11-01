# change 实施 C1 contract-preflight-and-loop-identity 计划 v1.0

> 按 superpowers:executing-plans / subagent-driven-development 执行。

**Goal:** options 预检前移+AC×基线树核对+结构化判重+Verification+cycle key，根治 F-51/F-52-L1/F-56。

**Architecture:** 校验上下文 threading（validator 面）→ AC 路径核对（新校验）→ fingerprint canonical 构造器 → policy/Verification/cycle key → prompt 教学。后端 Rust 为主。

**Spec:** `openspec/changes/single-candidate-contract-preflight-and-loop-identity/`（strict valid）；统一方案 §5-C1；实证 f51/f52/f56-diagnosis.md。

## Global Constraints

- 三族校验复用 `WorkItemSplitValidator::validate` 语义（禁复制规则）；Error 走 F5 mechanical ReviewVerdict 回灌（contract_prerevision 先例）。
- AC 路径核对只认显式路径形态（反引号/引号内 repo 相对路径、trusted_commands/验证命令文件参数），不做全文模糊匹配。
- identity：机械=确定性投影；reviewer=受限提取 WI/CT/AC ID（排序剥下标规范化尾段）；unstable fail-safe 人工；legacy fallback 保留。
- Verification scope/cycle key 按 TOP-04 既有契约实现（`sc:candidate:<source_revision_hash>`）；存量 seen_fingerprints/旧 cycle key 不迁移（新身份新 cycle 生效，旧值 fail-safe unknown）。
- prompt 预算红线内净增（超线按 EarsImpl 方案 A 删冗余先例）。
- 测试命令照仓规；前端零改（本 change 纯后端+prompt）。

## Review Focus

1. options threading 三路径漏一（fixture 默认值丢意图）→ T1 用例三路径各一
2. AC 核对误报（模糊匹配/非路径文本）→ T2 负例
3. identity 撞指纹（异题 collapse）与漏判（同题异措辞）→ T3 用 F-52 真实 9 轮 golden
4. Verification 误开开放式初审 → T4 scope 断言
5. cycle key 与修订轮的稳定性（同 revision 多节点）→ T4 用例

---

### Task 1: options threading+三族预检（REQ-WSC-02 场景 11-12）

- [ ] 1.1 失败测试：①integration=true 无 integration WI 首轮 Error（附修复动作文案）②e2e/split 同构 ③三族同报 ④flag=false 保持 warning ⑤三生产路径（author/revision/运行期）各一 threading 用例（fixture 显式 options）。
- [ ] 1.2 跑红→实现（context 增字段+三路径提供+复用 split validator+Error 适配 F5 verdict 回灌）→绿；既有 validator 回归。
- [ ] 1.3 回灌链回归（Error→修订→复评收敛路径全绿）。

### Task 2: AC×基线树核对（REQ-WSC-02 场景 13）

- [ ] 2.1 失败测试：①F-56 真实形态（引用 issue_0001 分支的 status.html）被拦+三修复建议 ②基线内路径放行 ③非路径文本不误报 ④trusted_commands 中路径形态核对。
- [ ] 2.2 跑红→实现（受限路径提取+fork base 树核对+Error finding）→绿。

### Task 3: 结构化 identity（REQ-TOP-04 场景 3-5）

- [ ] 3.1 F-52 真实 9 轮 findings 提取为 golden fixture（python 读 durable，勿 jq）。
- [ ] 3.2 失败测试：①同题异措辞同身份（golden 实证）②异题不撞 ③unstable 标记+fail-safe 人工 ④legacy fallback 回归。
- [ ] 3.3 跑红→fingerprint canonical 构造器（机械投影+受限提取）→绿；seen_fingerprints 消费侧回归。

### Task 4: 前轮注入+Verification+cycle key（REQ-TOP-04 场景 6-7 / WSC-06 场景）

- [ ] 4.1 失败测试：①reviewer prompt 含前轮结构化清单+复评带重复标注 ②自动返修后复评 scope=Verification（不再 Initial）③VerificationNewFindings 人工路径 ④同 source revision 多 reviewer 节点同 cycle key、新 revision 新 cycle ⑤旧 key 会话 fail-safe unknown。
- [ ] 4.2 跑红→实现（prompt 注入/scope 切换/cycle key 改造）→绿。

### Task 5: prompt 教学（REQ-WSC-06）

- [ ] 5.1 options 镜像+AC 基线纪律教学（与校验口径逐字对齐；预算红线内）；红：contract 断言缺；绿：含且口径词一致。

### Task 6: 门禁收口

- [ ] 6.1 全链回归 lib/it_core/it_web+strict+fmt/clippy；F-51/F-52/F-56 三案现场回放记录。

---

## Self-Review

REQ-WSC-02 场景 11-13→T1/T2；TOP-04 场景 3-7→T3/T4；WSC-06→T4.1/T5；Review Focus 五条全挂测试。单 worker 串行（policy/consume 侧文件耦合，不拆并行）。
