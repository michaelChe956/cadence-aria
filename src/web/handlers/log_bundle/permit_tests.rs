use std::os::unix::net::UnixStream;
use std::time::Duration;

const TEST_ORIGIN: &str = "http://127.0.0.1:4317";

/// 回收本次申请留在 OS 临时目录里的私有目录与 socket。
fn cleanup(ticket: &Ticket) {
    if let Ok(directory) = private_directory_path(&ticket.request_id) {
        let _ = std::fs::remove_dir_all(directory);
    }
}

#[test]
fn redeem_before_local_approval_is_rejected() {
    let grants = Grants::default();
    let ticket = grants.issue(TEST_ORIGIN).expect("issue");

    assert!(
        !grants.redeem(TEST_ORIGIN, &ticket.handle),
        "未获本机批准时句柄不得放行下载"
    );

    cleanup(&ticket);
}

#[test]
fn approved_grant_can_be_redeemed_exactly_once() {
    let grants = Grants::default();
    let ticket = grants.issue(TEST_ORIGIN).expect("issue");

    assert!(mark_approved(&grants.inner, &ticket.request_id, Instant::now()));
    assert!(grants.redeem(TEST_ORIGIN, &ticket.handle));
    assert!(
        !grants.redeem(TEST_ORIGIN, &ticket.handle),
        "重放必须失败关闭"
    );

    cleanup(&ticket);
}

#[test]
fn wrong_origin_does_not_consume_the_grant() {
    let grants = Grants::default();
    let ticket = grants.issue(TEST_ORIGIN).expect("issue");

    assert!(mark_approved(&grants.inner, &ticket.request_id, Instant::now()));
    assert!(
        !grants.redeem("http://evil.example", &ticket.handle),
        "跨来源核销必须被拒"
    );
    assert!(
        grants.redeem(TEST_ORIGIN, &ticket.handle),
        "跨来源的失败尝试不得消费既有授权"
    );

    cleanup(&ticket);
}

#[test]
fn grant_from_another_instance_is_not_redeemable() {
    let first = Grants::default();
    let second = Grants::default();
    let ticket = first.issue(TEST_ORIGIN).expect("issue");

    assert!(mark_approved(&first.inner, &ticket.request_id, Instant::now()));
    assert!(
        !second.redeem(TEST_ORIGIN, &ticket.handle),
        "别的实例不得核销本实例签发的授权"
    );
    assert!(first.redeem(TEST_ORIGIN, &ticket.handle));

    cleanup(&ticket);
}

#[test]
fn expired_grant_is_rejected() {
    let grants = Grants::default();
    let ticket = grants.issue(TEST_ORIGIN).expect("issue");

    assert!(mark_approved(&grants.inner, &ticket.request_id, Instant::now()));
    let after_expiry = Instant::now() + Duration::from_secs(GRANT_TTL.as_secs() + 60);
    assert!(
        !grants.redeem_at(TEST_ORIGIN, &ticket.handle, after_expiry),
        "过期授权必须失败关闭"
    );

    cleanup(&ticket);
}

#[test]
fn grant_issued_in_the_past_is_already_expired() {
    let grants = Grants::default();
    let issued_at = Instant::now()
        .checked_sub(Duration::from_secs(GRANT_TTL.as_secs() + 60))
        .expect("系统时钟足够");
    let ticket = grants.issue_at(TEST_ORIGIN, issued_at).expect("issue");

    assert!(
        !grants.redeem(TEST_ORIGIN, &ticket.handle),
        "签发即过期的申请不得放行"
    );

    cleanup(&ticket);
}

#[test]
fn pending_requests_are_capped_per_instance() {
    let grants = Grants::default();
    let mut tickets = Vec::new();
    for _ in 0..MAX_PENDING_PER_INSTANCE {
        tickets.push(grants.issue(TEST_ORIGIN).expect("issue"));
    }

    let overflow = grants.issue(TEST_ORIGIN).expect_err("待批准请求必须有上限");
    assert_eq!(
        overflow.kind(),
        io::ErrorKind::WouldBlock,
        "上限触发必须回报 WouldBlock 供 HTTP 层显示 429"
    );
    assert_eq!(tickets.len(), MAX_PENDING_PER_INSTANCE);

    for ticket in &tickets {
        cleanup(ticket);
    }
}

#[test]
fn request_ids_are_32_lowercase_hex_and_handles_are_independent() {
    let grants = Grants::default();
    let first = grants.issue(TEST_ORIGIN).expect("issue");
    let second = grants.issue(TEST_ORIGIN).expect("issue");

    assert_eq!(first.request_id.len(), REQUEST_ID_LEN);
    assert!(is_valid_request_id(&first.request_id));
    assert!(is_valid_request_id(&second.request_id));
    assert_ne!(first.request_id, second.request_id);
    assert_ne!(first.handle, second.handle);
    assert_ne!(first.handle, first.request_id);

    cleanup(&first);
    cleanup(&second);
}

#[test]
fn private_socket_approval_marks_the_request_approved_without_leaking_the_handle() {
    let grants = Grants::default();
    let ticket = grants.issue(TEST_ORIGIN).expect("issue");
    let socket_path = private_directory_path(&ticket.request_id)
        .expect("private directory")
        .join(SOCKET_FILE_NAME);
    let mut stream = UnixStream::connect(&socket_path).expect("connect to approval socket");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .expect("write timeout");

    stream
        .write_all(format!("{}\n", ticket.request_id).as_bytes())
        .expect("write request id");
    let mut buffer = [0u8; MAX_PROMPT_MESSAGE];
    let read = stream.read(&mut buffer).expect("read prompt");
    let prompt = String::from_utf8_lossy(&buffer[..read]).to_string();
    assert!(prompt.contains(&ticket.request_id), "提示必须带申请号");
    assert!(prompt.contains(TEST_ORIGIN), "提示必须带已验证来源");
    assert!(
        !prompt.contains(&ticket.handle),
        "批准通道不得递送下载句柄"
    );

    stream.write_all(b"y\n").expect("write confirmation");
    let read = stream.read(&mut buffer).expect("read verdict");
    assert_eq!(String::from_utf8_lossy(&buffer[..read]).trim(), "ok");

    let mut redeemed = false;
    for _ in 0..100 {
        if grants.redeem(TEST_ORIGIN, &ticket.handle) {
            redeemed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(redeemed, "本机批准后必须可以核销");

    cleanup(&ticket);
}

#[test]
fn socket_rejects_a_mismatched_request_id() {
    let grants = Grants::default();
    let ticket = grants.issue(TEST_ORIGIN).expect("issue");
    let socket_path = private_directory_path(&ticket.request_id)
        .expect("private directory")
        .join(SOCKET_FILE_NAME);
    let mut stream = UnixStream::connect(&socket_path).expect("connect to approval socket");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");

    stream
        .write_all(b"00000000000000000000000000000000\n")
        .expect("write foreign request id");
    let mut buffer = [0u8; MAX_PROMPT_MESSAGE];
    let read = stream.read(&mut buffer).expect("read verdict");
    assert_eq!(String::from_utf8_lossy(&buffer[..read]).trim(), "denied");

    assert!(
        !grants.redeem(TEST_ORIGIN, &ticket.handle),
        "申请号不匹配的批准不得放行下载"
    );

    cleanup(&ticket);
}

#[test]
fn socket_refusal_does_not_approve_the_request() {
    let grants = Grants::default();
    let ticket = grants.issue(TEST_ORIGIN).expect("issue");
    let socket_path = private_directory_path(&ticket.request_id)
        .expect("private directory")
        .join(SOCKET_FILE_NAME);
    let mut stream = UnixStream::connect(&socket_path).expect("connect to approval socket");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");

    stream
        .write_all(format!("{}\n", ticket.request_id).as_bytes())
        .expect("write request id");
    let mut buffer = [0u8; MAX_PROMPT_MESSAGE];
    let read = stream.read(&mut buffer).expect("read prompt");
    assert!(read > 0, "服务端必须先给出安全提示");
    stream.write_all(b"n\n").expect("write refusal");
    let read = stream.read(&mut buffer).expect("read verdict");
    assert_eq!(String::from_utf8_lossy(&buffer[..read]).trim(), "denied");

    assert!(
        !grants.redeem(TEST_ORIGIN, &ticket.handle),
        "用户拒绝后不得放行下载"
    );

    cleanup(&ticket);
}

#[test]
fn denied_approval_reclaims_the_pending_request_immediately() {
    let grants = Grants::default();
    let ticket = grants.issue(TEST_ORIGIN).expect("issue");
    let socket_path = private_directory_path(&ticket.request_id)
        .expect("private directory")
        .join(SOCKET_FILE_NAME);
    let mut stream = UnixStream::connect(&socket_path).expect("connect to approval socket");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");

    stream
        .write_all(format!("{}\n", ticket.request_id).as_bytes())
        .expect("write request id");
    let mut buffer = [0u8; MAX_PROMPT_MESSAGE];
    let read = stream.read(&mut buffer).expect("read prompt");
    assert!(read > 0);
    stream.write_all(b"n\n").expect("write refusal");
    let read = stream.read(&mut buffer).expect("read verdict");
    assert_eq!(String::from_utf8_lossy(&buffer[..read]).trim(), "denied");

    let mut pending = 1;
    for _ in 0..100 {
        pending = grants
            .inner
            .lock()
            .map(|guard| guard.pending.len())
            .unwrap_or(0);
        if pending == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        pending, 0,
        "被拒绝的申请必须立即回收，不得占满待批准上限"
    );

    cleanup(&ticket);
}

#[test]
fn private_directory_uses_the_fixed_short_tmp_base() {
    let grants = Grants::default();
    let ticket = grants.issue(TEST_ORIGIN).expect("issue");
    let directory = private_directory_path(&ticket.request_id).expect("private directory");

    assert_eq!(
        directory.parent(),
        Some(Path::new("/tmp")),
        "批准通道必须固定在 /tmp，不能随各进程 TMPDIR 漂移"
    );

    cleanup(&ticket);
}

#[test]
fn private_directory_is_owner_only() {
    let grants = Grants::default();
    let ticket = grants.issue(TEST_ORIGIN).expect("issue");
    let directory = private_directory_path(&ticket.request_id).expect("private directory");
    let mode = std::fs::metadata(&directory)
        .expect("directory metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700, "私有目录必须是 0700");

    cleanup(&ticket);
}

#[tokio::test]
async fn approve_local_rejects_invalid_request_ids() {
    for invalid in [
        "../../etc/passwd",
        "ABCDEF0123456789ABCDEF0123456789",
        "0123456789abcdef",
        "",
    ] {
        let error = approve_local(invalid)
            .await
            .expect_err("非法申请号必须被拒");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{invalid}");
    }
}

#[tokio::test]
async fn approve_local_fails_closed_without_a_pending_channel() {
    let error = approve_local("0123456789abcdef0123456789abcdef")
        .await
        .expect_err("没有待批准通道时必须失败");
    if std::io::stdin().is_terminal() {
        // 交互终端：走到连接步骤，通道不存在即失败。
        assert_eq!(error.kind(), io::ErrorKind::NotFound, "{error}");
    } else {
        // 无终端（CI）：必须先在终端要求处失败，不接受脚本化批准。
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{error}");
    }
}
