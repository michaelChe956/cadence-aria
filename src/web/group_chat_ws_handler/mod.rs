//! 群聊 Spec 生成独立 WebSocket endpoint。
mod session;

pub use session::group_chat_ws;

#[cfg(test)]
mod tests;
