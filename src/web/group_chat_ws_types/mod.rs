//! 群聊 Spec 生成独立 WebSocket 的线协议 DTO。
pub mod in_;
pub mod out_;

pub use in_::GroupChatWsInMessage;
pub use out_::GroupChatWsOutMessage;
