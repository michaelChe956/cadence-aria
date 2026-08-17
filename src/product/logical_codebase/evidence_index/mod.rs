// C-4 跨仓只读证据中介（EvidenceIndex）。
//
// Task 1（spike）已完成：在 2 成员仓临时聚合根（api/ + web/）上实测
// `codegraph query --json` 的真实输出，固化为 tests/fixtures/query_hit.json，
// 并有一条解析冒烟测试断言字段存在（见 tests/query_hit.rs）。
//
// Task 2 在此实现 `EvidenceHit { file_path, start_line, symbol }` 解析：
// file_path <- node.filePath（聚合根相对路径），start_line <- node.startLine，
// symbol <- node.name。本文件仅为骨架，实现由 T2 填充。

#[cfg(test)]
mod tests;
