# Tasks: add-multi-repo-issue-entry

## 1. 后端:preflight 多仓放行

- [ ] 1.1 TDD:`preflight_single_repository_candidate` 语义测试先红——**六象限**(①involved⊆上界非空过含真子集②involved 空回退「上界恰一仓」口径(单成员过/多成员拒)③focus=∅ 存量 AllMembers 走 resolved 上界放行④界外拒⑤单成员等价旧语义⑥多成员上界+空 involved 拒);实现:上界=focus 非空取原集/空取 resolve_effective_members(design 关键点 0),恰一仓判定改子集判定+空回退;`plan_preparation.rs` 调用面同步
- [ ] 1.2 SC spec 主 spec 同步:REQ-WSC-08 场景按 delta 修订(归档时 sync,此处仅备好 delta 文本)
- [ ] 1.3 回归:lifecycle/plan 面既有测试全绿(单仓零回归准绳,含缺陷#7 回退口径面)

## 2. 后端:design 钉定上界化+守卫三环

- [ ] 2.1 TDD:钉定块内容测试先红(prompt 含上界集合+子集允许+界外即拒措辞);实现:`prompts.rs`/`entity.rs`/`builder.rs` 钉定块参数化(镜像 e78a97c6,上界=resolved 集合同源)
- [ ] 2.2 TDD:write-back 越界拒测试先红(sentinel involved ⊄ resolved 上界 → 拒回写+诊断);实现于 aggregate_writeback 面
- [ ] 2.3 TDD(k3 P2-5):generate 入口出生值校验先红(`GenerateDesignSpecsRequest.involved_repository_ids ⊄ resolved 上界 → 拒`,收敛早于 preflight);实现于请求消费面
- [ ] 2.4 回归:design 面既有测试全绿;单仓钉定行为不变断言

## 3. 创建链与入口多选(前后端)

- [ ] 3.1 TDD(k3 P1-3):后端创建链先红——`CreateProjectIssueRequest`(src/web/types.rs)增 `focus_repository_ids` 复数字段+`create_logical_codebase_issue`(product_resources.rs)改 explicit policy(带 focus=写勾选集,不带=维持 all_members 存量兼容);durable 校验(focus⊆include)通过
- [ ] 3.2 TDD:前端对话框先红(LC 成员复选/多选提交带 focus_repository_ids/勾 1 等价单选/仓库路径不变);`web/src/api/types/common.ts` DTO 同步;实现复选与提交链
- [ ] 3.3 回归:dialog 既有测试+web 单测全绿;create 链既有测试全绿

## 4. 集成与收口

- [ ] 4.1 集成测试:多选→create→selection durable 校验通过;单成员路径全链等价;focus=∅ 存量路径等价
- [ ] 4.2 全量门禁四条(fmt/clippy -D/check/test)绿;预存基线按名对照
- [ ] 4.3 k3 独立审查(单仓零回归面+钉定语义面+三环守卫同源性为重点)并处置发现
- [ ] 4.4 openspec validate + tasks 勾选;页面 E2E 终验另行启动(本 change 交付门=4.2+4.3)
