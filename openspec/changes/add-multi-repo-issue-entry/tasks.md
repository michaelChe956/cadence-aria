# Tasks: add-multi-repo-issue-entry

## 1. 后端:preflight 多仓放行

- [ ] 1.1 TDD:`preflight_single_repository_candidate` 语义测试先红——四象限(involved⊆focus 非空过/空集拒/界外拒/单成员等价旧语义);实现:读 selection.focus_repository_ids,恰一仓判定改子集判定;`plan_preparation.rs` 调用面同步
- [ ] 1.2 SC spec 主 spec 同步:REQ-WSC-08 场景按 delta 修订(归档时 sync,此处仅备好 delta 文本)
- [ ] 1.3 回归:lifecycle/plan 面既有测试全绿(单仓零回归准绳)

## 2. 后端:design 钉定上界化

- [ ] 2.1 TDD:钉定块内容测试先红(prompt 含勾选集上界+子集允许+界外即拒措辞);实现:`prompts.rs`/`entity.rs`/`builder.rs` 钉定块参数化(镜像 e78a97c6 单仓面的多仓推广)
- [ ] 2.2 TDD:write-back 越界拒测试先红(sentinel involved ⊄ focus → 拒回写+诊断);实现于 aggregate_writeback 面
- [ ] 2.3 回归:design 面既有测试全绿;单仓钉定行为不变断言

## 3. 前端:入口多选

- [ ] 3.1 TDD:`CreateLifecycleIssueDialog` 测试先红(LC 成员复选/多选提交写 focus_repository_ids/勾 1 等价单选/仓库路径不变);实现复选与提交链
- [ ] 3.2 回归:dialog 既有测试全绿+web 单测全绿

## 4. 集成与收口

- [ ] 4.1 集成测试:多选→create→selection durable 校验(focus⊆include)通过;单成员路径全链等价
- [ ] 4.2 全量门禁四条(fmt/clippy -D/check/test)绿;预存基线按名对照
- [ ] 4.3 k3 独立审查(单仓零回归面+钉定语义面为重点)并处置发现
- [ ] 4.4 openspec validate + tasks 勾选;页面 E2E 终验另行启动(本 change 交付门=4.2+4.3)
