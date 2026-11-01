// 退役留档（T5/REQ-RET-02）：`review_returns_verdict_for_whole_candidate` 直接驱动已删除的 legacy 决策面
// （原 #[ignore]：legacy full-candidate review 流已被取代；本测含 author_decision 退役 wire 字面量），
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`work_item_plan_review_returns_decision_response` 直接驱动已删除的 legacy 决策面
// （原 #[ignore]：含 author_decision/review_decision_response 退役 wire 字面量），
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：review 模块夹具族（ws 连接/prepare_work_item_plan_and_author_to_confirm/
// enable_revise_review_fixture）为上者专用，随消息族一并退役。
