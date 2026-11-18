//! 宿主人工批准通道（`aria diagnostics approve <申请号>`）。
//!
//! 契约要点（openspec/changes/add-cockpit-log-bundle）：
//! - 批准只发生在宿主终端；网页端没有批准口，HTTP 也不暴露任何批准接口。
//! - 授权只驻内存：绑定 instance + origin + handle，300 秒内一次性核销。
//! - 每次申请在 OS 短临时目录（0700）里绑定一个独立 Unix socket；socket 只接同 euid 的客户端。
//! - 通道永远不递送下载句柄；批准只翻转内存里的 approved 标记。
//! - 磁盘上不写任何可被读文件接口取走的凭据，句柄不进命令、URL、日志与归档。

use std::collections::HashMap;
use std::io::{self, IsTerminal, Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// 授权有效期：5 分钟。
const GRANT_TTL: Duration = Duration::from_secs(300);
/// 单实例同时待批准的申请上限。
const MAX_PENDING_PER_INSTANCE: usize = 8;
/// 单次通道 I/O 的超时上限。
const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// 宿主在终端上明确确认的等待上限（人的输入不适用机器级 I/O 超时）。
const HUMAN_RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);
/// 申请号长度：32 位小写十六进制（公开 id）。
const REQUEST_ID_LEN: usize = 32;
/// 下载句柄长度：64 位小写十六进制（与申请号独立随机）。
const HANDLE_LEN: usize = 64;
/// 控制消息（申请号）长度上限。
const MAX_CONTROL_MESSAGE: usize = 128;
/// 安全提示长度上限。
const MAX_PROMPT_MESSAGE: usize = 1024;
/// 应答长度上限。
const MAX_REPLY_MESSAGE: usize = 16;
/// 终端应答长度上限。
const MAX_TERMINAL_ANSWER: usize = 64;
/// Unix socket 文件名。
const SOCKET_FILE_NAME: &str = "approve.sock";
/// Unix socket 路径长度上限（sun_path 约 108，留足余量）。
const MAX_SOCKET_PATH_LEN: usize = 96;
/// 私有临时目录名前缀。
const PRIVATE_DIR_PREFIX: &str = "aria-log-bundle-";
/// 监听轮询间隔。
const POLL_INTERVAL: Duration = Duration::from_millis(25);
/// 固定的批准提示正文。
const SAFETY_NOTICE: &str = "Aria 正在请求导出本次诊断日志包。\n\
内容可能包含工作区绝对路径与会话诊断轨迹（不含对话正文），请确认来源可信后再批准。\n\
本次授权自申请起 5 分钟内有效（仅当前实例、当前来源），且只能使用一次。";

/// 内存中的授权登记簿（每个实例一份）。
#[derive(Clone, Debug)]
pub struct Grants {
    inner: Arc<Mutex<GrantsInner>>,
}

#[derive(Debug)]
struct GrantsInner {
    instance_id: String,
    pending: HashMap<String, Grant>,
}

struct Grant {
    instance_id: String,
    handle: String,
    origin: String,
    approved: bool,
    expires_at: Instant,
}

impl std::fmt::Debug for Grant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Grant")
            .field("instance_id", &self.instance_id)
            .field("handle", &"<redacted>")
            .field("origin", &self.origin)
            .field("approved", &self.approved)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// 一次申请的结果：公开申请号 + 一次性下载句柄。
///
/// 句柄只交给内存登记簿与发起申请的那次 HTTP 响应；`Debug` 不打印句柄，
/// 避免它经由日志或错误回执外泄。
pub struct Ticket {
    pub(super) request_id: String,
    pub(super) handle: String,
}

impl std::fmt::Debug for Ticket {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Ticket")
            .field("request_id", &self.request_id)
            .field("handle", &"<redacted>")
            .finish()
    }
}

impl Default for Grants {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(GrantsInner {
                instance_id: random_hex(8),
                pending: HashMap::new(),
            })),
        }
    }
}

impl Grants {
    /// 登记一次新的导出申请：生成公开申请号 + 独立随机句柄，并为本次申请
    /// 在私有目录（0700）里绑定一个只能同 UID 接入的 Unix socket。
    pub(super) fn issue(&self, origin: &str) -> io::Result<Ticket> {
        self.issue_at(origin, Instant::now())
    }

    /// 同 [`Grants::issue`]，但显式给出签发时刻（测试用）。
    pub(super) fn issue_at(&self, origin: &str, now: Instant) -> io::Result<Ticket> {
        let request_id = random_hex(REQUEST_ID_LEN / 2);
        let handle = random_hex(HANDLE_LEN / 2);
        let expires_at = now + GRANT_TTL;
        let directory = private_directory_path(&request_id)?;
        let socket_path = directory.join(SOCKET_FILE_NAME);
        create_private_directory(&directory)?;
        let listener = match bind_private_socket(&socket_path) {
            Ok(listener) => listener,
            Err(error) => {
                let _ = std::fs::remove_dir(&directory);
                return Err(error);
            }
        };
        {
            let mut inner = self.lock_inner()?;
            inner.purge_expired(now);
            if inner.pending.len() >= MAX_PENDING_PER_INSTANCE {
                // 上限只增不减：不驱逐操作中的有效申请（新申请不得挤掉已批准
                // 待核销或正在等待确认的条目），交由 HTTP 层按 429 处理。
                drop(inner);
                remove_private_socket(&socket_path, &directory);
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "待批准的请求过多，请稍后重试",
                ));
            }
            let instance_id = inner.instance_id.clone();
            inner.pending.insert(
                request_id.clone(),
                Grant {
                    instance_id,
                    handle: handle.clone(),
                    origin: origin.to_string(),
                    approved: false,
                    expires_at,
                },
            );
        }
        spawn_approval_listener(
            listener,
            self.inner.clone(),
            request_id.clone(),
            origin.to_string(),
            expires_at,
            directory,
        );
        Ok(Ticket { request_id, handle })
    }

    /// 一次性核销：仅当 instance、origin、批准状态与期限全部匹配时才消费。
    ///
    /// 未批准、跨来源、过期与重放一律返回 `false`，且**不**消费既有授权。
    pub(super) fn redeem(&self, origin: &str, handle: &str) -> bool {
        self.redeem_at(origin, handle, Instant::now())
    }

    /// 同 [`Grants::redeem`]，但显式给出核销时刻（测试用）。
    pub(super) fn redeem_at(&self, origin: &str, handle: &str, now: Instant) -> bool {
        let Ok(mut inner) = self.inner.lock() else {
            return false;
        };
        inner.purge_expired(now);
        let instance_id = inner.instance_id.clone();
        let Some(request_id) = inner
            .pending
            .iter()
            .find(|(_, grant)| grant.handle == handle)
            .map(|(request_id, _)| request_id.clone())
        else {
            return false;
        };
        let Some(grant) = inner.pending.get(&request_id) else {
            return false;
        };
        if grant.instance_id != instance_id
            || grant.origin != origin
            || !grant.approved
            || now >= grant.expires_at
        {
            return false;
        }
        inner.pending.remove(&request_id);
        true
    }

    fn lock_inner(&self) -> io::Result<MutexGuard<'_, GrantsInner>> {
        self.inner
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "授权登记簿已损坏"))
    }
}

impl GrantsInner {
    fn purge_expired(&mut self, now: Instant) {
        self.pending.retain(|_, grant| now < grant.expires_at);
    }
}

/// 宿主人工批准入口：连接本次申请的私有 socket，展示固定安全提示，
/// 由用户在 TTY 上明确输入 `y` 之后，才把内存里的申请标记为已批准。
pub(crate) async fn approve_local(request_id: &str) -> io::Result<()> {
    if !is_valid_request_id(request_id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "申请号格式无效（应为 32 位小写十六进制）",
        ));
    }
    if !std::io::stdin().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "批准必须在交互式终端执行",
        ));
    }
    let directory = private_directory_path(request_id)?;
    let socket_path = directory.join(SOCKET_FILE_NAME);
    let mut stream = tokio::net::UnixStream::connect(&socket_path)
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "没有找到待批准的本机通道（申请号可能已过期）",
            )
        })?;
    {
        let peer = peer_uid(&stream).map_err(|_| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "无法核验批准通道的宿主身份",
            )
        })?;
        if peer != current_euid() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "批准通道的宿主身份校验失败",
            ));
        }
    }
    let deadline = tokio::time::Instant::now() + IO_TIMEOUT;
    let mut request = Vec::with_capacity(REQUEST_ID_LEN + 1);
    request.extend_from_slice(request_id.as_bytes());
    request.push(b'\n');
    write_with_deadline(&mut stream, &request, deadline).await?;
    let mut buffer = vec![0u8; MAX_PROMPT_MESSAGE];
    let read = read_with_deadline(&mut stream, &mut buffer, deadline).await?;
    if read == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "批准通道提前关闭",
        ));
    }
    let prompt = String::from_utf8_lossy(&buffer[..read]).to_string();
    if !prompt.contains(request_id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "批准通道返回的申请号不匹配",
        ));
    }
    print!("{prompt}\n输入 y 确认批准本次日志下载（其它输入取消）: ");
    let _ = io::stdout().flush();
    let answer = tokio::time::timeout(
        HUMAN_RESPONSE_TIMEOUT,
        tokio::task::spawn_blocking(read_terminal_answer),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "终端确认超时"))?
    .map_err(|_| io::Error::new(io::ErrorKind::Other, "读取终端输入失败"))??;
    if answer.trim() != "y" {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "未确认，本次日志下载没有被批准",
        ));
    }
    // 人确认可能耗时，确认之后重新起算通道 I/O 期限。
    let verdict_deadline = tokio::time::Instant::now() + IO_TIMEOUT;
    write_with_deadline(&mut stream, b"y\n", verdict_deadline).await?;
    let mut verdict = vec![0u8; MAX_REPLY_MESSAGE];
    let read = read_with_deadline(&mut stream, &mut verdict, verdict_deadline).await?;
    let verdict = String::from_utf8_lossy(&verdict[..read]).trim().to_string();
    if verdict != "ok" {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "批准未生效",
        ));
    }
    Ok(())
}

/// 批准通道的处理结果。
#[derive(Debug, PartialEq, Eq)]
enum ApprovalOutcome {
    Approved,
    Denied,
    Expired,
    Failed,
}

fn spawn_approval_listener(
    listener: UnixListener,
    inner: Arc<Mutex<GrantsInner>>,
    request_id: String,
    origin: String,
    expires_at: Instant,
    directory: PathBuf,
) {
    let socket_path = directory.join(SOCKET_FILE_NAME);
    let cleanup_inner = inner.clone();
    let cleanup_request_id = request_id.clone();
    let cleanup_socket_path = socket_path.clone();
    let cleanup_directory = directory.clone();
    let spawn = std::thread::Builder::new()
        .name("aria-log-bundle-approve".to_string())
        .spawn(move || {
            let outcome = serve_approval(&listener, &inner, &request_id, &origin, expires_at);
            if outcome != ApprovalOutcome::Approved {
                // 取消、失败与到期都已无法再批准：立即回收申请条目，
                // 免得占满待批准上限（socket 与目录一并清理）。
                if let Ok(mut guard) = inner.lock() {
                    guard.pending.remove(&request_id);
                }
            }
            remove_private_socket(&socket_path, &directory);
        });
    if spawn.is_err() {
        // 线程起不来：撤掉刚登记的申请并回收目录（fail closed）。
        if let Ok(mut guard) = cleanup_inner.lock() {
            guard.pending.remove(&cleanup_request_id);
        }
        remove_private_socket(&cleanup_socket_path, &cleanup_directory);
    }
}

fn serve_approval(
    listener: &UnixListener,
    inner: &Arc<Mutex<GrantsInner>>,
    request_id: &str,
    origin: &str,
    expires_at: Instant,
) -> ApprovalOutcome {
    if listener.set_nonblocking(true).is_err() {
        return ApprovalOutcome::Failed;
    }
    loop {
        if Instant::now() >= expires_at {
            return ApprovalOutcome::Expired;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
                let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
                match peer_uid(&stream) {
                    Ok(uid) if uid == current_euid() => {
                        return handle_approval_client(stream, inner, request_id, origin);
                    }
                    _ => {
                        // 非同 UID 客户端：直接断开，不展示提示、不消费任何状态。
                        drop(stream);
                        continue;
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(_) => return ApprovalOutcome::Failed,
        }
    }
}

fn handle_approval_client(
    mut stream: UnixStream,
    inner: &Arc<Mutex<GrantsInner>>,
    request_id: &str,
    origin: &str,
) -> ApprovalOutcome {
    let deadline = Instant::now() + IO_TIMEOUT;
    let claimed = match read_line_bounded(&mut stream, MAX_CONTROL_MESSAGE, deadline) {
        Ok(line) => line,
        Err(_) => return ApprovalOutcome::Failed,
    };
    if claimed != request_id {
        let _ = stream.write_all(b"denied\n");
        return ApprovalOutcome::Denied;
    }
    let prompt = build_prompt(origin, request_id);
    if stream.write_all(prompt.as_bytes()).is_err() {
        return ApprovalOutcome::Failed;
    }
    // 人的确认等待必须覆盖 accept 处设置的机器级读超时（SO_RCVTIMEO）。
    let _ = stream.set_read_timeout(Some(HUMAN_RESPONSE_TIMEOUT));
    let answer = match read_line_bounded(
        &mut stream,
        MAX_REPLY_MESSAGE,
        Instant::now() + HUMAN_RESPONSE_TIMEOUT,
    ) {
        Ok(line) => line,
        Err(_) => return ApprovalOutcome::Failed,
    };
    if answer != "y" {
        let _ = stream.write_all(b"denied\n");
        return ApprovalOutcome::Denied;
    }
    if mark_approved(inner, request_id, Instant::now()) {
        let _ = stream.write_all(b"ok\n");
        ApprovalOutcome::Approved
    } else {
        let _ = stream.write_all(b"denied\n");
        ApprovalOutcome::Failed
    }
}

fn mark_approved(inner: &Arc<Mutex<GrantsInner>>, request_id: &str, now: Instant) -> bool {
    let Ok(mut guard) = inner.lock() else {
        return false;
    };
    let instance_id = guard.instance_id.clone();
    let Some(grant) = guard.pending.get_mut(request_id) else {
        return false;
    };
    if grant.instance_id != instance_id || grant.approved || now >= grant.expires_at {
        return false;
    }
    grant.approved = true;
    true
}

fn build_prompt(origin: &str, request_id: &str) -> String {
    format!(
        "{SAFETY_NOTICE}\norigin: {}\nrequest_id: {request_id}\n",
        truncate(origin, 256)
    )
}

fn read_line_bounded(
    stream: &mut UnixStream,
    max_bytes: usize,
    deadline: Instant,
) -> io::Result<String> {
    let mut collected: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 64];
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "批准通道读取超时"));
        }
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        collected.extend_from_slice(&chunk[..read]);
        if collected.contains(&b'\n') {
            break;
        }
        if collected.len() > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "批准通道消息超长",
            ));
        }
    }
    if collected.len() > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "批准通道消息超长",
        ));
    }
    Ok(String::from_utf8_lossy(&collected).trim().to_string())
}

fn read_terminal_answer() -> io::Result<String> {
    let mut stdin = io::stdin();
    let mut collected: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 32];
    loop {
        let read = stdin.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        collected.extend_from_slice(&chunk[..read]);
        if collected.contains(&b'\n') || collected.len() > MAX_TERMINAL_ANSWER {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&collected).to_string())
}

async fn write_with_deadline(
    stream: &mut tokio::net::UnixStream,
    payload: &[u8],
    deadline: tokio::time::Instant,
) -> io::Result<()> {
    tokio::time::timeout_at(deadline, stream.write_all(payload))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "批准通道写入超时"))?
}

async fn read_with_deadline(
    stream: &mut tokio::net::UnixStream,
    buffer: &mut [u8],
    deadline: tokio::time::Instant,
) -> io::Result<usize> {
    tokio::time::timeout_at(deadline, stream.read(buffer))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "批准通道读取超时"))?
}

fn private_directory_path(request_id: &str) -> io::Result<PathBuf> {
    // 固定短基线 /tmp：与 daemon socket（/tmp/aria-daemon）同惯例。
    // 不取进程级 TMPDIR —— 后台服务与宿主终端的 TMPDIR 可能不同，
    // 也会撞上 macOS 的长 TMPDIR 与 sun_path 104 字节上限。
    let directory = Path::new("/tmp").join(format!("{PRIVATE_DIR_PREFIX}{request_id}"));
    let socket_path = directory.join(SOCKET_FILE_NAME);
    if socket_path.as_os_str().len() > MAX_SOCKET_PATH_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "临时路径过长，无法建立批准通道",
        ));
    }
    Ok(directory)
}

fn create_private_directory(directory: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700);
    builder.create(directory)?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn bind_private_socket(path: &Path) -> io::Result<UnixListener> {
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

fn remove_private_socket(socket_path: &Path, directory: &Path) {
    let _ = std::fs::remove_file(socket_path);
    let _ = std::fs::remove_dir(directory);
}

fn is_valid_request_id(value: &str) -> bool {
    value.len() == REQUEST_ID_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn random_hex(bytes: usize) -> String {
    let mut value = String::with_capacity(bytes * 2);
    while value.len() < bytes * 2 {
        value.push_str(&uuid::Uuid::new_v4().simple().to_string());
    }
    value.truncate(bytes * 2);
    value
}

fn truncate(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

#[cfg(target_os = "linux")]
fn peer_uid(stream: &impl AsRawFd) -> io::Result<u32> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut credentials as *mut libc::ucred as *mut libc::c_void,
            &mut length,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(credentials.uid)
}

#[cfg(target_os = "macos")]
fn peer_uid(stream: &impl AsRawFd) -> io::Result<u32> {
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    let result = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(uid)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn peer_uid(_stream: &impl AsRawFd) -> io::Result<u32> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "当前平台不支持本机身份核验",
    ))
}

fn current_euid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    include!("permit_tests.rs");
}
