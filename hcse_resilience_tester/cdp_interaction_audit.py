"""
LRC Desktop v0.8.21 五层交互韧性审计 — CDP 真实交互测试

测试方法：
  - 通过 CDP（端口 9223）连接到正在运行的 lrc-desktop.exe
  - 使用 Runtime.evaluate 检查前端状态
  - 使用 Network.setRequestInterception 模拟 sidecar 异常（不真实关闭 sidecar）
  - 使用 Page.captureScreenshot 保存证据
  - 通过 sidecar HTTP API（端口 3099）验证后端状态

覆盖 25 个测试点（L1-L5 × 5 类异常路径）：
  L1 一级页面：加载失败/数据为空/超时/卡死/错误
  L2 二级弹窗：打开失败/操作超时/取消中断/数据丢失/竞态
  L3 三级卡片：加载失败/无响应/数据为空/超时/竞态
  L4 四级嵌套：超时/状态不恢复/验证失败/取消/竞态
  L5 异常全局：网络断开/进程崩溃/资源耗尽/全局错误/跨层级竞态
"""

from __future__ import annotations

import base64
import json
import os
import re
import sys
import time
import threading
from collections import deque
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Any, Optional

import requests
import websocket  # type: ignore

# ============================================================
# 常量
# ============================================================

CDP_ENDPOINT = "http://127.0.0.1:9223"
SIDECAR_ENDPOINT = "http://127.0.0.1:3099"
EXPECTED_VERSION = "0.8.21"

REPORT_DIR = Path("g:/code-memory/hcse_resilience_tester/interaction_audit")
SCREENSHOT_DIR = REPORT_DIR / "screenshots"
LOG_DIR = REPORT_DIR / "logs"
REPORT_DIR.mkdir(parents=True, exist_ok=True)
SCREENSHOT_DIR.mkdir(parents=True, exist_ok=True)
LOG_DIR.mkdir(parents=True, exist_ok=True)


# ============================================================
# 测试结果数据结构
# ============================================================

@dataclass
class TestResult:
    test_id: str
    layer: str  # L1-L5
    category: str  # 加载失败/超时/卡死/错误/取消/竞态 等
    title: str
    status: str = "PENDING"  # PASS / FAIL / PARTIAL / SKIP / BLOCKED
    severity: str = ""  # P0/P1/P2
    description: str = ""
    evidence: list[str] = field(default_factory=list)
    code_location: str = ""
    repro_steps: list[str] = field(default_factory=list)
    fix_suggestion: str = ""
    global_impact: str = ""
    timestamp: str = ""
    duration_ms: int = 0

    def to_dict(self) -> dict:
        return {
            "test_id": self.test_id,
            "layer": self.layer,
            "category": self.category,
            "title": self.title,
            "status": self.status,
            "severity": self.severity,
            "description": self.description,
            "evidence": self.evidence,
            "code_location": self.code_location,
            "repro_steps": self.repro_steps,
            "fix_suggestion": self.fix_suggestion,
            "global_impact": self.global_impact,
            "timestamp": self.timestamp,
            "duration_ms": self.duration_ms,
        }


# ============================================================
# CDP 客户端
# ============================================================

class CDPClient:
    """轻量级 CDP 客户端，通过 WebSocket 与 Tauri WebView2 通信"""

    def __init__(self, cdp_endpoint: str = CDP_ENDPOINT):
        self.cdp_endpoint = cdp_endpoint
        self.ws: Optional[websocket.WebSocket] = None
        self.msg_id = 0
        self.target_id: Optional[str] = None
        self.ws_url: Optional[str] = None
        self._lock = threading.Lock()
        self.console_logs: deque = deque(maxlen=200)

    def connect(self) -> None:
        """连接到 CDP，自动选择 LRC Desktop 目标页面"""
        resp = requests.get(f"{self.cdp_endpoint}/json", timeout=5)
        targets = resp.json()
        lrc_target = None
        for t in targets:
            if t.get("type") == "page" and "龙忆" in t.get("title", ""):
                lrc_target = t
                break
        if not lrc_target:
            raise RuntimeError(f"未找到 LRC Desktop 目标页面，可用目标: {targets}")
        self.target_id = lrc_target["id"]
        self.ws_url = lrc_target["webSocketDebuggerUrl"]
        # WebView2/Chromium 要求 Origin 为空或 devtools://，否则 403
        # 使用 suppress_origin 完全不发 Origin header（Chromium 接受无 Origin 的连接）
        self.ws = websocket.create_connection(
            self.ws_url, timeout=15,
            suppress_origin=True,
        )
        print(f"[CDP] 已连接到 {lrc_target['title']} (id={self.target_id})")

    def _send(self, method: str, params: dict | None = None) -> dict:
        """发送 CDP 命令并等待结果"""
        if self.ws is None:
            raise RuntimeError("CDP 未连接")
        with self._lock:
            self.msg_id += 1
            msg = {"id": self.msg_id, "method": method, "params": params or {}}
            self.ws.send(json.dumps(msg))
            # 等待对应 id 的响应（跳过事件）
            deadline = time.time() + 30
            while time.time() < deadline:
                raw = self.ws.recv()
                data = json.loads(raw)
                # 记录 console 日志
                if data.get("method") == "Runtime.consoleAPICalled":
                    args = data["params"].get("args", [])
                    text = " ".join(str(a.get("value", a.get("description", ""))) for a in args)
                    self.console_logs.append({
                        "type": data["params"].get("type", ""),
                        "text": text,
                        "ts": datetime.utcnow().isoformat() + "Z",
                    })
                if data.get("id") == self.msg_id:
                    if "error" in data:
                        raise RuntimeError(f"CDP 错误: {data['error']}")
                    return data.get("result", {})
            raise TimeoutError(f"CDP 命令超时: {method}")

    def enable_runtime(self) -> None:
        self._send("Runtime.enable")
        self._send("Page.enable")
        self._send("Network.enable")

    def evaluate(self, expression: str, await_promise: bool = True, timeout_ms: int = 20000) -> Any:
        """执行 JS 表达式并返回结果"""
        result = self._send("Runtime.evaluate", {
            "expression": expression,
            "awaitPromise": await_promise,
            "returnByValue": True,
            "timeout": timeout_ms,
        })
        if "exceptionDetails" in result:
            exc = result["exceptionDetails"]
            err_text = exc.get("exception", {}).get("description", str(exc))
            raise RuntimeError(f"JS 执行错误: {err_text}")
        return result.get("result", {}).get("value")

    def screenshot(self, filename: str) -> str:
        """截图并保存到 screenshots 目录，返回路径"""
        result = self._send("Page.captureScreenshot", {"format": "png"})
        data = result.get("data", "")
        path = SCREENSHOT_DIR / filename
        path.write_bytes(base64.b64decode(data))
        return str(path)

    def get_console_logs(self, since_seconds: int = 60) -> list[dict]:
        """获取最近 N 秒的 console 日志"""
        cutoff = datetime.utcnow().timestamp() - since_seconds
        return [l for l in self.console_logs
                if datetime.fromisoformat(l["ts"].replace("Z", "")).timestamp() >= cutoff]

    def clear_console_logs(self) -> None:
        self.console_logs.clear()

    def close(self) -> None:
        if self.ws:
            self.ws.close()


# ============================================================
# Sidecar HTTP 探针
# ============================================================

class SidecarProbe:
    def __init__(self, endpoint: str = SIDECAR_ENDPOINT):
        self.endpoint = endpoint

    def health(self, timeout: float = 3.0) -> dict:
        try:
            r = requests.get(f"{self.endpoint}/health", timeout=timeout)
            return {"ok": r.ok, "status_code": r.status_code, "body": r.json() if r.ok else r.text}
        except Exception as e:
            return {"ok": False, "error": str(e)}

    def dao_metrics(self, timeout: float = 3.0) -> dict:
        try:
            r = requests.get(f"{self.endpoint}/v1/dao/metrics", timeout=timeout)
            return {"ok": r.ok, "status_code": r.status_code,
                    "body": r.json() if r.ok else r.text}
        except Exception as e:
            return {"ok": False, "error": str(e)}

    def dashboard(self, timeout: float = 3.0) -> dict:
        try:
            r = requests.get(f"{self.endpoint}/v1/dashboard", timeout=timeout)
            return {"ok": r.ok, "status_code": r.status_code,
                    "body": r.json() if r.ok else r.text}
        except Exception as e:
            return {"ok": False, "error": str(e)}

    def memories(self, timeout: float = 3.0) -> dict:
        try:
            r = requests.get(f"{self.endpoint}/v1/memories?limit=5", timeout=timeout)
            return {"ok": r.ok, "status_code": r.status_code,
                    "body": r.json() if r.ok else r.text}
        except Exception as e:
            return {"ok": False, "error": str(e)}


# ============================================================
# 测试基类
# ============================================================

class InteractionTest:
    def __init__(self, cdp: CDPClient, sidecar: SidecarProbe):
        self.cdp = cdp
        self.sidecar = sidecar
        self.results: list[TestResult] = []

    def record(self, result: TestResult) -> None:
        result.timestamp = datetime.utcnow().isoformat() + "Z"
        self.results.append(result)
        status_emoji = {"PASS": "[OK]", "FAIL": "[FAIL]", "PARTIAL": "[PART]",
                        "SKIP": "[SKIP]", "BLOCKED": "[BLK]"}.get(result.status, "[?]")
        print(f"  {status_emoji} {result.test_id} {result.title} ({result.status})"
              + (f" severity={result.severity}" if result.severity else ""))

    def snapshot_ui_state(self) -> dict:
        """采集当前 UI 关键状态"""
        return self.cdp.evaluate("""
            (function() {
                const state = {
                    title: document.title,
                    url: location.hash,
                    sidecarBannerVisible: !!document.querySelector('#sidecar-down-banner') &&
                        !document.querySelector('#sidecar-down-banner').hidden,
                    sidecarBannerText: document.querySelector('#sidecar-down-banner .banner-text')?.textContent || '',
                    toasts: Array.from(document.querySelectorAll('.toast')).map(t => ({
                        text: t.textContent?.trim(),
                        class: t.className,
                    })),
                    modalsOpen: Array.from(document.querySelectorAll('.modal')).filter(m =>
                        m.style.display !== 'none' && getComputedStyle(m).display !== 'none'
                    ).map(m => ({ id: m.id, class: m.className })),
                    activeTab: document.querySelector('.nav-item.active')?.getAttribute('data-tab') || '',
                    statusBarText: document.querySelector('.status-bar')?.textContent?.trim() || '',
                    dashboardCards: Array.from(document.querySelectorAll('.card')).map(c => ({
                        title: c.querySelector('.card-title, h3')?.textContent?.trim() || '',
                        text: c.textContent?.trim().substring(0, 200),
                    })),
                    daoMetricsText: document.querySelector('#dao-metrics, [data-component=\"dao-metrics\"]')?.textContent?.trim() || '',
                    memoryCount: document.querySelectorAll('.memory-card, .memory-item').length,
                    buttonsLoading: Array.from(document.querySelectorAll('button.is-loading, button[disabled]')).map(b => ({
                        text: b.textContent?.trim(),
                        class: b.className,
                    })),
                    errorMessages: Array.from(document.querySelectorAll('.error, .error-message, .has-error')).map(e => ({
                        text: e.textContent?.trim(),
                    })),
                    lockBusy: window.sidecarHealthMonitor?._lockBusy || false,
                    sidecarStatus: window.sidecarHealthMonitor?._sidecarStatus || 'unknown',
                };
                return state;
            })()
        """, await_promise=False)

    def wait_for(self, js_predicate: str, timeout_ms: int = 5000, interval_ms: int = 300) -> bool:
        """轮询等待 JS 谓词返回 true"""
        deadline = time.time() + timeout_ms / 1000
        while time.time() < deadline:
            try:
                ok = self.cdp.evaluate(f"({js_predicate})", await_promise=False, timeout_ms=2000)
                if ok:
                    return True
            except Exception:
                pass
            time.sleep(interval_ms / 1000)
        return False


# ============================================================
# 主入口
# ============================================================

def main():
    print("=" * 72)
    print("LRC Desktop v0.8.21 五层交互韧性审计 — CDP 真实交互测试")
    print("=" * 72)

    cdp = CDPClient()
    sidecar = SidecarProbe()
    try:
        cdp.connect()
        cdp.enable_runtime()
    except Exception as e:
        print(f"[FATAL] CDP 连接失败: {e}")
        sys.exit(1)

    # 健康检查
    h = sidecar.health()
    print(f"[Probe] sidecar health: {h}")
    if not h.get("ok"):
        print("[WARN] sidecar 不可达，L1 测试将记录此状态")

    # 采集基线 UI 状态
    baseline = TestResult(
        test_id="BASELINE-00",
        layer="L0",
        category="baseline",
        title="采集基线 UI 状态",
    )
    try:
        ui = cdp.evaluate("""
            (function() {
                return {
                    title: document.title,
                    url: location.hash,
                    bodyTextLen: document.body.innerText.length,
                    navItems: Array.from(document.querySelectorAll('.nav-item')).map(n => n.getAttribute('data-tab')),
                    hasSidecarMonitor: !!window.sidecarHealthMonitor,
                    sidecarStatus: window.sidecarHealthMonitor?._sidecarStatus || 'unknown',
                    lockBusy: window.sidecarHealthMonitor?._lockBusy || false,
                };
            })()
        """, await_promise=False)
        baseline.description = f"基线 UI 状态: {json.dumps(ui, ensure_ascii=False)}"
        baseline.status = "PASS"
        baseline.evidence.append(f"UI 状态: {ui}")
        screenshot_path = cdp.screenshot("baseline.png")
        baseline.evidence.append(f"截图: {screenshot_path}")
    except Exception as e:
        baseline.status = "FAIL"
        baseline.description = f"基线采集失败: {e}"
    print(f"[BASELINE] {baseline.description[:200]}")

    # 此处后续测试由各层测试模块填充
    print("\n[INFO] 基础设施已就绪。后续测试由各层测试模块（layer_tests.py）填充。")
    print(f"[INFO] console 日志缓冲区: {len(cdp.console_logs)} 条")

    cdp.close()
    print("\n[DONE] CDP 连接已关闭")


if __name__ == "__main__":
    main()
