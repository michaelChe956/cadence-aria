#!/usr/bin/env python3
"""WIG 真机 8 臂串行对照装置（唯一变量 = 二进制）。

协议闭环（从 src 逐点核对，非猜测；与既有真实链脚本
cadence/reports/provider-validation-round/wave2-2c/story_terminate.mjs 同形）：

  1. POST /api/projects/{p}/issues —— 建 issue（带 logical_codebase_id）。
  2. POST .../story-specs:generate —— **只是 prepare**：落 draft story spec
     （confirmation_status=draft）+ open 状态的 story session，
     **不启动任何 provider**（src/web/handlers/lifecycle.rs
     generate_story_specs 只 create_story_spec + create_workspace_session；
     2026-11-13 真机冒烟 /tmp/aria-wig-smoke/run-smoke-4701 佐证：返回 200 后
     30 分钟仍 status=open、current_version=null、provider_start_ledger=[]、
     无 timeline——等待/加超时都无济于事，缺的是显式 start）。
  3. 真实启动走 WS：GET /api/workspace-sessions/{id}/ws（app.rs:670）
     → {"type":"hello","session_id":...,"last_seen_node_id":null}
     → 首帧 session_state.stage=prepare_context 时发
       {"type":"start_generation","provider_config":{...},"reviewer_enabled":false}
     （stage 守卫 src/web/workspace_ws_handler/protocol.rs
      is_message_valid_for_stage_with_flow：PrepareContext 放行 StartGeneration；
      引擎侧 src/product/workspace_engine/lifecycle.rs start_generation 经
      provider_drive 驱动真实 provider）。
  4. stage 到 author_confirm（story 的确认门）后，approve 走 HTTP：
     POST /api/workspace-sessions/{id}/confirm {"confirmed_by","with_review":false}
     （protocol.rs AuthorConfirm 注释：story/design 的 approve 走 HTTP confirm
      端点，WS Confirm 帧不放行；在途 Running 会被 F-31 守卫 409 拒收，故只在
      到门后发，另带小步重试覆盖 durable 残留窗口）。
  5. durable 验证：story spec confirmation_status=confirmed、current_version
     非空、session.provider_start_ledger 非空（= 真实 provider 启动证据）。

固定输入（消灭第二变量）：story seed 一次性生成（--seed，controller 授权的
真实 provider run，只跑一次），8 臂全部克隆同一 seed 快照；**不允许**每臂各自
生成 story。原 3 成员「公告删除」案例的历史 seed 已不可恢复（见 README
「原案例证据现状」），本装置重建的 seed ≠ 原验收输入。

权限/信任门纪律：permission_modes 取 auto；若服务端发来 permission_request /
choice_request / trust 指引等未知门，本装置**明确报阻塞并判该臂失败**，绝不
自动批准，也不发 register_trust_entry（不写任何 HOME 级用户配置）。

产物纪律：只写 --out（/tmp）；已存在的 run/seed 目录一律拒绝覆盖（失败证据
不删除）；任一臂超时/异常 ⇒ 进程退出码非 0。

用法：
  # 一次性固定输入（也是 controller 的真实 smoke 入口）
  python3 run_arms.py --seed <aria二进制> <3成员案例工作区> <out根> [端口]
  # 8 臂 ABBA（4 before + 4 after），唯一变量 = 二进制
  python3 run_arms.py <before二进制> <after二进制> <seed工作区> <out根> 2 [起始端口]
"""
import base64
import hashlib
import json
import os
import shutil
import signal
import socket
import struct
import subprocess
import sys
import time

PROJECT = "project_0001"
PROVIDER = "codex"
TITLE = "公告删除"
TERMINAL = {"confirmed", "stopped_needs_human", "failed", "rejected",
            "terminated", "abandoned"}
GATE_STAGES = {"author_confirm", "human_confirm"}
ARM_TIMEOUT_S = 60 * 60          # 单臂总预算（沿用 11-09 装置量级，未加大）
CONFIRM_RETRY_N = 6              # F-31 在途守卫的小步重试（409 in flight）
CONFIRM_RETRY_WAIT_S = 5
REWRITE_EXT = {".json", ".jsonl", ".md", ".txt", ".toml", ".yaml", ".yml"}


def http(method, url, body=None, timeout=60):
    """走 curl：mutation 必须带受信精确 Origin（loopback 绑定 ⇒ 精确回环 origin）。

    历史归因更正：旧 403 不是「urllib 丢头」，而是 origin 拼接双冒号
    （"http:" + "//host" ⇒ "http:://host"）。此处先剥 scheme 尾冒号再拼。
    """
    parts = url.split("/", 3)
    scheme = parts[0].rstrip(":") if len(parts) >= 3 else ""
    origin = "%s://%s" % (scheme, parts[2]) if len(parts) >= 3 else ""
    cmd = ["curl", "-sS", "-X", method, url, "-H", "Origin: " + origin,
           "-w", "\n%{http_code}"]
    if body is not None:
        cmd += ["-H", "content-type: application/json",
                "--data-binary", json.dumps(body)]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout + 15)
    raw, _, code = proc.stdout.rpartition("\n")
    code = int(code) if code.strip().isdigit() else 0
    try:
        return code, json.loads(raw)
    except json.JSONDecodeError:
        return code, raw


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def sha256_bin(path):
    try:
        return sha256_file(path)
    except OSError as e:
        raise SystemExit("读二进制失败 %s: %s" % (path, e))


def read_json(path):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return None


def wait_health(port, proc, timeout=180):
    end = time.time() + timeout
    while time.time() < end:
        if proc.poll() is not None:
            raise SystemExit("daemon 提前退出 rc=%s" % proc.returncode)
        try:
            st, _ = http("GET", "http://127.0.0.1:%d/api/health" % port, timeout=5)
            if st == 200:
                return
        except Exception:
            pass
        time.sleep(1)
    raise SystemExit("health 超时")


def discover_lc(run):
    """从工作区状态动态发现逻辑代码库与其活跃成员（不写死 id）。"""
    base = os.path.join(run, ".aria", "projects", PROJECT, "logical-codebases")
    for name in sorted(os.listdir(base)):
        lc_dir = os.path.join(base, name)
        members_dir = os.path.join(lc_dir, "members")
        if not os.path.isdir(members_dir):
            continue
        logical_ids, physical_id = [], None
        for m in sorted(os.listdir(members_dir)):
            rec = read_json(os.path.join(members_dir, m))
            if not rec or rec.get("status") != "active":
                continue
            logical_ids.append(rec.get("logical_repository_id"))
            physical_id = physical_id or rec.get("physical_repository_id")
        if logical_ids:
            return name, logical_ids, physical_id
    raise SystemExit("工作区里找不到带活跃成员的逻辑代码库")


def repoint_paths(run, src_root):
    """把副本状态里指向源工作区的绝对路径改指到本副本。

    不改则逻辑代码库 aggregate_root 仍指向源工作区 ⇒ 多臂写同一棵树、跨臂
    污染 ⇒ 差异不可归因（装置失效）。
    """
    n = 0
    for root, _dirs, files in os.walk(os.path.join(run, ".aria")):
        for name in files:
            if os.path.splitext(name)[1] not in REWRITE_EXT:
                continue
            path = os.path.join(root, name)
            try:
                with open(path, encoding="utf-8") as f:
                    text = f.read()
            except (UnicodeDecodeError, OSError):
                continue
            if src_root not in text:
                continue
            with open(path, "w", encoding="utf-8") as f:
                f.write(text.replace(src_root, run))
            n += 1
    return n


def member_heads(root):
    """工作区内全部 git 仓（成员仓/聚合视图）的 HEAD 快照（输入身份之一）。"""
    heads = {}
    if not os.path.isdir(root):
        return heads
    for name in sorted(os.listdir(root)):
        git_dir = os.path.join(root, name, ".git")
        if not os.path.isdir(git_dir) and not os.path.isfile(git_dir):
            continue
        try:
            out = subprocess.run(["git", "-C", os.path.join(root, name),
                                  "rev-parse", "HEAD"],
                                 capture_output=True, text=True, timeout=60)
            heads[name] = out.stdout.strip() if out.returncode == 0 else "<git-error>"
        except Exception:
            heads[name] = "<git-error>"
    return heads


# ───────────────────────── 极简 WS 客户端（stdlib，RFC6455 文本帧） ─────────────────────────

class WsError(Exception):
    pass


class WsClosed(WsError):
    pass


class WsClient:
    def __init__(self, host, port, path, origin, handshake_timeout=30):
        self.sock = socket.create_connection((host, port), timeout=handshake_timeout)
        key = base64.b64encode(os.urandom(16)).decode()
        req = ("GET %s HTTP/1.1\r\n"
               "Host: %s:%d\r\n"
               "Upgrade: websocket\r\n"
               "Connection: Upgrade\r\n"
               "Sec-WebSocket-Key: %s\r\n"
               "Sec-WebSocket-Version: 13\r\n"
               "Origin: %s\r\n\r\n") % (path, host, port, key, origin)
        self.sock.sendall(req.encode())
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise WsError("ws 握手中断")
            buf += chunk
        head, rest = buf.split(b"\r\n\r\n", 1)
        if b" 101 " not in head.split(b"\r\n")[0]:
            raise WsError("ws 握手被拒: %s" % head[:200].decode("utf-8", "replace"))
        self._buf = rest
        self._frag = []  # 跨 recv_text 调用的半消息分片（pending message state）

    def _read_exact(self, n, deadline):
        while len(self._buf) < n:
            remain = deadline - time.time()
            if remain <= 0:
                # 预算耗尽：返回 None 让上层以「本轮无消息」收口；
                # 已收字节留在 self._buf，下轮继续拼同一帧（不丢流同步）。
                return None
            self.sock.settimeout(max(0.5, min(30.0, remain)))
            try:
                chunk = self.sock.recv(1 << 16)
            except socket.timeout:
                return None if not self._buf else False
            if not chunk:
                raise WsClosed("连接关闭")
            self._buf += chunk
        return True

    def _read_frame(self, deadline):
        """解析一条完整帧；帧未凑齐时**不消费任何字节**，可跨超时安全重试。

        回归修复（controller 探针 2026-11-13）：旧实现逐段消费 _buf（先切掉
        2 字节头再等载荷），payload 半齐时头已丢失，下次把载荷首字节当头 ⇒
        流永久错位。现在先窥视（peek）算出整帧长度，凑齐后一次性切出。

        返回 (fin, opcode, payload)；None=预算耗尽/空轮（上层收口）；
        False=有部分缓冲且仍在预算内（上层继续等，缓冲原样保留）。
        """
        ok = self._read_exact(2, deadline)
        if ok is not True:
            return ok
        b1, b2 = self._buf[0], self._buf[1]
        masked = b2 >> 7
        ln7 = b2 & 0x7F
        ext = 2 if ln7 == 126 else (8 if ln7 == 127 else 0)
        head = 2 + ext + (4 if masked else 0)
        ok = self._read_exact(head, deadline)
        if ok is not True:
            return ok
        if ext == 2:
            ln = struct.unpack(">H", self._buf[2:4])[0]
        elif ext == 8:
            ln = struct.unpack(">Q", self._buf[2:10])[0]
        else:
            ln = ln7
        if ln > (1 << 26):
            raise WsError("帧过大: %d" % ln)
        ok = self._read_exact(head + ln, deadline)
        if ok is not True:
            return ok
        frame = self._buf[:head + ln]
        self._buf = self._buf[head + ln:]
        fin, opcode = b1 >> 7, b1 & 0x0F
        off = 2 + ext
        mask = frame[off:off + 4] if masked else b""
        if masked:
            off += 4
        payload = frame[off:]
        if masked:
            payload = bytes(c ^ mask[i % 4] for i, c in enumerate(payload))
        return (fin, opcode, payload)

    def _send_frame(self, opcode, payload):
        header = bytes([0x80 | opcode])
        n = len(payload)
        mask = os.urandom(4)
        if n < 126:
            header += bytes([0x80 | n])
        elif n < (1 << 16):
            header += bytes([0x80 | 126]) + struct.pack(">H", n)
        else:
            header += bytes([0x80 | 127]) + struct.pack(">Q", n)
        header += mask
        masked = bytes(c ^ mask[i % 4] for i, c in enumerate(payload))
        self.sock.sendall(header + masked)

    def send_text(self, text):
        self._send_frame(0x1, text.encode("utf-8"))

    def recv_text(self, timeout_s):
        """收一条完整文本消息；超时返回 None；ping 自动回 pong。

        跨超时续拼：已收分片保存在 self._frag（实例级 pending message
        state），超时返回 None 不丢分片，下一次调用继续拼同一条消息
        （controller 闭集修复 2026-11-13：局部 parts 会把半消息丢成流洞）。
        """
        deadline = time.time() + timeout_s
        while True:
            frame = self._read_frame(deadline)
            if frame is None:
                return None
            if frame is False:
                continue  # 帧不完整但仍在预算内：整帧未消费，继续等
            fin, opcode, payload = frame
            if opcode == 0x9:  # ping → pong
                self._send_frame(0xA, payload)
                continue
            if opcode == 0x8:  # close
                try:
                    self._send_frame(0x8, payload[:2])
                except Exception:
                    pass
                raise WsClosed("服务端关闭: %s" % payload[:120].decode("utf-8", "replace"))
            if opcode in (0x0, 0x1, 0x2):  # continuation / text / binary
                self._frag.append(payload)
                if fin:
                    msg = b"".join(self._frag)
                    self._frag = []
                    return msg.decode("utf-8", "replace")
            # 其余控制帧（0xA pong 等）：忽略

    def close(self):
        try:
            self._send_frame(0x8, struct.pack(">H", 1000))
        except Exception:
            pass
        try:
            self.sock.close()
        except Exception:
            pass


# ───────────────────────── 日志 ─────────────────────────

log_path = ""


def log(msg):
    line = "[%s] %s" % (time.strftime("%H:%M:%S"), msg)
    print(line, flush=True)
    if log_path:
        with open(log_path, "a", encoding="utf-8") as f:
            f.write(line + "\n")


# ───────────────────────── daemon ─────────────────────────

def start_daemon(binary, run, port):
    daemon_log = os.path.join(run, "daemon.log")
    return subprocess.Popen(
        [binary, "web", "--workspace", run, "--port", str(port),
         "--work-item-plan-single-candidate"],
        stdout=open(daemon_log, "wb"), stderr=subprocess.STDOUT,
        cwd=run, start_new_session=True)


def stop_daemon(proc):
    try:
        os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
    except Exception:
        proc.terminate()
    try:
        proc.wait(timeout=30)
    except Exception:
        try:
            os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
        except Exception:
            pass


# ───────────────────────── story 协议闭环 ─────────────────────────

def issue_paths(run, issue_id):
    root = os.path.join(run, ".aria", "projects", PROJECT, "issues", issue_id)
    return {
        "root": root,
        "enrollment": os.path.join(root, "automation-enrollment.json"),
        "sessions": os.path.join(root, "workspace-sessions"),
        "story": lambda sid: os.path.join(root, "story-specs", "%s.json" % sid),
    }


def create_issue(port, lc_id, logical_ids, physical_id):
    base = "http://127.0.0.1:%d/api/projects/%s/issues" % (port, PROJECT)
    st, body = http("POST", base, {
        "title": TITLE,
        "description": "删除公告",
        "logical_codebase_id": lc_id,
        "repository_id": physical_id,
        "focus_repository_ids": logical_ids,
    })
    log("  建 issue: %s %s" % (st, str(body)[:300]))
    if st not in (200, 201):
        raise SystemExit("建 issue 失败")
    issue_id = ""
    issue_rec = body.get("issue") if isinstance(body, dict) else None
    if isinstance(issue_rec, dict):
        issue_id = issue_rec.get("id") or ""
    if not issue_id and isinstance(body, dict):
        issue_id = body.get("id") or body.get("issue_id") or ""
    if not issue_id:
        raise SystemExit("建 issue 响应缺 issue_id: %s" % str(body)[:200])
    return issue_id


def generate_story_session(port, issue_id):
    """story-specs:generate = prepare（draft + open session，无 provider 启动）。"""
    url = "http://127.0.0.1:%d/api/projects/%s/issues/%s/story-specs:generate" % (
        port, PROJECT, issue_id)
    st, body = http("POST", url, {
        "title": TITLE,
        "author_provider": PROVIDER,
        "reviewer_provider": PROVIDER,
        "review_rounds": 1,
        "superpowers_enabled": True,
        "openspec_enabled": True,
    })
    log("  story-specs:generate(prepare): %s %s" % (st, str(body)[:200]))
    if st not in (200, 201):
        raise SystemExit("story spec prepare 失败")
    specs = (body or {}).get("story_specs") or []
    if not specs:
        raise SystemExit("story spec prepare 返回为空")
    story_id = specs[0].get("story_spec_id") or specs[0].get("id")
    sess = (body or {}).get("workspace_session") or {}
    session_id = (sess.get("workspace_session_id") or sess.get("session_id")
                  or sess.get("id"))
    if not story_id or not session_id:
        raise SystemExit("prepare 响应缺 story_spec_id/workspace_session: %s"
                         % str(body)[:300])
    return story_id, session_id


def drive_story_to_confirmed(run, port, issue_id, session_id, story_id,
                             frames_path, budget_s=ARM_TIMEOUT_S):
    """WS hello → start_generation → 等确认门 → HTTP confirm → durable 验证。

    返回证据 dict；任何未知权限/信任/选择门 ⇒ SystemExit（明确阻塞，不自动批准）。
    """
    origin = "http://127.0.0.1:%d" % port
    cli = WsClient("127.0.0.1", port,
                   "/api/workspace-sessions/%s/ws" % session_id, origin)
    stage_seq, blocked, gate_stage, started = [], None, None, False
    frames_fh = open(frames_path, "a", encoding="utf-8")

    def record(entry):
        frames_fh.write(json.dumps(entry, ensure_ascii=False) + "\n")
        frames_fh.flush()

    record({"event": "ws_open", "session_id": session_id})
    try:
        cli.send_text(json.dumps({"type": "hello", "session_id": session_id,
                                  "last_seen_node_id": None}))
        deadline = time.time() + budget_s
        while time.time() < deadline:
            try:
                raw = cli.recv_text(30)
            except WsClosed as e:
                raise SystemExit("story WS 被服务端关闭: %s" % e)
            if raw is None:
                continue
            try:
                msg = json.loads(raw)
            except json.JSONDecodeError:
                record({"event": "non_json_frame", "len": len(raw)})
                continue
            t = msg.get("type")
            if t == "session_state" and msg.get("stage"):
                stage = msg["stage"]
                if not stage_seq or stage_seq[-1] != stage:
                    stage_seq.append(stage)
                record({"event": "session_state", "stage": stage})
                if stage == "prepare_context" and not started:
                    started = True
                    payload = {"type": "start_generation",
                               "provider_config": {
                                   "author": PROVIDER,
                                   "reviewer": PROVIDER,
                                   "review_rounds": 1,
                                   "permission_modes": {"author": "auto",
                                                        "reviewer": "auto"}},
                               "reviewer_enabled": False}
                    cli.send_text(json.dumps(payload))
                    record({"event": "start_generation_sent",
                            "reviewer_enabled": False,
                            "provider_config": payload["provider_config"]})
                elif stage in GATE_STAGES and gate_stage is None:
                    gate_stage = stage
                    break
            elif t == "stage_change" and msg.get("stage"):
                stage = msg["stage"]
                if not stage_seq or stage_seq[-1] != stage:
                    stage_seq.append(stage)
                record({"event": "stage_change", "stage": stage})
                if stage in GATE_STAGES and gate_stage is None:
                    gate_stage = stage
                    break
            elif t in ("error", "protocol_error"):
                blocked = "服务端 %s: code=%s message=%s" % (
                    t, msg.get("code"), str(msg.get("message"))[:300])
                record({"event": "server_reject", "code": msg.get("code"),
                        "message": str(msg.get("message"))[:300]})
                break
            elif t == "permission_request":
                blocked = ("permission_request(tool=%s) 到达：装置不自动批准"
                           "未知权限门（需 controller 裁决授权面）"
                           % msg.get("tool_name"))
                record({"event": "permission_request",
                        "tool_name": msg.get("tool_name"), "id": msg.get("id")})
                break
            elif t == "choice_request":
                blocked = ("choice_request(id=%s) 到达：装置不自动作答未知选择门"
                           % msg.get("id"))
                record({"event": "choice_request", "id": msg.get("id")})
                break
            else:
                record({"event": "frame", "type": t})
        if blocked:
            raise SystemExit("story 驱动被阻塞: %s" % blocked)
        if gate_stage is None:
            raise SystemExit("story 生成未到确认门（%ds 预算耗尽，最后 stage=%s）"
                             % (budget_s, stage_seq[-1] if stage_seq else "无"))
        log("  确认门: stage=%s" % gate_stage)

        # approve 走 HTTP confirm（story 门上 WS Confirm 帧不放行）
        url = "http://127.0.0.1:%d/api/workspace-sessions/%s/confirm" % (port, session_id)
        st, body = None, None
        for _ in range(CONFIRM_RETRY_N):
            st, body = http("POST", url, {"confirmed_by": "wig-harness",
                                          "with_review": False})
            record({"event": "http_confirm", "status": st,
                    "body": str(body)[:200]})
            if st in (200, 201):
                break
            if st == 409:  # F-31 在途守卫：durable 残留 Running，小步重试
                time.sleep(CONFIRM_RETRY_WAIT_S)
                continue
            raise SystemExit("HTTP confirm 失败: %s %s" % (st, str(body)[:300]))
        if st not in (200, 201):
            raise SystemExit("HTTP confirm 重试后仍失败: %s %s" % (st, str(body)[:300]))
    finally:
        cli.close()
        frames_fh.close()

    # durable 验证：spec confirmed + 版本 + 真实 provider 启动账目
    spec_path = os.path.join(run, ".aria", "projects", PROJECT, "issues",
                             issue_id, "story-specs", "%s.json" % story_id)
    spec = None
    end = time.time() + 120
    while time.time() < end:
        spec = read_json(spec_path)
        if spec and spec.get("confirmation_status") == "confirmed" \
                and spec.get("current_version"):
            break
        time.sleep(2)
    if not spec or spec.get("confirmation_status") != "confirmed" \
            or not spec.get("current_version"):
        raise SystemExit("story spec 未进入 confirmed（durable 验证失败）")
    session_json = os.path.join(run, ".aria", "projects", PROJECT, "issues",
                                issue_id, "workspace-sessions",
                                "%s.json" % session_id)
    rec = read_json(session_json) or {}
    ledger = rec.get("provider_start_ledger") or []
    return {"session_id": session_id, "story_spec_id": story_id,
            "story_version": spec.get("current_version"),
            "gate_stage": gate_stage, "stage_seq": stage_seq,
            "provider_start_ledger": len(ledger),
            "session_status": rec.get("status"),
            "story_input_sha256": sha256_file(spec_path)}





def wait_terminated(run, issue_id, timeout=180):
    """enrollment 存在时等 terminated_at 落盘再 +2s；不存在（seed 克隆首臂）直接过。

    间隔不足时 Enable 会被判为「非 terminated 的常规重开」：command_id 不被消费、
    prepare_intent_id 不轮换、新 plan 不建立（11-09 装置已踩实的坑）。
    """
    path = issue_paths(run, issue_id)["enrollment"]
    if not os.path.exists(path):
        log("  enrollment 不存在（fresh seed 克隆），跳过 terminated 等待")
        time.sleep(2)
        return
    end = time.time() + timeout
    while time.time() < end:
        rec = read_json(path)
        if rec and rec.get("terminated_at"):
            time.sleep(2)
            return
        time.sleep(1)
    raise SystemExit("terminated_at 未落盘")


def wait_terminal(run, issue_id, timeout=ARM_TIMEOUT_S):
    """等 automation 建立的 plan 会话（id 含 auto_）进终态。返回 (status, session_id)。"""
    sess_dir = issue_paths(run, issue_id)["sessions"]
    end = time.time() + timeout
    seen, sid = None, None
    while time.time() < end:
        if os.path.isdir(sess_dir):
            for name in sorted(os.listdir(sess_dir)):
                if "auto_" not in name or not name.endswith(".json"):
                    continue
                rec = read_json(os.path.join(sess_dir, name))
                if not rec:
                    continue
                seen, sid = rec.get("status"), name[:-5]
                if seen in TERMINAL:
                    return seen, sid
        time.sleep(5)
    return seen or "timeout", sid


# ───────────────────────── seed（一次性固定输入，兼 controller smoke） ─────────────────────────

def make_seed(binary, src, out_root, port):
    seed_ws = os.path.join(out_root, "seed-workspace")
    if os.path.exists(seed_ws):
        raise SystemExit("seed 目录已存在，拒绝覆盖（证据保护）: %s" % seed_ws)
    os.makedirs(out_root, exist_ok=True)
    log("=== seed: 复制案例工作区 %s -> %s" % (src, seed_ws))
    shutil.copytree(src, seed_ws, symlinks=True)
    n = repoint_paths(seed_ws, src)
    log("  路径重指向: %d 个文件" % n)
    heads = member_heads(seed_ws)
    log("  成员 heads: %s" % json.dumps(heads, ensure_ascii=False))
    proc = start_daemon(binary, seed_ws, port)
    try:
        wait_health(port, proc)
        lc_id, logical_ids, physical_id = discover_lc(seed_ws)
        log("  逻辑代码库: %s 活跃成员=%d" % (lc_id, len(logical_ids)))
        issue_id = create_issue(port, lc_id, logical_ids, physical_id)
        log("  issue_id=%s" % issue_id)
        story_id, session_id = generate_story_session(port, issue_id)
        log("  story_spec_id=%s story_session=%s（prepare 完成，尚未启动 provider）"
            % (story_id, session_id))
        ev = drive_story_to_confirmed(
            seed_ws, port, issue_id, session_id, story_id,
            os.path.join(seed_ws, "story-ws-frames.jsonl"))
        log("  story confirmed: version=%s provider_start_ledger=%d"
            % (ev["story_version"], ev["provider_start_ledger"]))
        if ev["provider_start_ledger"] <= 0:
            raise SystemExit("provider_start_ledger 为空：story 未发生真实 provider 启动")
    finally:
        stop_daemon(proc)
    manifest = {
        "schema": "wig-seed-manifest/1",
        "mode": "seed",
        "created_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "binary": binary,
        "binary_sha256": sha256_bin(binary),
        "src": src,
        "project_id": PROJECT,
        "issue_id": issue_id,
        "story_spec_id": ev["story_spec_id"],
        "story_version": ev["story_version"],
        "story_session_id": ev["session_id"],
        "story_input_sha256": ev["story_input_sha256"],
        "gate_stage": ev["gate_stage"],
        "provider_start_ledger": ev["provider_start_ledger"],
        "member_heads": heads,
        "active_members": len(logical_ids),
    }
    mpath = os.path.join(out_root, "seed-manifest.json")
    with open(mpath, "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=1)
    log("seed 完成：%s" % mpath)
    print(json.dumps({k: manifest[k] for k in
                      ("issue_id", "story_spec_id", "story_version",
                       "story_session_id", "provider_start_ledger",
                       "gate_stage", "binary_sha256", "active_members")},
                     ensure_ascii=False, indent=1))
    return manifest


# ───────────────────────── 单臂 ─────────────────────────

def run_arm(binary, tag, arm, port, seed_ws, seed_manifest, out_root):
    run = os.path.join(out_root, "run-%s-%d" % (arm, port))
    if os.path.exists(run):
        raise SystemExit("run 目录已存在，拒绝覆盖（失败证据不删除）: %s" % run)
    manifest = {
        "schema": "wig-run-manifest/1", "mode": "arm", "tag": tag, "arm": arm,
        "port": port, "started_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "binary": binary, "binary_sha256": sha256_bin(binary),
        "seed_workspace": seed_ws,
        "seed_manifest_sha256": sha256_file(
            os.path.join(os.path.dirname(seed_ws.rstrip("/")),
                         "seed-manifest.json")),
        "project_id": PROJECT,
        "issue_id": seed_manifest["issue_id"],
        "story_spec_id": seed_manifest["story_spec_id"],
        "story_version": seed_manifest["story_version"],
        "status": None, "plan_session_id": None, "provider_start_ledger": 0,
        "checks": {}, "valid": False, "invalid_reason": None,
    }
    log("[%s] 复制 seed 快照 -> %s" % (time.strftime("%H:%M:%S"), run))
    shutil.copytree(seed_ws, run, symlinks=True,
                    ignore=shutil.ignore_patterns("daemon.log",
                                                  "story-ws-frames.jsonl"))
    log("  路径重指向: %d 个文件" % repoint_paths(run, seed_ws))


    issue_id = manifest["issue_id"]
    story_id = manifest["story_spec_id"]
    proc = start_daemon(binary, run, port)
    try:
        wait_health(port, proc)
        base = "http://127.0.0.1:%d/api/projects/%s/issues" % (port, PROJECT)

        st, target = http("GET", "%s/%s/automation-target?author_provider=%s"
                          "&reviewer_provider=%s" % (base, issue_id, PROVIDER, PROVIDER))
        log("  automation-target: %s" % st)
        if st != 200:
            raise SystemExit("automation-target 失败: %s" % str(target)[:200])
        options = target["resolved_options"]
        options["plan_approval"] = "auto"

        st, prep = http("POST", "%s/%s/work-item-plans:prepare" % (base, issue_id), {
            "title": TITLE,
            "story_spec_ids": [story_id],
            "design_spec_ids": [],
            "author_provider": PROVIDER,
            "reviewer_provider": PROVIDER,
            "run_policy": "auto_if_valid",
        })
        log("  prepare: %s %s" % (st, str(prep)[:200]))
        if st not in (200, 201):
            raise SystemExit("prepare 失败")

        wait_terminated(run, issue_id)
        log("  terminated 已满足")

        st, enr = http("PUT", "%s/%s/automation-enrollment" % (base, issue_id), {
            "expected_revision": None,
            "command": {"type": "enable",
                        "command_id": "cmd-%s-%d-%d" % (arm, port, int(time.time())),
                        "selection_key": "human-choice-1",
                        "source": {"stories": [{"id": story_id,
                                                "version": manifest["story_version"]}],
                                   "designs": []},
                        "options": options,
                        "target": target["enrollment_target"]}})
        log("  enable: %s %s" % (st, str(enr)[:200]))
        if st not in (200, 201):
            raise SystemExit("enable 失败")

        status, plan_sid = wait_terminal(run, issue_id)
        manifest["status"] = status
        manifest["plan_session_id"] = plan_sid
        log("  终态: %s (plan_session=%s)" % (status, plan_sid))
        if plan_sid:
            rec = read_json(os.path.join(
                run, ".aria", "projects", PROJECT, "issues", issue_id,
                "workspace-sessions", "%s.json" % plan_sid)) or {}
            manifest["provider_start_ledger"] = len(rec.get("provider_start_ledger") or [])
    finally:
        stop_daemon(proc)

    # 输入身份复核：story spec 逐字节一致 + 成员 heads 未漂移
    spec_arm = os.path.join(run, ".aria", "projects", PROJECT, "issues", issue_id,
                            "story-specs", "%s.json" % story_id)
    manifest["checks"]["story_input_identical"] = (
        os.path.exists(spec_arm)
        and sha256_file(spec_arm) == seed_manifest["story_input_sha256"])
    heads_after = member_heads(run)
    manifest["checks"]["member_heads_stable"] = (heads_after == seed_manifest["member_heads"])
    manifest["checks"]["member_heads_after"] = heads_after
    if status not in TERMINAL:
        manifest["invalid_reason"] = "未达终态（%s）" % status
    elif not manifest["checks"]["story_input_identical"]:
        manifest["invalid_reason"] = "story 输入与 seed 不一致（第二变量污染）"
    elif not manifest["checks"]["member_heads_stable"]:
        manifest["invalid_reason"] = "成员 heads 相对 seed 漂移（跨臂污染疑点）"
    elif manifest["provider_start_ledger"] <= 0:
        manifest["invalid_reason"] = "plan 会话 provider_start_ledger 为空（非真实 provider run）"
    else:
        manifest["valid"] = True
    manifest["finished_at"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    with open(os.path.join(run, "run-manifest.json"), "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=1)
    with open(log_path, "a", encoding="utf-8") as f:
        f.write("finished %s %s port=%d status=%s valid=%d sha=%s\n" % (
            tag, arm, port, status, 1 if manifest["valid"] else 0,
            manifest["binary_sha256"][:12]))
    return manifest["valid"]


# ───────────────────────── main ─────────────────────────

def _write_failed_manifest(out_root, arm, tag, port, binary, seed_ws,
                           seed_manifest, reason):
    """失败臂也落 manifest（绑定 + invalid 原因），收割器按排除清单呈现。"""
    run = os.path.join(out_root, "run-%s-%d" % (arm, port))
    if not os.path.isdir(run):
        return
    mpath = os.path.join(run, "run-manifest.json")
    if os.path.exists(mpath):
        return
    manifest = {
        "schema": "wig-run-manifest/1", "mode": "arm", "tag": tag, "arm": arm,
        "port": port, "started_at": None,
        "finished_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "binary": binary,
        "binary_sha256": sha256_bin(binary) if os.path.exists(binary) else None,
        "seed_workspace": seed_ws,
        "project_id": PROJECT,
        "issue_id": (seed_manifest or {}).get("issue_id"),
        "story_spec_id": (seed_manifest or {}).get("story_spec_id"),
        "story_version": (seed_manifest or {}).get("story_version"),
        "status": None, "plan_session_id": None, "provider_start_ledger": 0,
        "checks": {}, "valid": False,
        "invalid_reason": "臂执行中断（%s）" % reason,
    }
    with open(mpath, "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=1)


def main():
    global log_path
    args = sys.argv[1:]
    if args and args[0] in ("--seed", "--smoke"):  # --smoke = seed 的同义入口
        if len(args) < 4:
            raise SystemExit("用法: run_arms.py --seed <binary> <src工作区> <out根> [端口=4709]")
        binary, src, out_root = args[1], args[2], args[3]
        port = int(args[4]) if len(args) > 4 else 4709
        os.makedirs(out_root, exist_ok=True)
        log_path = os.path.join(out_root, "sequence.log")
        make_seed(binary, src, out_root, port)
        return
    if len(args) < 5:
        raise SystemExit("用法: run_arms.py <before二进制> <after二进制> <seed工作区> "
                         "<out根> <每侧臂数=2> [起始端口=4701]")
    before_bin, after_bin, seed_ws, out_root = args[0], args[1], args[2], args[3]
    n = int(args[4])
    port0 = int(args[5]) if len(args) > 5 else 4701
    seed_manifest_path = os.path.join(os.path.dirname(seed_ws.rstrip("/")),
                                      "seed-manifest.json")
    seed_manifest = read_json(seed_manifest_path)
    if not seed_manifest or seed_manifest.get("schema") != "wig-seed-manifest/1":
        raise SystemExit("seed-manifest.json 缺失或 schema 不符: %s"
                         % seed_manifest_path)
    if seed_manifest.get("provider_start_ledger", 0) <= 0:
        raise SystemExit("seed 无真实 provider 启动证据，拒绝开臂")
    os.makedirs(out_root, exist_ok=True)
    log_path = os.path.join(out_root, "sequence.log")
    log("seed 输入: issue=%s story=%s v%s members=%d sha=%s" % (
        seed_manifest["issue_id"], seed_manifest["story_spec_id"],
        seed_manifest["story_version"], seed_manifest.get("active_members", 0),
        seed_manifest["story_input_sha256"][:12]))

    # ABBA 交替（每对内 after,before,before,after）：抵消顺序效应；
    # 两侧各 n 臂，唯一变量 = 二进制。
    order = []
    for i in range(n):
        order.append(("after", "after%da" % (i + 1), after_bin))
        order.append(("before", "before%da" % (i + 1), before_bin))
        order.append(("before", "before%db" % (i + 1), before_bin))
        order.append(("after", "after%db" % (i + 1), after_bin))

    any_invalid = False
    port = port0
    for tag, arm, binary in order:
        log("=== 臂 %s (%s) port=%d binary=%s" % (arm, tag, port, binary))
        try:
            ok = run_arm(binary, tag, arm, port, seed_ws, seed_manifest, out_root)
            any_invalid = any_invalid or not ok
        except SystemExit as e:
            log("  臂失败(计入invalid): %s" % e)
            _write_failed_manifest(out_root, arm, tag, port, binary, seed_ws,
                                   seed_manifest, "error: %s" % e)
            with open(log_path, "a", encoding="utf-8") as f:
                f.write("failed %s %s port=%d error=%s\n" % (tag, arm, port, e))
            any_invalid = True
        except Exception as e:
            log("  臂异常(计入invalid): %r" % e)
            _write_failed_manifest(out_root, arm, tag, port, binary, seed_ws,
                                   seed_manifest, "exception: %r" % e)
            with open(log_path, "a", encoding="utf-8") as f:
                f.write("failed %s %s port=%d exception=%r\n" % (tag, arm, port, e))
            any_invalid = True
        port += 1
    log("全部臂结束: %s" % ("存在 invalid 臂" if any_invalid else "全部有效"))
    sys.exit(1 if any_invalid else 0)


if __name__ == "__main__":
    main()
