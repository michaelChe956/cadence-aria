# Tasks: single-candidate-contract-preflight-and-loop-identity

**全局边界**：不翻转 options 默认值；prose 不取代 mechanical truth；CG typed turn/UI 语义不动（C2/C3 域）；既有 fail-closed 边界零放宽。

## 1. options 预检前移

- [x] 1.1 PlanCandidateValidationContext 增存储 options；三生产构造路径显式提供（fixture 同步显式化）；红绿：integration=true 无 integration WI 首轮 Error（附修复动作）/三族同报/flag=false 保持 warning（REQ-WSC-02 场景 11-12）。
- [x] 1.2 Error→F5 mechanical ReviewVerdict 回灌链复用与回归（回灌后修订收敛路径全绿）。

## 2. AC×基线树交叉核对

- [x] 2.1 路径提取（受限模式）+fork base 树核对+Error finding（清单+三修复建议）；F-56 真实形态 fixture（status.html 跨分支引用被拦/基线内路径放行/显式声明路径豁免形态评审）（REQ-WSC-02 场景 13）。

## 3. 结构化 identity

- [x] 3.1 fingerprint canonical 构造器（机械投影+受限 ID 提取+排序剥下标）；红绿：F-52 真实 9 轮 findings golden——同题异措辞同身份/异题不撞/unstable fail-safe 人工（REQ-TOP-04 场景 3-5）。
- [x] 3.2 legacy schema fallback 保留回归。

## 4. 前轮注入+Verification+cycle key

- [x] 4.1 reviewer prompt 结构化前轮清单注入；复评带重复标注用例（REQ-WSC-06 场景）。
- [x] 4.2 自动返修后 Verification scope 复评（不再 Initial 重开）；VerificationNewFindings 人工路径（REQ-TOP-04 场景 6）。
- [x] 4.3 cycle key=`sc:candidate:<source_revision_hash>`；同 revision 多节点同 cycle/新 revision 新 cycle（REQ-TOP-04 场景 7）；旧 key 不迁移 fail-safe unknown。

## 5. prompt 教学

- [x] 5.1 options 镜像+AC 基线纪律教学（与校验口径逐字对齐，预算红线内）；prompt contract 断言（REQ-WSC-06 场景）。

## 6. 门禁收口

- [x] 6.1 全链回归：lib/it_core（policy/handler 面）/it_web；strict 复跑；F-51/F-52/F-56 三案现场回放（options 首轮拦/9 轮 golden 判重/status.html 被拦）；记录证据。
