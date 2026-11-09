//! 知识库模块：Schema 4.0 KnowledgeBase 的 operation 数据模型与 per-LC
//! durable store（Task 1 后端底座）。

pub mod manifest;
pub mod operation;
pub mod store;

pub use operation::{
    KnowledgeBaseOperation, KnowledgeBaseOperationInput, KnowledgeBaseOperationKind,
    KnowledgeBaseOperationStatus, KnowledgeBaseStepKind, KnowledgeBaseStepRecord,
    KnowledgeBaseStepStatus, derive_operation_id,
};
pub use store::KnowledgeBaseOperationStore;
