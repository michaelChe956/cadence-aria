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
「原案例证据现状」），本装置重建的 seed ≠ 原验收输入。issue 正文必须来自显式
输入文件（--issue-input 或 WIG_ISSUE_INPUT_FILE），装置**不再内置任何 description
占位**，并把文件 sha256 + 来源路径写进 seed/arm manifest。

输入身份（谁才是「同一输入」）：
  * story 判等只认 confirmed **版本正文**（version json 的 markdown 字段 UTF-8
    bytes）的 sha256；story spec 记录自带可变时间戳，不得用作判等依据。
  * 成员身份按产品 durable 落盘解析（manifest → members → checkouts），逐成员
    比 alias / canonical_path / checkout_path_hash / git_dir_identity /
    canonical_git_dir / HEAD / tree / branch / **dirty(status)**；HEAD 未动但
    dirty 内容变了 ⇒ 判定不稳定（不是「同一输入」）。
  * seed 完成时冻结整份成员快照，臂前/臂后都按它判等。

权限/信任门纪律：permission_modes 取 auto；permission_request / choice_request
等未知门 ⇒ **明确报阻塞并判该臂失败**，绝不自动批准。trust 缺项单独处理：
装置只读观测（$CODEX_HOME 或 $HOME/.codex/config.toml 的 [projects."<root>"]
trust_level），缺项时默认**只报 controller 需登记的命令片段并失败**；仅当
controller 显式置 WIG_TRUST_AUTHORIZED=1 且 scratch HOME 真实位于 /tmp 时，
才登记该条并**显式重试 start_generation**。装置不打印 config 内容、不复制任何
凭据/secret，临时环境一律由 controller 的 prepare_env 提供。

产物纪律：只写 --out（/tmp）；已存在的 run/seed 目录一律拒绝覆盖（失败证据
不删除）；任一臂超时/异常/未终态 ⇒ 进程退出码非 0。

用法：
  # 一次性固定输入（也是 controller 的真实 smoke 入口）
  #   <state 根> = .aria 所在、会被克隆成 seed-workspace 的目录（controller 的 WIG_STATE_ROOT）
  #   固定案例根 = 3 成员仓所在目录，唯一来源 --case-root 或 WIG_CASE_ROOT（不从 src 推断）
  python3 run_arms.py --seed <aria二进制> <state根> <out根> [端口] \
      --case-root /tmp/wig-measurement-setup/case \
      --issue-input /tmp/wig-measurement-setup/issue_input.md
  # 8 臂 ABBA（4 before + 4 after），唯一变量 = 二进制
  python3 run_arms.py <before二进制> <after二进制> <seed工作区> <out根> 2 [起始端口]
  # issue 输入/案例根也可走环境变量：WIG_ISSUE_INPUT_FILE / WIG_CASE_ROOT
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

# 状态根重指向：只认「显式状态根字段」自身的引用，且只改 JSON 值（不碰正文/哈希字段）
JSON_REWRITE_EXT = {".json", ".jsonl"}
STATEROOT_KEYS = {"state_root", "workspace_root", "aria_root",
                  "product_state_root", "root_dir"}
AUTHORITY_DIRS = ("/members/", "/checkouts/", "/versions/")  # 权威/正文落盘，禁止改写
FIXED_MEMBER_ALIASES = ("busi", "frontend", "gateway")
MEMBER_SNAP_SCHEMA = "wig-member-snapshot/1"
SKIP_TREE_DIRS = {".codegraph"}      # 聚合索引缓存：读会改 sqlite 附属文件，不计入
ISSUE_INPUT_ANCHORS = ("公告删除", "REQ-001", "REQ-007", "AC-001", "AC-006")
TRUST_HINTS = ("trust_level", "trust entry", "trust 条目", "workspace trust",
               "untrusted", "not trusted", "trust 缺失")


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


# ───────────────────────── 环境/输入门禁 ─────────────────────────

def assert_scratch_home():
    """scratch HOME 必须由 controller（prepare_env）提供且真实位于 /tmp。

    装置不复制任何凭据/secret、不写真实 HOME；HOME 缺失或指向宿主根 ⇒ 显式失败，
    绝不 fallback 到宿主 HOME（那会污染真实 codex/agents 配置）。
    """
    home = os.environ.get("HOME") or ""
    if not home or not os.path.isdir(home):
        raise SystemExit(
            "HOME 不存在或未设置（%r）：装置拒绝 fallback 宿主 root。\n"
            "controller 需先 source <setup>/env.sh（scratch HOME 必须在 /tmp 下）"
            % home)
    real = os.path.realpath(home)
    if not (real == "/tmp" or real.startswith("/tmp/")):
        raise SystemExit(
            "scratch HOME 必须位于 /tmp（当前 realpath=%s）：装置不写真实 HOME。"
            "\ncontroller 需用 prepare_env 提供的隔离 HOME" % real)
    return real


def load_issue_input(path):
    """读完整 issue 输入正文（显式文件，非占位串），返回 path/text/sha256/bytes。

    空文件/文件缺失/非 UTF-8 ⇒ 非 0 退出（不降级为内置占位 description）。
    """
    if not path:
        raise SystemExit(
            "缺少 issue 输入：必须显式给 --issue-input <issue_input.md> "
            "或环境变量 WIG_ISSUE_INPUT_FILE（装置不再内置 description 占位）")
    src = os.path.abspath(path)
    if not os.path.isfile(src):
        raise SystemExit("issue 输入文件不存在: %s" % src)
    with open(src, "rb") as f:
        raw = f.read()
    if not raw:
        raise SystemExit("issue 输入文件为空: %s" % src)
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as e:
        raise SystemExit("issue 输入不是 UTF-8 文本（%s）: %s" % (src, e))
    if not text.strip():
        raise SystemExit("issue 输入文件只有空白: %s" % src)
    missing = [a for a in ISSUE_INPUT_ANCHORS if a not in text]
    if missing:
        raise SystemExit(
            "issue 输入与固定案例不符（缺 %s）：本装置只接受「公告删除」重建输入，"
            "拒绝混入其它更大故事（issue-003 / story_spec_0015）: %s"
            % (missing, src))
    return {"path": src, "text": text,
            "sha256": hashlib.sha256(raw).hexdigest(), "bytes": len(raw)}


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


def _git(repo, *args, timeout=120):
    try:
        proc = subprocess.run(["git", "-C", repo, *args], capture_output=True,
                              text=True, timeout=timeout)
    except Exception:
        return ""
    return proc.stdout.strip() if proc.returncode == 0 else ""


def _tree_digest(root):
    """聚合根内「非成员目录」内容摘要（相对路径 + 模式 + 内容 sha256）。"""
    entries = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = sorted(d for d in dirnames
                             if d not in SKIP_TREE_DIRS
                             and d not in FIXED_MEMBER_ALIASES)
        for name in sorted(filenames):
            full = os.path.join(dirpath, name)
            rel = os.path.relpath(full, root)
            try:
                st = os.lstat(full)
                if os.path.islink(full):
                    entries.append("%s|l|%s" % (rel, os.readlink(full)))
                    continue
                h = hashlib.sha256()
                with open(full, "rb") as fh:
                    for chunk in iter(lambda: fh.read(1 << 20), b""):
                        h.update(chunk)
                entries.append("%s|f|%o|%d|%s" % (rel, st.st_mode & 0o777,
                                                  st.st_size, h.hexdigest()))
            except OSError:
                entries.append("%s|err" % rel)
    blob = "\n".join(sorted(entries)).encode("utf-8")
    return hashlib.sha256(blob).hexdigest(), len(entries)


def member_snapshot(root):
    """成员身份快照：manifest → members → checkouts（产品 durable 落盘解析链）。

    与「浅扫 <root>/*/.git 只拿 HEAD」的区别：canonical_path /
    checkout_path_hash / git_dir_identity / source_identity 来自产品权威记录，
    并把 dirty 状态（git status --porcelain）一并纳入——HEAD 未动但工作树被改
    不会被误判成「同一输入」。
    """
    root = os.path.abspath(root)
    lc_root = os.path.join(root, ".aria", "projects")
    manifest, lc_dir = None, None
    if os.path.isdir(lc_root):
        for project in sorted(os.listdir(lc_root)):
            pdir = os.path.join(lc_root, project, "logical-codebases")
            if not os.path.isdir(pdir):
                continue
            for lc in sorted(os.listdir(pdir)):
                cand = read_json(os.path.join(pdir, lc, "manifest.json"))
                if cand and cand.get("member_ids"):
                    manifest, lc_dir = cand, os.path.join(pdir, lc)
                    break
            if manifest:
                break
    if not manifest:
        raise SystemExit("成员快照失败：未找到带成员的产品 manifest（state_root=%s）"
                         % root)

    members = {}
    for member_id in manifest.get("member_ids") or []:
        rec = read_json(os.path.join(lc_dir, "members", "%s.json" % member_id)) or {}
        checkout = None
        for cid in rec.get("checkout_ids") or []:
            crec = read_json(os.path.join(lc_dir, "checkouts", "%s.json" % cid))
            if crec and crec.get("canonical_path"):
                checkout = crec
                break
        if not checkout:
            raise SystemExit("成员 %s 无 checkout canonical_path（authority 无法复核）"
                             % member_id)
        path = checkout["canonical_path"]
        members[member_id] = {
            "alias": rec.get("alias"),
            "path": path,
            "checkout_path_hash": checkout.get("checkout_path_hash"),
            "git_dir_identity": checkout.get("git_dir_identity"),
            "canonical_git_dir": (rec.get("source_identity") or {}).get(
                "canonical_git_dir"),
            "head": _git(path, "rev-parse", "HEAD"),
            "tree": _git(path, "rev-parse", "HEAD^{tree}"),
            "branch": _git(path, "rev-parse", "--abbrev-ref", "HEAD"),
            "status": _git(path, "status", "--porcelain"),
        }
    agg_root = manifest.get("provider_context_root")
    agg_sha, agg_files = ("", 0)
    if agg_root and os.path.isdir(agg_root):
        agg_sha, agg_files = _tree_digest(agg_root)
    return {
        "schema": MEMBER_SNAP_SCHEMA,
        "state_root": root,
        "project_id": manifest.get("project_id"),
        "logical_codebase_id": manifest.get("logical_codebase_id"),
        "aggregate_root": agg_root,
        "aggregate_nonmember_tree_sha256": agg_sha,
        "aggregate_nonmember_file_count": agg_files,
        "members": members,
    }


SNAPSHOT_TOP_FIELDS = ("project_id", "logical_codebase_id", "aggregate_root",
                       "aggregate_nonmember_tree_sha256",
                       "aggregate_nonmember_file_count")
MEMBER_FIELDS = ("alias", "path", "checkout_path_hash", "git_dir_identity",
                 "canonical_git_dir", "head", "tree", "branch", "status")


def snapshot_diff(before, after):
    """逐字段差异清单（只忽略 state_root 自身：每臂 run 目录本就不同）。"""
    diff = []
    for k in SNAPSHOT_TOP_FIELDS:
        if (before or {}).get(k) != (after or {}).get(k):
            diff.append(k)
    bm = (before or {}).get("members") or {}
    am = (after or {}).get("members") or {}
    if set(bm) != set(am):
        diff.append("member_ids")
    for mid in sorted(set(bm) & set(am)):
        for f in MEMBER_FIELDS:
            if bm[mid].get(f) != am[mid].get(f):
                diff.append("%s.%s" % (mid, f))
    return diff


def assert_three_members(snap, where):
    """固定案例 = 精确 3 成员 busi/frontend/gateway，且身份字段齐全。"""
    members = (snap or {}).get("members") or {}
    if len(members) != 3:
        raise SystemExit("%s: 成员数=%d，固定案例必须精确 3 个"
                         % (where, len(members)))
    aliases = sorted((m.get("alias") or "") for m in members.values())
    if aliases != sorted(FIXED_MEMBER_ALIASES):
        raise SystemExit("%s: 成员 alias=%s，与固定 %s 不符"
                         % (where, aliases, list(FIXED_MEMBER_ALIASES)))
    for mid, m in members.items():
        if not m.get("path") or not os.path.isdir(m["path"]):
            raise SystemExit("%s: 成员 %s 的 canonical_path 不存在: %r"
                             % (where, m.get("alias") or mid, m.get("path")))
        for f in ("head", "tree", "checkout_path_hash", "git_dir_identity"):
            if not m.get(f):
                raise SystemExit("%s: 成员 %s 身份字段 %s 缺失"
                                 % (where, m.get("alias") or mid, f))


def assert_fixed_case_paths(snap, case_root, where):
    """成员 canonical_path 必须仍在固定外部案例根之下（臂不得改写成员位置）。"""
    case_root = os.path.abspath(case_root).rstrip("/")
    for mid, m in ((snap or {}).get("members") or {}).items():
        p = os.path.abspath(m.get("path") or "")
        if not (p == case_root or p.startswith(case_root + "/")):
            raise SystemExit("%s: 成员 %s 的 canonical_path=%s 不在固定案例根 %s 之下"
                             % (where, m.get("alias") or mid, p, case_root))


def _repoint_str(value, src_root, run_root):
    if value == src_root:
        return run_root
    if value.startswith(src_root + "/"):
        return run_root + value[len(src_root):]
    return value


def _rewrite_node(node, src_root, run_root, hits):
    if isinstance(node, dict):
        out = {}
        for k, v in node.items():
            if isinstance(v, str) and k in STATEROOT_KEYS:
                nv = _repoint_str(v, src_root, run_root)
                if nv != v:
                    hits.append(k)
                    v = nv
            else:
                v = _rewrite_node(v, src_root, run_root, hits)
            out[k] = v
        return out
    if isinstance(node, list):
        return [_rewrite_node(i, src_root, run_root, hits) for i in node]
    return node


def repoint_state_root_refs(run, src_root):
    """只把「显式状态根字段」自身的引用改指到本副本；返回 (文件数, 字段命中)。

    纪律（P5）：
      * 只处理 .aria 下 JSON/JSONL 的**值**，只认 STATEROOT_KEYS 白名单字段；
      * 固定案例路径 / member canonical_path / source_identity / *_hash 字段
        一律不动（authority 目录整目录跳过，且这些 key 不在白名单）；
      * 没有任何命中就**不写文件**（不为「看起来动过」重写 hashed JSON）；
        固定外部案例保持原样 ⇒ 多臂共享同一物理成员仓是预期语义。
    """
    src_root = os.path.abspath(src_root).rstrip("/")
    run_root = os.path.abspath(run).rstrip("/")
    if src_root == run_root:
        return 0, []
    aria = os.path.join(run_root, ".aria")
    if not os.path.isdir(aria):
        return 0, []
    changed, all_hits = 0, []
    for root, _dirs, files in os.walk(aria):
        posix = root.replace(os.sep, "/") + "/"
        if any(d in posix for d in AUTHORITY_DIRS):
            continue
        for name in sorted(files):
            ext = os.path.splitext(name)[1]
            if ext not in JSON_REWRITE_EXT:
                continue
            path = os.path.join(root, name)
            try:
                with open(path, encoding="utf-8") as f:
                    text = f.read()
            except (UnicodeDecodeError, OSError):
                continue
            if src_root not in text:
                continue
            hits = []
            try:
                if ext == ".jsonl":
                    lines = [ln for ln in text.splitlines() if ln.strip()]
                    payload = "".join(
                        json.dumps(_rewrite_node(json.loads(ln), src_root,
                                                 run_root, hits),
                                   ensure_ascii=False) + "\n" for ln in lines)
                else:
                    payload = json.dumps(
                        _rewrite_node(json.loads(text), src_root, run_root, hits),
                        ensure_ascii=False, indent=1) + "\n"
            except json.JSONDecodeError:
                continue          # 非 JSON 正文：不猜、不改
            if not hits:
                continue
            with open(path, "w", encoding="utf-8") as f:
                f.write(payload)
            changed += 1
            all_hits.extend(sorted(set(hits)))
    return changed, sorted(set(all_hits))


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


def create_issue(port, lc_id, logical_ids, physical_id, description):
    """建 issue：description 必须是**显式输入文件的完整正文**（不是占位串）。"""
    if not isinstance(description, str) or not description.strip():
        raise SystemExit("建 issue 的 description 为空：必须来自 issue 输入文件")
    base = "http://127.0.0.1:%d/api/projects/%s/issues" % (port, PROJECT)
    st, body = http("POST", base, {
        "title": TITLE,
        "description": description,
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


# ───────────────────────── trust 门（只读观测 + 授权登记） ─────────────────────────

def codex_config_path():
    home = os.environ.get("CODEX_HOME") or os.path.join(
        os.environ.get("HOME") or "", ".codex")
    return os.path.join(home, "config.toml")


def trust_entry_header(root):
    """codex 用户级 workspace trust 表头（与产品 CodexTrustAdapter 同形）。"""
    return '[projects."%s"]' % os.path.abspath(root).rstrip("/")


def trust_entry_state(root):
    """观测 root 的 codex trust 条目状态；只读，绝不打印 config 内容。"""
    path = codex_config_path()
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            text = f.read()
    except OSError:
        return "config_missing"
    header = trust_entry_header(root)
    idx = text.find(header)
    if idx < 0:
        return "entry_missing"
    for ln in text[idx + len(header):].split("\n")[1:]:
        s = ln.strip()
        if s.startswith("["):
            break
        if s.startswith("trust_level"):
            val = s.split("=", 1)[1].strip().strip('"') if "=" in s else ""
            return "trusted" if val == "trusted" else "not_trusted"
    return "entry_without_trust_level"


def register_trust_entry(root):
    """controller 已授权时补登记 trust 条目；只写 /tmp scratch HOME 内 config.toml。"""
    home = assert_scratch_home()
    path = codex_config_path()
    real = os.path.realpath(path)
    if not (real == os.path.join(home, ".codex", "config.toml")
            or real.startswith(home.rstrip("/") + "/")):
        raise SystemExit("codex config 不在 scratch HOME 内（%s）：拒绝写入" % real)
    state = trust_entry_state(root)
    if state == "not_trusted":
        raise SystemExit(
            "root=%s 已有非 trusted 条目：不覆盖用户条目（产品同样判 conflict）"
            "；请 controller 裁决后重跑" % root)
    if state == "trusted":
        return "already_trusted"
    if state == "entry_without_trust_level":
        raise SystemExit("root=%s 的 trust 表存在但缺 trust_level：不猜不改" % root)
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError:
        text = ""
    snippet = "\n" if text and not text.endswith("\n") else ""
    snippet += "%s\ntrust_level = \"trusted\"\n" % trust_entry_header(root)
    os.makedirs(os.path.dirname(path), mode=0o700, exist_ok=True)
    with open(path, "a", encoding="utf-8") as f:
        f.write(snippet)
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass
    return "registered"


def trust_roots(run):
    """trust 观测面：聚合根 + 每个成员的 canonical_path（去重、保序）。

    codex 的 trust key 是「provider 在该目录下运行」的 canonical root；装置不猜
    唯一根，而是把观测到的候选全部列出/登记。
    """
    snap = member_snapshot(run)
    cand = []
    agg = snap.get("aggregate_root")
    if agg:
        cand.append(agg)
    for m in (snap.get("members") or {}).values():
        if m.get("path"):
            cand.append(m["path"])
    seen, out = set(), []
    for r in cand:
        rp = os.path.abspath(r).rstrip("/")
        if rp not in seen:
            seen.add(rp)
            out.append(rp)
    return out


def resolve_trust_gate(roots):
    """返回 (resolution, missing)。缺项且未授权 ⇒ SystemExit（只报命令片段）。

    已授权（WIG_TRUST_AUTHORIZED=1）⇒ 补登记缺失条并返回 "registered"；
    登记本身只在 scratch HOME 位于 /tmp 时进行（assert_scratch_home 兜底）。
    """
    states = {r: trust_entry_state(r) for r in roots}
    missing = [r for r in roots if states[r] != "trusted"]
    if not missing:
        return "trusted", []
    if os.environ.get("WIG_TRUST_AUTHORIZED") != "1":
        snippets = "\n".join("%s\ntrust_level = \"trusted\""
                             % trust_entry_header(r) for r in missing)
        raise SystemExit(
            "codex trust 条目缺失（config=%s，观测：%s）：装置不写 HOME、不自动批准。\n"
            "controller 二选一后重跑：\n"
            "  a) 在上述 config 追加（或并入既有表）：\n%s\n"
            "  b) 置 WIG_TRUST_AUTHORIZED=1（scratch HOME 必须在 /tmp 下）重跑，"
            "装置会登记缺失条并显式重试 start_generation。"
            % (codex_config_path(),
               ", ".join("%s=%s" % (os.path.basename(r), states[r]) for r in missing),
               snippets))
    for r in missing:
        register_trust_entry(r)
    return "registered", missing


def story_version_digest(run, issue_id, story_id, version):
    """confirmed 版本正文的逐字节 digest（判等只认 markdown，不认 spec 记录）。

    story spec 记录带可变时间戳/指针，同一正文两次落盘也可能不同字节；
    version json 的 markdown 字段才是「输入」本身。
    """
    vdir = os.path.join(run, ".aria", "projects", PROJECT, "issues", issue_id,
                        "versions", story_id)
    vpath = os.path.join(vdir, "version_%04d.json" % int(version))
    if not os.path.isfile(vpath):
        raise SystemExit("确认版版本文件缺失: %s" % vpath)
    rec = read_json(vpath) or {}
    markdown = rec.get("markdown")
    if not isinstance(markdown, str) or not markdown.strip():
        raise SystemExit("确认版版本文件无 markdown 正文: %s" % vpath)
    body = markdown.encode("utf-8")
    return {
        "story_version_ref": "%s@version_%04d" % (story_id, int(version)),
        "version_file": vpath,
        "version_file_sha256": sha256_file(vpath),
        "markdown_sha256": hashlib.sha256(body).hexdigest(),
        "markdown_bytes": len(body),
    }


def observed_provider_fields(session_rec, sent_cfg):
    """provider/model/effort 的观测来源；没观测到就写 not_observed（不猜默认档）。"""
    out = {}
    for key in ("model", "effort", "reasoning_effort"):
        val, src = None, "not_observed"
        for cfg, tag in ((sent_cfg or {}, "start_generation_payload"),
                         ((session_rec or {}).get("provider_config") or {},
                          "session_record")):
            if isinstance(cfg, dict) and cfg.get(key):
                val, src = cfg[key], tag
                break
        out[key] = {"value": val, "source": src}
    out["provider"] = {"value": PROVIDER, "source": "start_generation_payload"}
    out["permission_modes"] = {"value": "auto", "source": "start_generation_payload"}
    return out


def drive_story_to_confirmed(run, port, issue_id, session_id, story_id,
                             frames_path, budget_s=ARM_TIMEOUT_S):
    """WS hello → start_generation → 等确认门 → HTTP confirm → durable 验证。

    返回证据 dict；任何未知权限/信任/选择门 ⇒ SystemExit（明确阻塞，不自动批准）。
    trust 缺项按 resolve_trust_gate 处理：未授权只报命令片段；已授权登记后
    **显式重试一次** start_generation。
    """
    origin = "http://127.0.0.1:%d" % port
    cli = WsClient("127.0.0.1", port,
                   "/api/workspace-sessions/%s/ws" % session_id, origin)
    stage_seq, blocked, gate_stage, started = [], None, None, False
    trust_retried = False
    sent_cfg = {"author": PROVIDER, "reviewer": PROVIDER, "review_rounds": 1,
                "permission_modes": {"author": "auto", "reviewer": "auto"}}
    frames_fh = open(frames_path, "a", encoding="utf-8")

    def record(entry):
        frames_fh.write(json.dumps(entry, ensure_ascii=False) + "\n")
        frames_fh.flush()

    def start_payload():
        return {"type": "start_generation", "provider_config": sent_cfg,
                "reviewer_enabled": False}

    def handle_trust(where):
        nonlocal trust_retried, started
        resolution, missing = resolve_trust_gate(trust_roots(run))
        record({"event": "trust_gate", "where": where,
                "missing_count": len(missing), "resolution": resolution})
        if resolution != "registered" or trust_retried:
            return False
        trust_retried = True
        log("  trust 门（%s）：按 controller 授权登记 %d 条，显式重试 start_generation"
            % (where, len(missing)))
        cli.send_text(json.dumps(start_payload()))
        record({"event": "start_generation_retry", "reason": "trust_registered"})
        started = True
        return True

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
                    payload = start_payload()
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
                text_blob = ("%s %s" % (msg.get("code") or "",
                                        msg.get("message") or "")).lower()
                record({"event": "server_reject", "code": msg.get("code"),
                        "message": str(msg.get("message"))[:300]})
                if any(h in text_blob for h in TRUST_HINTS) and handle_trust(t):
                    continue          # 已按授权登记并显式重试 start
                blocked = "服务端 %s: code=%s message=%s" % (
                    t, msg.get("code"), str(msg.get("message"))[:300])
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
            elif t and "trust" in str(t).lower():
                record({"event": "trust_frame", "type": t})
                if handle_trust("frame:%s" % t):
                    continue
                blocked = ("%s 到达且无法自行解决：装置不自动批准（需 controller 登记 "
                           "codex workspace trust 或授权后重跑）" % t)
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
    version = spec.get("current_version")
    digest = story_version_digest(run, issue_id, story_id, version)
    log("  story 正文 digest: %s（%d bytes，%s）"
        % (digest["markdown_sha256"][:12], digest["markdown_bytes"],
           digest["story_version_ref"]))
    return {"session_id": session_id, "story_spec_id": story_id,
            "story_version": version,
            "gate_stage": gate_stage, "stage_seq": stage_seq,
            "provider_start_ledger": len(ledger),
            "session_status": rec.get("status"),
            # spec 记录只作参考（含可变时间戳），判等一律用下面的正文 digest
            "story_record_sha256": sha256_file(spec_path),
            "story_version_ref": digest["story_version_ref"],
            "version_file_sha256": digest["version_file_sha256"],
            "markdown_sha256": digest["markdown_sha256"],
            "markdown_bytes": digest["markdown_bytes"],
            "observed": observed_provider_fields(rec, sent_cfg)}





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

def resolve_case_root(cli_value):
    """固定案例根（3 成员仓所在目录）的唯一权威来源：--case-root 或 WIG_CASE_ROOT。

    绝不从 `--seed` 的 src 推断：src 是 **state root**（`.aria` 所在、会被克隆成
    seed-workspace 的目录），两者可以是完全不同的路径（controller 的
    `WIG_CASE_ROOT=<case>` vs `WIG_STATE_ROOT=<state-seed>`）。缺值 ⇒ 显式失败，
    不做任何猜测/回退。
    """
    value = (cli_value or os.environ.get("WIG_CASE_ROOT") or "").strip()
    if not value:
        raise SystemExit(
            "缺少固定案例根：显式给 --case-root <3成员案例根> 或环境变量 "
            "WIG_CASE_ROOT（controller 的 env.sh 已导出）；不从 --seed 的 src 推断")
    root = os.path.abspath(value).rstrip("/")
    if not os.path.isdir(root):
        raise SystemExit("固定案例根不存在: %s" % root)
    return root


def make_seed(binary, src, out_root, port, issue_input, case_root):
    home = assert_scratch_home()
    src = os.path.abspath(src).rstrip("/")
    if not os.path.isdir(src):
        raise SystemExit("案例工作区不存在: %s" % src)
    case_root = os.path.abspath(case_root).rstrip("/")
    if src == case_root:
        raise SystemExit("--case-root 与 --seed 的 src 相同（%s）：前者是成员案例根、"
                         "后者是 state root，混用会误判成员身份" % src)
    seed_ws = os.path.join(out_root, "seed-workspace")
    if os.path.exists(seed_ws):
        raise SystemExit("seed 目录已存在，拒绝覆盖（证据保护）: %s" % seed_ws)
    os.makedirs(out_root, exist_ok=True)
    log("=== seed: 复制案例工作区 %s -> %s" % (src, seed_ws))
    shutil.copytree(src, seed_ws, symlinks=True)

    # 先建 manifest 骨架：任何早失败（成员断言 / 重指向漂移）也要留 invalid 证据
    manifest = {
        "schema": "wig-seed-manifest/1",
        "mode": "seed",
        "created_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "binary": binary,
        "binary_sha256": sha256_bin(binary),
        "src": src,
        "state_root_src": src,
        "fixed_case_root": case_root,
        "case_root_env": os.environ.get("WIG_CASE_ROOT"),
        "project_id": PROJECT,
        "scratch_home": home,
        "issue_input_path": issue_input["path"],
        "issue_input_sha256": issue_input["sha256"],
        "issue_input_bytes": issue_input["bytes"],
        "member_snapshot": None,
        "member_snapshot_sha256": None,
        "valid": False,
        "invalid_reason": "seed 未完成",
    }
    mpath = os.path.join(out_root, "seed-manifest.json")

    def flush():
        with open(mpath, "w", encoding="utf-8") as f:
            json.dump(manifest, f, ensure_ascii=False, indent=1)

    try:
        # 成员 authority 冻结：复制前后必须逐字段一致（重指向不得碰
        # canonical_path / checkout_path_hash / git_dir_identity / dirty）
        snap_before = member_snapshot(seed_ws)
        assert_three_members(snap_before, "seed")
        assert_fixed_case_paths(snap_before, case_root, "seed")
        agg_root = os.path.abspath(snap_before.get("aggregate_root") or "").rstrip("/")
        if agg_root and agg_root != case_root:
            raise SystemExit(
                "固定案例根与产品记录不一致：case_root=%s，snapshot.aggregate_root=%s。"
                "唯一权威是 --case-root/WIG_CASE_ROOT（不静默纠正、不把 state root 当案例根）"
                % (case_root, agg_root))
        changed, hits = repoint_state_root_refs(seed_ws, src)
        if changed:
            log("  状态根引用重指向: %d 个文件（字段 %s）" % (changed, hits))
        else:
            log("  未发现需改写的状态根引用：保留原固定外部案例（不重写任何 JSON）")
        snap_after = member_snapshot(seed_ws)
        diff = snapshot_diff(snap_before, snap_after)
        if diff:
            raise SystemExit("重指向后成员 authority 变化（%s）：拒绝继续（装置失效）"
                             % diff)
        log("  成员身份冻结: %s" % json.dumps(
            {m.get("alias"): {"path": m.get("path"), "head": m.get("head"),
                              "tree": m.get("tree"),
                              "dirty_entries": len((m.get("status") or "").splitlines())}
             for m in snap_after["members"].values()}, ensure_ascii=False))
        manifest["member_snapshot"] = snap_after
        manifest["member_snapshot_sha256"] = hashlib.sha256(
            json.dumps(snap_after, ensure_ascii=False, sort_keys=True)
            .encode("utf-8")).hexdigest()
    except BaseException as e:
        manifest["invalid_reason"] = "seed 失败（%s）" % e
        flush()
        raise

    try:
        proc = start_daemon(binary, seed_ws, port)
        try:
            wait_health(port, proc)
            lc_id, logical_ids, physical_id = discover_lc(seed_ws)
            log("  逻辑代码库: %s 活跃成员=%d" % (lc_id, len(logical_ids)))
            issue_id = create_issue(port, lc_id, logical_ids, physical_id,
                                    issue_input["text"])
            manifest["issue_id"] = issue_id
            log("  issue_id=%s（description 来源=%s sha256=%s）"
                % (issue_id, issue_input["path"], issue_input["sha256"][:12]))
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
        manifest.update({
            "story_spec_id": ev["story_spec_id"],
            "story_version": ev["story_version"],
            "story_version_ref": ev["story_version_ref"],
            "story_session_id": ev["session_id"],
            "story_markdown_sha256": ev["markdown_sha256"],
            "story_markdown_bytes": ev["markdown_bytes"],
            "story_version_file_sha256": ev["version_file_sha256"],
            "story_record_sha256": ev["story_record_sha256"],
            "gate_stage": ev["gate_stage"],
            "provider_start_ledger": ev["provider_start_ledger"],
            "observed": ev["observed"],
            "active_members": len(logical_ids),
            "member_authority_after": member_snapshot(seed_ws),
            "valid": True,
            "invalid_reason": None,
        })
        after = manifest["member_authority_after"]
        d2 = snapshot_diff(manifest["member_snapshot"], after)
        if d2 or manifest["member_snapshot_sha256"] != hashlib.sha256(
                json.dumps(after, ensure_ascii=False, sort_keys=True)
                .encode("utf-8")).hexdigest():
            raise SystemExit("seed 结束时成员 authority 已漂移（%s）" % d2)
    except BaseException as e:
        manifest["invalid_reason"] = "seed 失败（%s）" % e
        flush()
        raise
    flush()
    log("seed 完成：%s" % mpath)
    print(json.dumps({k: manifest[k] for k in
                      ("issue_id", "story_spec_id", "story_version",
                       "story_version_ref", "story_session_id",
                       "provider_start_ledger", "gate_stage", "binary_sha256",
                       "issue_input_sha256", "story_markdown_sha256",
                       "active_members", "valid")},
                     ensure_ascii=False, indent=1))
    return manifest


# ───────────────────────── 单臂 ─────────────────────────

def run_arm(binary, tag, arm, port, seed_ws, seed_manifest, out_root):
    home = assert_scratch_home()
    run = os.path.join(out_root, "run-%s-%d" % (arm, port))
    if os.path.exists(run):
        raise SystemExit("run 目录已存在，拒绝覆盖（失败证据不删除）: %s" % run)
    seed_snapshot = seed_manifest.get("member_snapshot")
    if not seed_snapshot:
        raise SystemExit("seed-manifest 缺冻结成员快照：拒绝开臂（输入身份无法复核）")
    fixed_case_root = (seed_manifest.get("fixed_case_root")
                       or seed_snapshot.get("aggregate_root"))
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
        "story_version_ref": seed_manifest.get("story_version_ref"),
        "issue_input_path": seed_manifest.get("issue_input_path"),
        "issue_input_sha256": seed_manifest.get("issue_input_sha256"),
        "story_markdown_sha256": seed_manifest.get("story_markdown_sha256"),
        "fixed_case_root": fixed_case_root,
        "scratch_home": home,
        "observed": {"seed": seed_manifest.get("observed"), "arm_session": None},
        "status": None, "plan_session_id": None, "provider_start_ledger": 0,
        "checks": {}, "valid": False, "invalid_reason": None,
    }
    log("[%s] 复制 seed 快照 -> %s" % (time.strftime("%H:%M:%S"), run))
    shutil.copytree(seed_ws, run, symlinks=True,
                    ignore=shutil.ignore_patterns("daemon.log",
                                                  "story-ws-frames.jsonl"))
    # 克隆即校验：成员 authority 必须与 seed 冻结快照逐字段一致
    snap0 = member_snapshot(run)
    assert_three_members(snap0, "arm:%s" % arm)
    assert_fixed_case_paths(snap0, fixed_case_root, "arm:%s" % arm)
    manifest["checks"]["member_snapshot_before_diff"] = snapshot_diff(seed_snapshot,
                                                                     snap0)
    changed, hits = repoint_state_root_refs(run, seed_ws)
    if changed:
        log("  状态根引用重指向: %d 个文件（字段 %s）" % (changed, hits))
    else:
        log("  未发现需改写的状态根引用：保留原固定外部案例（不重写任何 JSON）")
    snap1 = member_snapshot(run)
    manifest["checks"]["member_snapshot_repoint_diff"] = snapshot_diff(snap0, snap1)
    drift = (manifest["checks"]["member_snapshot_before_diff"]
             or (manifest["checks"]["member_snapshot_repoint_diff"] if changed else []))
    if drift:
        manifest["invalid_reason"] = ("成员 authority 在克隆/重指向后漂移（%s）："
                                      "不把「HEAD 相同」当稳定" % drift[:6])
        return _finish_arm(manifest, run, tag, arm, port)

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
            manifest["observed"]["arm_session"] = observed_provider_fields(rec, None)
    finally:
        stop_daemon(proc)

    # 输入身份复核：confirmed 版本正文 digest + 成员快照（含 dirty）逐字段一致
    try:
        vdig = story_version_digest(run, issue_id, story_id,
                                    manifest["story_version"])
        manifest["checks"]["story_markdown_sha256"] = vdig["markdown_sha256"]
        manifest["checks"]["story_markdown_identical"] = (
            vdig["markdown_sha256"] == manifest["story_markdown_sha256"])
    except SystemExit as e:
        manifest["checks"]["story_markdown_identical"] = False
        manifest["checks"]["story_version_error"] = str(e)
    snap_after = member_snapshot(run)
    snapshot_after_diff = snapshot_diff(seed_snapshot, snap_after)
    manifest["checks"]["member_snapshot_diff"] = snapshot_after_diff
    manifest["checks"]["member_snapshot_stable"] = not snapshot_after_diff
    manifest["checks"]["member_snapshot_after"] = snap_after
    if status not in TERMINAL:
        manifest["invalid_reason"] = "未达终态（%s）" % status
    elif not manifest["checks"].get("story_markdown_identical"):
        manifest["invalid_reason"] = ("confirmed 版本正文 digest 与 seed 不一致"
                                      "（第二变量污染）")
    elif not manifest["checks"]["member_snapshot_stable"]:
        manifest["invalid_reason"] = ("成员快照相对 seed 漂移（%s：HEAD 相同也可能是 "
                                      "dirty 内容被改）" % snapshot_after_diff[:6])
    elif manifest["provider_start_ledger"] <= 0:
        manifest["invalid_reason"] = "plan 会话 provider_start_ledger 为空（非真实 provider run）"
    else:
        manifest["valid"] = True
    return _finish_arm(manifest, run, tag, arm, port)


def _finish_arm(manifest, run, tag, arm, port):
    """落 run-manifest.json + sequence.log（失败证据不删、invalid 显式）。"""
    manifest["finished_at"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    with open(os.path.join(run, "run-manifest.json"), "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=1)
    if log_path:
        with open(log_path, "a", encoding="utf-8") as f:
            f.write("finished %s %s port=%d status=%s valid=%d sha=%s\n" % (
                tag, arm, port, manifest.get("status"),
                1 if manifest.get("valid") else 0,
                str(manifest.get("binary_sha256") or "")[:12]))
    return bool(manifest.get("valid"))



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
        "seed_manifest_sha256": sha256_file(
            os.path.join(os.path.dirname(seed_ws.rstrip("/")),
                         "seed-manifest.json"))
        if os.path.isfile(os.path.join(os.path.dirname(seed_ws.rstrip("/")),
                                       "seed-manifest.json")) else None,
        "scratch_home": os.environ.get("HOME"),
        "project_id": PROJECT,
        "issue_id": (seed_manifest or {}).get("issue_id"),
        "story_spec_id": (seed_manifest or {}).get("story_spec_id"),
        "story_version": (seed_manifest or {}).get("story_version"),
        "story_version_ref": (seed_manifest or {}).get("story_version_ref"),
        "issue_input_path": (seed_manifest or {}).get("issue_input_path"),
        "issue_input_sha256": (seed_manifest or {}).get("issue_input_sha256"),
        "story_markdown_sha256": (seed_manifest or {}).get("story_markdown_sha256"),
        "status": None, "plan_session_id": None, "provider_start_ledger": 0,
        "checks": {}, "valid": False,
        "invalid_reason": "臂执行中断（%s）" % reason,
    }
    with open(mpath, "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=1)


def _pop_option(args, flag, env_key):
    """剥离 `flag <value>`（可出现在任意位置）；无则回退 env_key。返回 (value, argv)。"""
    if flag not in args:
        return os.environ.get(env_key) or "", args
    i = args.index(flag)
    if i + 1 >= len(args):
        raise SystemExit("%s 需要文件路径/目录参数" % flag)
    return args[i + 1], args[:i] + args[i + 2:]


def _pop_issue_input(args):
    """剥离 --issue-input <path>；无则回退 WIG_ISSUE_INPUT_FILE。"""
    return _pop_option(args, "--issue-input", "WIG_ISSUE_INPUT_FILE")


def main():
    global log_path
    issue_input_path, args = _pop_issue_input(sys.argv[1:])
    case_root_cli, args = _pop_option(args, "--case-root", "WIG_CASE_ROOT")
    if args and args[0] in ("--seed", "--smoke"):  # --smoke = seed 的同义入口
        if len(args) < 4:
            raise SystemExit("用法: run_arms.py --seed <binary> <state根> <out根> "
                             "[端口=4709] --case-root <案例根> --issue-input <file>")
        binary, src, out_root = args[1], args[2], args[3]
        port = int(args[4]) if len(args) > 4 else 4709
        issue_input = load_issue_input(issue_input_path)   # 空/缺/错案例 ⇒ 非 0
        case_root = resolve_case_root(case_root_cli)       # 唯一权威来源，不猜 src
        assert_scratch_home()
        os.makedirs(out_root, exist_ok=True)
        log_path = os.path.join(out_root, "sequence.log")
        log("issue 输入: %s sha256=%s bytes=%d"
            % (issue_input["path"], issue_input["sha256"], issue_input["bytes"]))
        log("state root=%s（会被克隆）  fixed case root=%s（成员仓所在）"
            % (os.path.abspath(src), case_root))
        make_seed(binary, src, out_root, port, issue_input, case_root)
        return
    if len(args) < 5:
        raise SystemExit("用法: run_arms.py <before二进制> <after二进制> <seed工作区> "
                         "<out根> <每侧臂数=2> [起始端口=4701]")
    before_bin, after_bin, seed_ws, out_root = args[0], args[1], args[2], args[3]
    n = int(args[4])
    port0 = int(args[5]) if len(args) > 5 else 4701
    assert_scratch_home()
    seed_manifest_path = os.path.join(os.path.dirname(seed_ws.rstrip("/")),
                                      "seed-manifest.json")
    seed_manifest = read_json(seed_manifest_path)
    if not seed_manifest or seed_manifest.get("schema") != "wig-seed-manifest/1":
        raise SystemExit("seed-manifest.json 缺失或 schema 不符: %s"
                         % seed_manifest_path)
    if not seed_manifest.get("valid"):
        raise SystemExit("seed-manifest 未标记 valid（%s），拒绝开臂"
                         % seed_manifest.get("invalid_reason"))
    if seed_manifest.get("provider_start_ledger", 0) <= 0:
        raise SystemExit("seed 无真实 provider 启动证据，拒绝开臂")
    if not seed_manifest.get("member_snapshot"):
        raise SystemExit("seed-manifest 缺冻结成员快照，拒绝开臂")
    if not seed_manifest.get("story_markdown_sha256"):
        raise SystemExit("seed-manifest 缺 confirmed 正文 digest，拒绝开臂")
    seed_case_root = seed_manifest.get("fixed_case_root")
    if not seed_case_root:
        raise SystemExit("seed-manifest 缺 fixed_case_root，拒绝开臂（成员 authority 无锚）")
    if case_root_cli:
        # 给了案例根 ⇒ 只允许与 seed 记录一致（不静默纠正、不重指向成员）
        given_case = os.path.abspath(case_root_cli).rstrip("/")
        if given_case != os.path.abspath(seed_case_root).rstrip("/"):
            raise SystemExit("给定 case root（%s）与 seed 记录的 fixed_case_root（%s）"
                             "不一致：拒绝开臂" % (given_case, seed_case_root))
    if issue_input_path:
        given = load_issue_input(issue_input_path)
        if given["sha256"] != seed_manifest.get("issue_input_sha256"):
            raise SystemExit("给定 issue 输入与 seed 记录的 digest 不一致：%s vs %s"
                             % (given["sha256"],
                                seed_manifest.get("issue_input_sha256")))
    os.makedirs(out_root, exist_ok=True)
    log_path = os.path.join(out_root, "sequence.log")
    log("seed 输入: issue=%s story=%s %s members=%d" % (
        seed_manifest["issue_id"], seed_manifest["story_spec_id"],
        seed_manifest.get("story_version_ref"), seed_manifest.get("active_members", 0)))
    log("输入 digest: issue_input=%s story_markdown=%s"
        % (str(seed_manifest.get("issue_input_sha256"))[:12],
           str(seed_manifest.get("story_markdown_sha256"))[:12]))

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
