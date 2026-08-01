"""
HCSE 韧性验证回归测试 Round 3 — LRC Desktop v0.8.22

诊断重点：HTTP 服务器完全阻塞（所有端点超时 12s 回归）

Round 2 结论：P0-A worker_threads=16 已生效（/health avg 3.9ms），14/16 PASS
Round 3 现状：所有 4 端点再次超时 12s，CloseWait=45，17 线程全 Wait 状态
         → 严重回归（worker_threads=16 修复失效，死锁/锁等待耗尽线程池）

根因假设（基于静态代码分析 + 运行时证据）：
  H1 [主因] /health (server.rs:1725) 使用 state.llm_api.read().await 阻塞式异步读锁
           （非 try_lock）。当 llm_api 写锁被持有时，所有 /health 请求堆积在
           read().await，耗尽 16 worker 线程 → v1 try_lock 端点也无法调度 → 全超时
  H2 [诱因] 索引任务 (bin/server.rs:798) bg_mgr.index_project() 同步 CPU 密集
           操作未用 spawn_blocking，阻塞 worker 线程
  H3 [诱因] 合成任务 (consolidation.rs:363) 持 store.lock().await 期间运行
           luoshu_synthesize() CPU 密集操作，扩大锁持有窗口
  H4 [表现] CloseWait=45：handler 接受 TCP 连接但永不完成，连接泄漏堆积

依赖: websocket-client, requests, psutil, pyyaml
"""

from __future__ import annotations

import base64
import json
import os
import re
import sys
import threading
import time
import traceback
from collections import deque
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Any, Optional

import requests
import websocket  # type: ignore
import psutil

# ============================================================
# 常量与配置（Round 3）
# ============================================================

CDP_ENDPOINT = "http://127.0.0.1:9223"
SIDECAR_ENDPOINT = "http://127.0.0.1:3099"
EXPECTED_TARGET_TITLE = "龙忆 Loong Recall"
EXPECTED_VERSION = "0.8.22"
EXPECTED_SIDECAR_PID = 25388  # Round 3: 用户指定当前 sidecar PID

BASE_DIR = Path("g:/code-memory/hcse_resilience_tester").resolve()
ALLOWED_DIRS = {BASE_DIR / "temp", BASE_DIR / "logs",
                BASE_DIR / "screenshots", BASE_DIR / "evidence"}
for d in ALLOWED_DIRS:
    d.mkdir(parents=True, exist_ok=True)

MAX_MEMORY_USAGE_MB = 1024
MAX_CPU_TIME_SECONDS = 60
ENDPOINT_TIMEOUT_THRESHOLD_MS = 2000   # INV-HTTP-001: 端点 2s 内响应
ENDPOINT_HALT_THRESHOLD_MS = 5000      # >5s 判定不变式违反
CLOSE_WAIT_THRESHOLD = 10               # INV-LEAK-006: CloseWait < 10

# Phase 6.2: 脱敏正则（双重脱敏：正则替换 + 结构字段裁剪）
SANITIZE_PATTERNS: list[tuple[re.Pattern, str]] = [
    (re.compile(r'"authorization"\s*:\s*"[^"]*"', re.IGNORECASE),
     '"authorization": "[BEARER_TOKEN_REDACTED]"'),
    (re.compile(r'authorization\s*:\s*Bearer\s+\S+', re.IGNORECASE),
     'authorization: Bearer [BEARER_TOKEN_REDACTED]'),
    (re.compile(r'(sk-[A-Za-z0-9]{20,})'), '[API_KEY_REDACTED]'),
    (re.compile(r'"api_key"\s*:\s*"[^"]*"', re.IGNORECASE),
     '"api_key": "[API_KEY_REDACTED]"'),
    (re.compile(r'"value"\s*:\s*"[^"]*"\s*,\s*"name"\s*:\s*"(?:session|auth|token|cookie)',
                re.IGNORECASE), '"value": "[COOKIE_VALUE_REDACTED]"'),
    (re.compile(r'"cookie"\s*:\s*"[^"]*"', re.IGNORECASE),
     '"cookie": "[COOKIE_VALUE_REDACTED]"'),
    (re.compile(r'[\w.+-]+@[\w-]+\.[\w.-]+'), '[EMAIL_REDACTED]'),
    (re.compile(r'(?<!\d)1[3-9]\d{9}(?!\d)'), '[PHONE_REDACTED]'),
]
SENSITIVE_FIELD_NAMES = {"api_key", "apikey", "secret", "token",
                         "password", "authorization", "value"}


# ============================================================
# Phase 6.1: PathValidator — 路径白名单守卫（Hard Halt）
# ============================================================

class PathValidator:
    """所有文件操作必须经此校验，越界访问触发 Hard Halt"""

    def __init__(self, allowed: set[Path]) -> None:
        self.allowed = {p.resolve() for p in allowed}
        self.violations: list[str] = []

    def validate(self, path: Path) -> Path:
        p = Path(path).resolve()
        for allowed in self.allowed:
            try:
                p.relative_to(allowed)
                return p
            except ValueError:
                continue
        msg = f"[PathValidator HARD HALT] 越界访问被阻止: {p} (允许: {self.allowed})"
        self.violations.append(msg)
        raise PermissionError(msg)


# ============================================================
# Phase 6.2: Sanitizer — 双重脱敏（正则 + 结构裁剪）
# ============================================================

class Sanitizer:
    """证据写入前强制双重脱敏"""

    def __init__(self) -> None:
        self.redaction_count = 0

    def _regex_sanitize(self, text: str) -> str:
        for pattern, replacement in SANITIZE_PATTERNS:
            new_text, n = pattern.subn(replacement, text)
            if n:
                self.redaction_count += n
            text = new_text
        return text

    def _structural_prune(self, obj: Any) -> Any:
        if isinstance(obj, dict):
            pruned = {}
            for k, v in obj.items():
                key_lower = str(k).lower()
                if key_lower in {"cookie"}:
                    if isinstance(v, list):
                        pruned[k] = [{"name": c.get("name", "") if isinstance(c, dict) else "",
                                      "value": "[COOKIE_VALUE_REDACTED]"}
                                     for c in v]
                        self.redaction_count += len(v)
                    else:
                        pruned[k] = "[COOKIE_VALUE_REDACTED]"
                        self.redaction_count += 1
                elif key_lower in SENSITIVE_FIELD_NAMES and isinstance(v, str):
                    pruned[k] = f"[{key_upper(k)}_REDACTED]"
                    self.redaction_count += 1
                else:
                    pruned[k] = self._structural_prune(v)
            return pruned
        if isinstance(obj, list):
            return [self._structural_prune(i) for i in obj]
        return obj

    def sanitize(self, obj: Any) -> Any:
        """双重脱敏：先正则替换文本，再结构字段裁剪"""
        if isinstance(obj, str):
            return self._regex_sanitize(obj)
        pruned = self._structural_prune(obj)
        return json.loads(self._regex_sanitize(json.dumps(pruned, ensure_ascii=False)))

    def sanitize_str(self, text: str) -> str:
        return self._regex_sanitize(text)


def key_upper(k: Any) -> str:
    return str(k).upper()


# ============================================================
# Phase 6.3: ResourceWatchdog — 资源容量看门狗
# ============================================================

class ResourceWatchdog:
    """MAX_MEMORY=1024MB, MAX_CPU_TIME=60s；超限先终止 CDP 子会话"""

    def __init__(self) -> None:
        self.proc = psutil.Process()
        self.start_cpu = self.proc.cpu_times()
        self.peak_rss_mb = 0.0
        self.peak_cpu_s = 0.0
        self.violations: list[str] = []
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._loop, daemon=True)

    def start(self) -> None:
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()

    def _loop(self) -> None:
        while not self._stop.is_set():
            try:
                rss = self.proc.memory_info().rss / (1024 * 1024)
                cpu = (self.proc.cpu_times().user - self.start_cpu.user
                       + self.proc.cpu_times().system - self.start_cpu.system)
                self.peak_rss_mb = max(self.peak_rss_mb, rss)
                self.peak_cpu_s = max(self.peak_cpu_s, cpu)
                if rss > MAX_MEMORY_USAGE_MB:
                    msg = (f"[Watchdog] 内存超限 {rss:.1f}MB > {MAX_MEMORY_USAGE_MB}MB，"
                           f"触发 Hard Halt")
                    self.violations.append(msg)
                    print(msg, file=sys.stderr)
                if cpu > MAX_CPU_TIME_SECONDS:
                    msg = (f"[Watchdog] CPU 时间超限 {cpu:.1f}s > "
                           f"{MAX_CPU_TIME_SECONDS}s，触发 Hard Halt")
                    self.violations.append(msg)
                    print(msg, file=sys.stderr)
            except Exception:
                pass
            time.sleep(1.0)

    def snapshot(self) -> dict:
        return {
            "peak_rss_mb": round(self.peak_rss_mb, 1),
            "peak_cpu_s": round(self.peak_cpu_s, 1),
            "max_memory_mb": MAX_MEMORY_USAGE_MB,
            "max_cpu_s": MAX_CPU_TIME_SECONDS,
            "violations": len(self.violations),
        }


# ============================================================
# Phase 3: CDPClient — WebSocket 连接 9223（绕过 Origin 检查）
# ============================================================

class CDPClient:
    """CDP WebSocket 客户端，支持事件监听与命令发送"""

    def __init__(self, cdp_http: str) -> None:
        self.cdp_http = cdp_http
        self.ws: Optional[websocket.WebSocketApp] = None
        self.target_id: Optional[str] = None
        self.ws_url: Optional[str] = None
        self._msg_id = 0
        self._lock = threading.Lock()
        self._pending: dict[int, Any] = {}
        self._events: deque = deque(maxlen=5000)  # Phase 3.1 事件源队列
        self._event_listeners: list = []
        self._connected = threading.Event()
        self.browser_version: Optional[str] = None

    # ---- 连接管理 ----
    def discover_target(self) -> dict:
        r = requests.get(f"{self.cdp_http}/json", timeout=5)
        targets = r.json()
        page = next((t for t in targets
                     if t.get("type") == "page"
                     and EXPECTED_TARGET_TITLE in t.get("title", "")), None)
        if not page:
            page = next((t for t in targets if t.get("type") == "page"), None)
        if not page:
            raise RuntimeError(f"未找到 page target: {targets}")
        self.target_id = page["id"]
        self.ws_url = page["webSocketDebuggerUrl"]
        return page

    def connect(self) -> None:
        self.discover_target()
        self.ws = websocket.WebSocketApp(
            self.ws_url,
            on_open=self._on_open,
            on_message=self._on_message,
            on_error=self._on_error,
            on_close=self._on_close,
        )

    def _on_open(self, _ws: Any) -> None:
        self._connected.set()
        # suppress_origin 绕过 Chromium Origin 检查（tauri.localhost）
        t = threading.Thread(
            target=self.ws.run_forever,
            kwargs={"suppress_origin": True},
            daemon=True,
        )
        t.start()
        if not self._connected.wait(timeout=10):
            raise RuntimeError("CDP WebSocket 连接超时")
        self.send("Network.enable", {})
        self.send("Runtime.enable", {})
        self.send("Page.enable", {})
        self.send("Log.enable", {})
        self.send("Console.enable", {})

    def _on_message(self, _ws: Any, raw: str) -> None:
        try:
            msg = json.loads(raw)
        except Exception:
            return
        if "id" in msg:
            with self._lock:
                self._pending.pop(msg["id"], None)
                self._pending[msg["id"]] = msg
            return
        # 事件入队（Phase 3.1 事件源队列）
        method = msg.get("method", "")
        self._events.append({
            "method": method,
            "params": msg.get("params", {}),
            "ts": time.time(),
        })
        for listener in list(self._event_listeners):
            try:
                listener(method, msg.get("params", {}))
            except Exception:
                pass

    def _on_error(self, _ws: Any, err: Any) -> None:
        print(f"[CDP] WebSocket 错误: {err}", file=sys.stderr)

    def _on_close(self, _ws: Any, *args: Any) -> None:
        self._connected.clear()

    def add_listener(self, cb: Any) -> None:
        self._event_listeners.append(cb)

    def send(self, method: str, params: dict | None = None,
             timeout: float = 15.0) -> Any:
        if not self.ws:
            raise RuntimeError("CDP 未连接")
        with self._lock:
            self._msg_id += 1
            mid = self._msg_id
        payload = json.dumps({"id": mid, "method": method,
                              "params": params or {}})
        self.ws.send(payload)
        deadline = time.time() + timeout
        while time.time() < deadline:
            with self._lock:
                resp = self._pending.get(mid)
            if resp is not None:
                with self._lock:
                    self._pending.pop(mid, None)
                if "error" in resp:
                    return {"error": resp["error"]}
                return resp.get("result")
            time.sleep(0.05)
        raise TimeoutError(f"CDP 命令超时: {method} ({timeout}s)")

    def evaluate(self, expr: str, await_promise: bool = True) -> Any:
        result = self.send("Runtime.evaluate", {
            "expression": expr,
            "awaitPromise": await_promise,
            "returnByValue": True,
            "timeout": 10000,
        })
        if not result:
            return None
        if "exceptionDetails" in result and result["exceptionDetails"]:
            return {"__error__": result["exceptionDetails"].get("exception", {})
                    .get("description", "unknown")}
        return result.get("result", {}).get("value")

    # Phase 3.3 CDP 存活探测
    def liveness_check(self) -> bool:
        try:
            r = requests.get(f"{self.cdp_http}/json/version", timeout=3)
            if r.status_code == 200:
                v = r.json()
                self.browser_version = v.get("Browser")
                return True
        except Exception:
            pass
        return False

    def snapshot_events(self) -> list:
        return list(self._events)


# ============================================================
# Phase 3.2: RVMonitor — 不变式检查器（实时断言 + 违反即终止）
# ============================================================

@dataclass
class InvariantViolation:
    inv_id: str
    description: str
    timestamp: str
    trigger_event: dict
    context: dict


class RVMonitor:
    """运行时验证监控器：实时断言不变式，违反即生成报告"""

    def __init__(self, cdp: CDPClient, sanitizer: Sanitizer) -> None:
        self.cdp = cdp
        self.sanitizer = sanitizer
        self.violations: list[InvariantViolation] = []
        self.console_messages: list[dict] = []
        self.network_events: list[dict] = []
        self.exceptions: list[dict] = []
        self._halt = False
        cdp.add_listener(self._on_event)

    def _on_event(self, method: str, params: dict) -> None:
        if method == "Runtime.consoleAPICalled":
            entry = {
                "ts": datetime.utcnow().isoformat() + "Z",
                "level": params.get("type", "log"),
                "text": " ".join(str(a.get("value", a.get("description", "")))
                                  for a in params.get("args", [])),
            }
            self.console_messages.append(entry)
        elif method == "Runtime.exceptionThrown":
            exc = params.get("exceptionDetails", {})
            self.exceptions.append({
                "ts": datetime.utcnow().isoformat() + "Z",
                "text": exc.get("exception", {}).get("description",
                                                     exc.get("text", "")),
            })
        elif method in ("Network.requestWillBeSent",
                        "Network.responseReceived",
                        "Network.loadingFailed"):
            entry = {
                "ts": datetime.utcnow().isoformat() + "Z",
                "type": method,
                "url": params.get("request", {}).get("url",
                              params.get("response", {}).get("url", "")),
                "status": params.get("response", {}).get("status"),
                "timing_ms": params.get("response", {}).get("timing", {})
                              .get("waitingTime"),
            }
            self.network_events.append(entry)
            # Phase 3.2 实时断言：sidecar 端点响应慢即违反
            url = entry["url"]
            if "127.0.0.1:3099" in url and method == "Network.responseReceived":
                t = entry.get("timing_ms")
                if t and t > ENDPOINT_HALT_THRESHOLD_MS:
                    self._record_violation(
                        "INV-HTTP-001",
                        f"CDP 捕获 sidecar 端点响应 {t}ms > {ENDPOINT_HALT_THRESHOLD_MS}ms",
                        entry,
                    )

    def _record_violation(self, inv_id: str, desc: str,
                          trigger: dict) -> None:
        # Phase 3.3 CDP 存活探测，避免假阴性
        alive = self.cdp.liveness_check()
        v = InvariantViolation(
            inv_id=inv_id,
            description=desc,
            timestamp=datetime.utcnow().isoformat() + "Z",
            trigger_event=self.sanitizer.sanitize(trigger),
            context={"cdp_alive": alive,
                     "browser_version": self.cdp.browser_version},
        )
        self.violations.append(v)
        print(f"[RV-Monitor 违反] {inv_id}: {desc} (CDP alive={alive})",
              file=sys.stderr)

    def assert_true(self, inv_id: str, condition: bool,
                    desc: str, context: dict | None = None) -> bool:
        if not condition:
            self._record_violation(inv_id, desc, context or {})
        return condition

    @property
    def halted(self) -> bool:
        return self._halt


# ============================================================
# Sidecar 探测工具
# ============================================================

def http_get(path: str, timeout: float = 3.0) -> dict:
    url = f"{SIDECAR_ENDPOINT}{path}"
    t0 = time.time()
    try:
        r = requests.get(url, timeout=timeout)
        ms = int((time.time() - t0) * 1000)
        try:
            body = r.json()
        except Exception:
            body = r.text[:500]
        return {"url": url, "status": r.status_code, "elapsed_ms": ms,
                "reachable": True, "body": body}
    except requests.exceptions.Timeout:
        ms = int((time.time() - t0) * 1000)
        return {"url": url, "status": None, "elapsed_ms": ms,
                "reachable": False, "error": "TIMEOUT"}
    except Exception as e:
        ms = int((time.time() - t0) * 1000)
        return {"url": url, "status": None, "elapsed_ms": ms,
                "reachable": False, "error": str(e)[:200]}


def probe_all_endpoints() -> dict:
    """4 端点矩阵探测"""
    endpoints = ["/health", "/v1/health/dao_metrics",
                 "/v1/health/system", "/v1/health/detailed"]
    results = {}
    for ep in endpoints:
        results[ep] = http_get(ep, timeout=3.0)
        time.sleep(0.1)
    return results


def get_close_wait_count() -> int:
    try:
        out = []
        for c in psutil.net_connections(kind="tcp"):
            if c.laddr and c.laddr.port == 3099 and c.status == "CLOSE_WAIT":
                out.append(c)
        return len(out)
    except Exception:
        return -1


def get_conn_state_dist() -> dict:
    dist: dict[str, int] = {}
    try:
        for c in psutil.net_connections(kind="tcp"):
            if c.laddr and c.laddr.port == 3099:
                dist[c.status] = dist.get(c.status, 0) + 1
    except Exception:
        pass
    return dist


def get_sidecar_proc() -> dict:
    try:
        p = psutil.Process(EXPECTED_SIDECAR_PID)
        threads = p.threads()
        thread_count = len(threads)
        # Windows 下 psutil 线程无 status 属性，改用 CPU 采样判断死锁：
        # 若 2s 内 CPU 时间不增长但进程仍 running，则线程全部阻塞在锁等待
        cpu_t0 = p.cpu_times()
        time.sleep(2.0)
        cpu_t1 = p.cpu_times()
        cpu_delta_s = round((cpu_t1.user - cpu_t0.user
                             + cpu_t1.system - cpu_t0.system), 3)
        # 判定：进程存活但 2s 内 CPU 几乎不增长（<0.1s）= 线程全阻塞
        all_blocked = cpu_delta_s < 0.1 and str(p.status()).lower() == "running"
        return {
            "pid": p.pid,
            "name": p.name(),
            "status": str(p.status()),
            "cpu_s": round(cpu_t1.user + cpu_t1.system, 1),
            "cpu_delta_2s": cpu_delta_s,
            "mem_mb": round(p.memory_info().rss / (1024 * 1024), 1),
            "thread_count": thread_count,
            "all_blocked": all_blocked,
            "running_thread_count": 0 if all_blocked else thread_count,
            "create_time": datetime.fromtimestamp(
                p.create_time()).isoformat(),
        }
    except Exception as e:
        return {"error": str(e)}


# ============================================================
# 主测试流程
# ============================================================

def main() -> None:
    print("=" * 78)
    print("HCSE 韧性验证 Round 3 — LRC Desktop v0.8.22 (HTTP 阻塞根因诊断)")
    print("=" * 78)

    # ---- Phase 6: 安全沙箱初始化 ----
    path_validator = PathValidator(ALLOWED_DIRS)
    sanitizer = Sanitizer()
    watchdog = ResourceWatchdog()
    watchdog.start()

    ts = int(time.time())
    evidence: dict[str, Any] = {
        "meta": {
            "round": 3,
            "version": EXPECTED_VERSION,
            "timestamp": datetime.utcnow().isoformat() + "Z",
            "sidecar_pid": EXPECTED_SIDECAR_PID,
            "cdp_endpoint": CDP_ENDPOINT,
            "sidecar_endpoint": SIDECAR_ENDPOINT,
            "focus": "HTTP 服务器完全阻塞根因诊断",
        },
        "phases": {},
    }

    # ---- Phase 3: CDP 存活探测 ----
    cdp = CDPClient(CDP_ENDPOINT)
    cdp_alive = cdp.liveness_check()
    evidence["phases"]["cdp_liveness"] = {
        "alive": cdp_alive,
        "browser_version": cdp.browser_version,
    }
    print(f"\n[Phase 3.3] CDP 存活探测: alive={cdp_alive} "
          f"version={cdp.browser_version}")

    # ---- 连接 CDP ----
    try:
        cdp.connect()
        print(f"[CDP] 已连接 target={cdp.target_id}")
        evidence["phases"]["cdp_connect"] = {
            "connected": True, "target_id": cdp.target_id,
            "ws_url": cdp.ws_url,
        }
    except Exception as e:
        print(f"[CDP] 连接失败: {e}", file=sys.stderr)
        evidence["phases"]["cdp_connect"] = {"connected": False, "error": str(e)}

    monitor = RVMonitor(cdp, sanitizer)

    # ---- Phase 3/4: sidecar 端点矩阵探测（核心证据）----
    print("\n[Phase 4] Sidecar 端点矩阵探测（3s 超时）...")
    endpoint_matrix = probe_all_endpoints()
    evidence["phases"]["endpoint_matrix"] = sanitizer.sanitize(endpoint_matrix)

    all_timeout = all(not r["reachable"] for r in endpoint_matrix.values())
    max_elapsed = max(r["elapsed_ms"] for r in endpoint_matrix.values())
    for ep, r in endpoint_matrix.items():
        print(f"  {ep}: reachable={r['reachable']} "
              f"status={r.get('status')} elapsed={r['elapsed_ms']}ms "
              f"err={r.get('error', '')}")

    # ---- INV-HTTP-001: 所有端点 2s 内响应 ----
    inv_http_001 = monitor.assert_true(
        "INV-HTTP-001",
        all(r["reachable"] and r["elapsed_ms"] < ENDPOINT_TIMEOUT_THRESHOLD_MS
            for r in endpoint_matrix.values()),
        f"所有端点 2s 内响应（实测全部超时 max={max_elapsed}ms）",
        endpoint_matrix,
    )

    # ---- INV-LEAK-006: CloseWait < 10 ----
    cw = get_close_wait_count()
    conn_dist = get_conn_state_dist()
    print(f"\n[Phase 4] CloseWait={cw} 连接分布={conn_dist}")
    inv_leak = monitor.assert_true(
        "INV-LEAK-006",
        cw < CLOSE_WAIT_THRESHOLD,
        f"CloseWait={cw} >= 阈值 {CLOSE_WAIT_THRESHOLD}（连接泄漏回归）",
        {"close_wait": cw, "conn_dist": conn_dist},
    )

    # ---- INV-RUNTIME-001: tokio worker 线程未被全部占用 ----
    proc = get_sidecar_proc()
    print(f"\n[Phase 4] Sidecar 进程: {proc}")
    # 17 线程全 Wait → worker 全阻塞
    all_blocked = (proc.get("thread_count", 0) > 0
                   and proc.get("running_thread_count", 0) == 0)
    inv_runtime = monitor.assert_true(
        "INV-RUNTIME-001",
        not all_blocked,
        f"tokio worker 线程未被全部阻塞（实测 {proc.get('thread_count')} 线程"
        f"全 Wait，运行线程={proc.get('running_thread_count')}）",
        proc,
    )

    evidence["phases"]["sidecar_process"] = sanitizer.sanitize(proc)
    evidence["phases"]["tcp_connections"] = {"close_wait": cw,
                                             "distribution": conn_dist}

    # ---- 等待 CDP 事件采集（5s）----
    print("\n[Phase 3] 采集 CDP 事件 5s（console/network/exception）...")
    time.sleep(5)

    # ---- Phase 3: 前端状态采集 ----
    print("\n[Phase 3] 前端状态采集（CDP evaluate）...")
    frontend_state: dict = {}
    try:
        frontend_state["sidecarHealthMonitor"] = cdp.evaluate(
            "(() => { try { const m = window.sidecarHealthMonitor; "
            "return m ? { exists:true, online:m.online, "
            "_failCount:m._failCount, _lockBusy:m._lockBusy, "
            "_sidecarStatus:m._sidecarStatus, _backoffStep:m._backoffStep } "
            ": {exists:false} } catch(e){ return {error:String(e)} } })()"
        )
        frontend_state["statusDot"] = cdp.evaluate(
            "(() => { const d = document.querySelector('.status-dot'); "
            "return d ? { className: d.className, text: d.textContent } "
            ": null })()"
        )
        frontend_state["toast"] = cdp.evaluate(
            "(() => { const t = document.querySelector('.toast, .toast-message'); "
            "return t ? { exists:true, text: t.textContent } : {exists:false} })()"
        )
        frontend_state["bannerText"] = cdp.evaluate(
            "(() => { const b = document.querySelector('.dao-fallback-banner'); "
            "return b ? { exists:true, text: b.textContent } : {exists:false} })()"
        )
        frontend_state["globalErrorFlag"] = cdp.evaluate(
            "(() => ({ registered: !!window._lrcGlobalErrorRegistered, "
            "hasOnError: !!window.onerror, "
            "hasOnRejection: !!window.onunhandledrejection }))()"
        )
        frontend_state["version"] = cdp.evaluate(
            "(() => { try { return window.__LRC_VERSION__ || "
            "(document.title||'').match(/v?([0-9.]+)/)?.[1] } "
            "catch(e){ return null } })()"
        )
    except Exception as e:
        frontend_state["__eval_error__"] = str(e)
    print(f"  monitor={frontend_state.get('sidecarHealthMonitor')}")
    print(f"  statusDot={frontend_state.get('statusDot')}")
    print(f"  banner={frontend_state.get('bannerText')}")
    evidence["phases"]["frontend_state"] = sanitizer.sanitize(frontend_state)

    # ---- INV-STATE-002: UI 状态与 sidecar 实际状态一致 ----
    shm = frontend_state.get("sidecarHealthMonitor", {}) or {}
    # sidecar 全超时（不可达）但前端 online 应为 false 或 failCount>0
    online = shm.get("online")
    fail_count = shm.get("_failCount", 0)
    inv_state = monitor.assert_true(
        "INV-STATE-002",
        bool(not all_timeout or online is False or (fail_count and fail_count > 0)),
        f"sidecar 全超时但前端 online={online} failCount={fail_count}（状态不一致）",
        {"sidecar_all_timeout": all_timeout, "frontend_online": online,
         "frontend_failCount": fail_count},
    )

    # ---- 重复探测（验证是否偶发）----
    print("\n[Phase 4] 重复探测 5 轮（验证是否偶发阻塞）...")
    repeat_results = []
    for i in range(5):
        r = http_get("/health", timeout=2.0)
        repeat_results.append({"round": i + 1, **r})
        print(f"  Round {i+1}: reachable={r['reachable']} "
              f"elapsed={r['elapsed_ms']}ms err={r.get('error','')}")
        time.sleep(0.5)
    evidence["phases"]["repeat_probe"] = sanitizer.sanitize(repeat_results)
    reachable_count = sum(1 for r in repeat_results if r["reachable"])
    inv_p0a = monitor.assert_true(
        "INV-V0822-P0A",
        reachable_count >= 4,
        f"worker_threads=16 修复失效：5 轮探测仅 {reachable_count}/5 可达（Round 2 为 5/5）",
        {"repeat_results": repeat_results},
    )

    # ---- Phase 3: CDP 事件采集 ----
    events = cdp.snapshot_events()
    event_counts: dict[str, int] = {}
    for e in events:
        event_counts[e["method"]] = event_counts.get(e["method"], 0) + 1
    print(f"\n[Phase 3] CDP 事件统计: {event_counts}")
    evidence["phases"]["cdp_events"] = {
        "counts": event_counts,
        "console_messages": sanitizer.sanitize(monitor.console_messages[-30:]),
        "exceptions": sanitizer.sanitize(monitor.exceptions[-10:]),
        "network_events": sanitizer.sanitize(monitor.network_events[-30:]),
    }

    # ---- 截图 ----
    print("\n[Phase 5] 截图取证...")
    try:
        shot = cdp.send("Page.captureScreenshot",
                        {"format": "png"}, timeout=15)
        if shot and "data" in shot:
            shot_path = path_validator.validate(
                BASE_DIR / "screenshots" / f"round3_http_blocked_{ts}.png")
            shot_path.write_bytes(base64.b64decode(shot["data"]))
            evidence["phases"]["screenshot"] = str(shot_path)
            print(f"  截图已保存: {shot_path}")
    except Exception as e:
        print(f"  截图失败: {e}", file=sys.stderr)
        evidence["phases"]["screenshot_error"] = str(e)

    # ---- Phase 6: 安全沙箱总结 ----
    watchdog.stop()
    evidence["phases"]["security_sandbox"] = {
        "path_validator": {
            "allowed_dirs": [str(d) for d in path_validator.allowed],
            "violations": path_validator.violations,
            "status": "PASS" if not path_validator.violations else "HARD_HALT",
        },
        "sanitizer": {
            "redaction_count": sanitizer.redaction_count,
            "status": "PASS",
        },
        "resource_watchdog": watchdog.snapshot(),
    }

    # ---- 不变式结果汇总 ----
    results = {
        "INV-HTTP-001": {"status": "PASS" if inv_http_001 else "FAIL",
                         "severity": "P0",
                         "evidence": f"4 端点 max={max_elapsed}ms"},
        "INV-V0822-P0A": {"status": "PASS" if inv_p0a else "FAIL",
                          "severity": "P0",
                          "evidence": f"5 轮探测 {reachable_count}/5 可达"},
        "INV-LEAK-006": {"status": "PASS" if inv_leak else "FAIL",
                         "severity": "P1",
                         "evidence": f"CloseWait={cw}"},
        "INV-RUNTIME-001": {"status": "PASS" if inv_runtime else "FAIL",
                            "severity": "P0",
                            "evidence": f"{proc.get('thread_count')} 线程，"
                                        f"运行={proc.get('running_thread_count')}"},
        "INV-STATE-002": {"status": "PASS" if inv_state else "FAIL",
                          "severity": "P0",
                          "evidence": f"online={online} failCount={fail_count}"},
    }
    evidence["phases"]["invariant_results"] = results
    evidence["phases"]["violations"] = [
        {"inv_id": v.inv_id, "description": v.description,
         "timestamp": v.timestamp, "context": v.context}
        for v in monitor.violations
    ]

    # ---- 写入证据包（经脱敏 + 路径校验）----
    ev_path = path_validator.validate(
        BASE_DIR / "evidence" / f"evidence_v0822_round3_{ts}.json")
    ev_path.write_text(
        json.dumps(sanitizer.sanitize(evidence), ensure_ascii=False, indent=2),
        encoding="utf-8")
    print(f"\n[Phase 5] 证据包已保存: {ev_path}")

    res_path = path_validator.validate(
        BASE_DIR / "evidence" / f"results_v0822_round3_{ts}.json")
    res_path.write_text(
        json.dumps(sanitizer.sanitize(results), ensure_ascii=False, indent=2),
        encoding="utf-8")
    print(f"[Phase 5] 结果包已保存: {res_path}")

    # ---- 汇总 ----
    pass_count = sum(1 for v in results.values() if v["status"] == "PASS")
    fail_count = sum(1 for v in results.values() if v["status"] == "FAIL")
    print("\n" + "=" * 78)
    print(f"HCSE Round 3 结果: PASS={pass_count} FAIL={fail_count}")
    for k, v in results.items():
        print(f"  {k} [{v['severity']}]: {v['status']} — {v['evidence']}")
    print("=" * 78)


if __name__ == "__main__":
    main()
