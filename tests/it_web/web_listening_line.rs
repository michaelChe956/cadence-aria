use cadence_aria::web::app::{LISTENING_LINE_PREFIX, listening_line};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

#[test]
fn listening_line_prefix_is_stable_contract() {
    // launcher (bin/aria.js) 匹配此前缀判定就绪；修改即破坏分发，需同步更新。
    assert_eq!(LISTENING_LINE_PREFIX, "aria web listening on http://");
}

#[test]
fn listening_line_renders_addr() {
    let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 4317));
    let line = listening_line(&addr);
    assert_eq!(line, "aria web listening on http://127.0.0.1:4317");
    assert!(line.starts_with(LISTENING_LINE_PREFIX));
}
