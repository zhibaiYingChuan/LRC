# -*- coding: utf-8 -*-
"""
LRC Desktop v0.8.22 五层交互韧性审计 - Round 3
重点审计: lock_busy 提示循环根因

通过 CDP WebSocket 连接到 Tauri 桌面端（端口 9223），采集运行时状态证据。
"""
import json
import time
import sys
import traceback
from datetime import datetime, timezone

import websocket  # type: ignore
import urllib.request

# CDP 配置
CDP_HTTP = "http://127.0.0.1:9223"
SIDECAR_HTTP = "http://127.0.0.1:3099"
EVIDENCE_PATH = r"g:\code-memory\hcse_resilience_tester\evidence\round3_lockbusy_evidence.json"
REPORT_PATH = r"g:\code-memory\hcse_resilience_tester\v0.8.22_interaction_audit_round3.md"


def log(msg):
    print(f"[{datetime.now(timezone.utc).strftime('%H:%M:%S.%f')[:-3]}] {msg}", flush=True)


def get_cdp_ws_url():
    """通过 CDP HTTP API 获取 target 的 WebSocket URL"""
    try:
        with urllib.request.urlopen(f"{CDP_HTTP}/json/list", timeout=5) as resp:
            data = json.loads(resp.read().decode("utf-8"))
        for t in data:
            if t.get("type") == "page" and "tauri.localhost" in t.get("url", ""):
                return t["webSocketDebuggerUrl"], t["id"], t.get("title", "")
        if data:
            return data[0]["webSocketDebuggerUrl"], data[0]["id"], data[0].get("title", "")
    except Exception as e:
        log(f"获取 CDP target 失败: {e}")
    return None, None, None


class CDPSession:
    def __init__(self, ws_url):
        self.ws = websocket.create_connection(ws_url, timeout=15, suppress_origin=True)
        self.msg_id = 0

    def send(self, method, params=None):
        self.msg_id += 1
        msg = {"id": self.msg_id, "method": method}
        if params:
            msg["params"] = params
        self.ws.send(json.dumps(msg))
        # 等待响应（跳过事件通知）
        deadline = time.time() + 15
        while time.time() < deadline:
            raw = self.ws.recv()
            data = json.loads(raw)
            if data.get("id") == self.msg_id:
                return data
        return {"error": {"message": "timeout"}}

    def evaluate(self, expression, await_promise=False):
        params = {"expression": expression, "returnByValue": True, "awaitPromise": await_promise}
        return self.send("Runtime.evaluate", params)

    def close(self):
        try:
            self.ws.close()
        except Exception:
            pass


def safe_json_fetch(url, timeout=3):
    """探测 sidecar HTTP，返回 (status_code, body, latency_ms, error)"""
    t0 = time.time()
    try:
        req = urllib.request.Request(url)
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            body = resp.read().decode("utf-8", errors="replace")
            return resp.status, body, int((time.time() - t0) * 1000), None
    except urllib.error.HTTPError as e:
        try:
            body = e.read().decode("utf-8", errors="replace")
        except Exception:
            body = ""
        return e.code, body, int((time.time() - t0) * 1000), None
    except Exception as e:
        return None, None, int((time.time() - t0) * 1000), str(e)


# ============================================================
# 审计脚本
# ============================================================

def probe_sidecar_endpoints():
    """探测 sidecar 4 个核心端点的可达性"""
    log("=== Step 1: 探测 sidecar 4 个端点 ===")
    endpoints = [
        "/health",
        "/v1/health/system",
        "/v1/health/detailed",
        "/v1/health/dao_metrics",
    ]
    results = {}
    for ep in endpoints:
        code, body, lat, err = safe_json_fetch(f"{SIDECAR_HTTP}{ep}", timeout=4)
        results[ep] = {
            "status_code": code,
            "latency_ms": lat,
            "error": err,
            "body_preview": (body or "")[:300],
        }
        log(f"  {ep}: status={code}, latency={lat}ms, err={err}")
    return results


def collect_frontend_state(cdp):
    """采集前端运行时状态"""
    log("=== Step 2: 采集前端运行时状态 ===")
    expr = r"""
    (() => {
      const out = {};
      // APP_VERSION
      out.appVersion = (typeof APP_VERSION !== 'undefined') ? APP_VERSION : null;
      // SidecarHealthMonitor 完整状态
      const m = window.sidecarHealthMonitor;
      out.monitor = m ? {
        exists: true,
        isReachable: m._isReachable,
        sidecarStatus: m._sidecarStatus,
        lockBusy: m._lockBusy,
        failCount: m._failCount,
        failThreshold: m._FAIL_THRESHOLD,
        backoffStep: m._backoffStep,
        pollInterval: m._pollInterval,
        inFlight: m._inFlight,
        isIndexing: (typeof m.isIndexing === 'function') ? m.isIndexing() : null,
        getSidecarStatus: (typeof m.getSidecarStatus === 'function') ? m.getSidecarStatus() : null,
        hasCheck: typeof m.check === 'function',
        hasStart: typeof m.start === 'function',
      } : { exists: false };
      // daoAbortController
      out.daoAbortController = window.daoAbortController ? {
        exists: true,
        signalAborted: window.daoAbortController.signal ? window.daoAbortController.signal.aborted : null,
      } : { exists: false };
      // 全局错误处理注册
      out.globalErrorRegistered = window._lrcGlobalErrorRegistered === true;
      // 重试计数器（_retryCounters 是 Map）
      try {
        const rc = (typeof _retryCounters !== 'undefined') ? _retryCounters : null;
        if (rc && rc instanceof Map) {
          out.retryCounters = Object.fromEntries(rc.entries());
        } else {
          out.retryCounters = null;
        }
      } catch(e) { out.retryCounters = { error: String(e) }; }
      // dao 重试计数器
      try {
        out.daoRetryCount = (typeof _daoRetryCount !== 'undefined') ? _daoRetryCount : null;
        out.daoRetryTimerExists = (typeof _daoRetryTimer !== 'undefined') ? (_daoRetryTimer !== null) : null;
        out.daoMaxRetries = (typeof _DAO_MAX_RETRIES !== 'undefined') ? _DAO_MAX_RETRIES : null;
      } catch(e) { out.daoRetry = { error: String(e) }; }
      // dashboard 重试计数器
      try {
        out.dashboardRetryCount = (typeof _dashboardRetryCount !== 'undefined') ? _dashboardRetryCount : null;
        out.dashboardRetryTimerExists = (typeof _dashboardRetryTimer !== 'undefined') ? (_dashboardRetryTimer !== null) : null;
        out.dashboardMaxRetries = (typeof _DASHBOARD_MAX_RETRIES !== 'undefined') ? _DASHBOARD_MAX_RETRIES : null;
      } catch(e) { out.dashboardRetry = { error: String(e) }; }
      // 状态栏 DOM
      const statusEl = document.querySelector('.status-bar, #status-bar, .status-dot');
      out.statusBar = {
        text: statusEl ? statusEl.textContent.trim() : null,
        outerHTML: statusEl ? statusEl.outerHTML.substring(0, 500) : null,
      };
      // 状态栏 dot class
      const dotEl = document.querySelector('.status-dot');
      out.statusDot = dotEl ? { className: dotEl.className, text: dotEl.textContent.trim() } : null;
      // 状态栏父级完整文本
      const sbParent = document.querySelector('.status-bar') || document.getElementById('status-bar');
      out.statusBarFull = sbParent ? sbParent.textContent.replace(/\s+/g, ' ').trim().substring(0, 300) : null;
      // 道同构度卡片
      const daoScore = document.getElementById('dao-ring-score');
      out.daoRingScore = daoScore ? daoScore.textContent.trim() : null;
      // 道同构度降级横幅
      const daoFallback = document.querySelector('.dao-fallback-banner');
      out.daoFallbackBanner = daoFallback ? {
        text: daoFallback.textContent.replace(/\s+/g, ' ').trim().substring(0, 300),
        outerHTML: daoFallback.outerHTML.substring(0, 600),
      } : null;
      // 道同构度索引提示
      const daoIdx = document.querySelector('.dao-indexing-hint');
      out.daoIndexingHint = daoIdx ? daoIdx.textContent.replace(/\s+/g, ' ').trim() : null;
      // 仪表盘错误
      const dashErr = document.getElementById('dashboard-error');
      out.dashboardError = dashErr ? {
        visible: dashErr.classList.contains('show'),
        text: dashErr.textContent.replace(/\s+/g, ' ').trim().substring(0, 300),
        innerHTML_preview: dashErr.innerHTML.substring(0, 500),
      } : null;
      // 仪表盘 loading
      const dashLoad = document.getElementById('dashboard-loading');
      out.dashboardLoading = dashLoad ? { visible: !dashLoad.classList.contains('hidden') } : null;
      // Toast 容器
      const toastC = document.getElementById('toast-container');
      out.toasts = {
        containerExists: !!toastC,
        count: toastC ? toastC.children.length : 0,
        messages: toastC ? Array.from(toastC.children).map(t => t.textContent.replace(/\s+/g, ' ').trim().substring(0, 200)) : [],
      };
      // 当前 active tab
      const activeTab = document.querySelector('[data-tab].active, .nav-item.active, .navbar-nav button.active');
      out.activeTab = activeTab ? activeTab.getAttribute('data-tab') : null;
      // sidecar-down-banner
      const banner = document.getElementById('sidecar-down-banner');
      out.sidecarDownBanner = banner ? { hidden: banner.hidden, text: banner.textContent.replace(/\s+/g, ' ').trim().substring(0, 200) } : null;
      // pendingRequestCount
      out.pendingRequestCount = (typeof window.__getPendingRequestCount === 'function') ? window.__getPendingRequestCount() : null;
      // _pendingBackgroundCount
      try {
        out.pendingBackgroundCount = (typeof _pendingBackgroundCount !== 'undefined') ? _pendingBackgroundCount : null;
      } catch(e) { out.pendingBackgroundCount = { error: String(e) }; }
      return out;
    })()
    """
    r = cdp.evaluate(expr)
    if "result" in r and "result" in r["result"]:
        result = r["result"]["result"].get("value")
        log(f"  采集完成: monitor.lockBusy={result.get('monitor', {}).get('lockBusy')}, "
            f"sidecarStatus={result.get('monitor', {}).get('sidecarStatus')}, "
            f"failCount={result.get('monitor', {}).get('failCount')}, "
            f"toasts.count={result.get('toasts', {}).get('count')}, "
            f"retryCounters={result.get('retryCounters')}")
        return result
    log(f"  ERROR: {r.get('error', r)}")
    return None


def inspect_handleHttpError_503(cdp):
    """检查 handleHttpError 503 分支的实现细节"""
    log("=== Step 3: 检查 handleHttpError 503 分支 ===")
    expr = r"""
    (() => {
      const out = {};
      // handleHttpError 源码
      if (typeof handleHttpError === 'function') {
        out.handleHttpErrorSrc = handleHttpError.toString();
      }
      // showToast 源码
      if (typeof showToast === 'function') {
        out.showToastSrc = showToast.toString().substring(0, 1500);
      }
      // MAX_RETRY_COUNT
      out.maxRetryCount = (typeof MAX_RETRY_COUNT !== 'undefined') ? MAX_RETRY_COUNT : null;
      // _retryModalActive
      out.retryModalActive = (typeof _retryModalActive !== 'undefined') ? _retryModalActive : null;
      // 503 分支关键代码片段
      if (out.handleHttpErrorSrc) {
        const idx = out.handleHttpErrorSrc.indexOf('503');
        out.lockbusy_branch_preview = idx >= 0 ? out.handleHttpErrorSrc.substring(idx, idx + 800) : null;
      }
      return out;
    })()
    """
    r = cdp.evaluate(expr)
    if "result" in r and "result" in r["result"]:
        result = r["result"]["result"].get("value")
        log(f"  handleHttpError 长度: {len(result.get('handleHttpErrorSrc') or '')}")
        log(f"  503 分支预览: {(result.get('lockbusy_branch_preview') or '')[:200]}")
        return result
    return None


def inspect_loadDaoMetrics_catch(cdp):
    """检查 loadDaoMetrics catch 块的实现细节"""
    log("=== Step 4: 检查 loadDaoMetrics catch 块 ===")
    expr = r"""
    (() => {
      const out = {};
      if (typeof loadDaoMetrics === 'function') {
        const src = loadDaoMetrics.toString();
        out.loadDaoMetricsSrc = src;
        // 提取 catch 块关键部分
        const catchIdx = src.indexOf('catch');
        out.catchBlockPreview = catchIdx >= 0 ? src.substring(catchIdx, catchIdx + 1500) : null;
        // 是否检查 err.status === 503
        out.hasStatus503Check = src.includes('err.status === 503') || src.includes("err.status===503");
        // 是否检查 _lockBusy
        out.hasLockBusyCheck = src.includes('_lockBusy');
        // 是否调用 _applyDaoMetricsFallback
        out.hasApplyFallback = src.includes('_applyDaoMetricsFallback');
      }
      return out;
    })()
    """
    r = cdp.evaluate(expr)
    if "result" in r and "result" in r["result"]:
        result = r["result"]["result"].get("value")
        log(f"  loadDaoMetrics 长度: {len(result.get('loadDaoMetricsSrc') or '')}")
        log(f"  hasStatus503Check: {result.get('hasStatus503Check')}")
        log(f"  hasLockBusyCheck: {result.get('hasLockBusyCheck')}")
        return result
    return None


def inspect_loadDashboard_catch(cdp):
    """检查 loadDashboard catch 块的 LOCK_BUSY 处理"""
    log("=== Step 5: 检查 loadDashboard catch 块 ===")
    expr = r"""
    (() => {
      const out = {};
      if (typeof loadDashboard === 'function') {
        const src = loadDashboard.toString();
        out.loadDashboardSrc = src;
        // LOCK_BUSY 处理
        const lbIdx = src.indexOf('LOCK_BUSY');
        out.lockbusyBlockPreview = lbIdx >= 0 ? src.substring(lbIdx, lbIdx + 1000) : null;
        // 是否检查 hasLockBusy
        out.hasHasLockBusyCheck = src.includes('hasLockBusy');
        // 是否调用 handleHttpError
        out.callsHandleHttpError = src.includes('handleHttpError');
      }
      return out;
    })()
    """
    r = cdp.evaluate(expr)
    if "result" in r and "result" in r["result"]:
        result = r["result"]["result"].get("value")
        log(f"  loadDashboard 长度: {len(result.get('loadDashboardSrc') or '')}")
        log(f"  hasHasLockBusyCheck: {result.get('hasHasLockBusyCheck')}")
        log(f"  callsHandleHttpError: {result.get('callsHandleHttpError')}")
        return result
    return None


def inspect_toast_dedup(cdp):
    """检查 showToast 去重逻辑"""
    log("=== Step 6: 检查 showToast 去重逻辑 ===")
    expr = r"""
    (() => {
      const out = {};
      if (typeof showToast === 'function') {
        const src = showToast.toString();
        out.showToastSrc = src;
        // 去重时间窗口
        out.hasDedup = src.includes('1500') || src.includes('dedup') || src.includes('lastToast');
        // 上限检查
        out.hasMaxLimit = src.includes('>= 3') || src.includes('> 3') || src.includes('max');
        // 关键片段：去重逻辑
        const dedupIdx = src.indexOf('去重');
        out.dedupPreview = dedupIdx >= 0 ? src.substring(dedupIdx, dedupIdx + 500) : null;
      }
      // 测试：连续触发 5 个相同 toast，看实际显示几个
      const toastC = document.getElementById('toast-container');
      const beforeCount = toastC ? toastC.children.length : 0;
      for (let i = 0; i < 5; i++) {
        if (typeof showToast === 'function') {
          showToast('记忆系统正在后台合成，请稍后重试', 'info', 5000);
        }
      }
      // 等待 100ms 后读取
      return new Promise(resolve => {
        setTimeout(() => {
          const afterCount = toastC ? toastC.children.length : 0;
          out.toastTest = {
            beforeCount,
            afterCount,
            dedupEffective: afterCount <= beforeCount + 1,
          };
          resolve(out);
        }, 100);
      });
    })()
    """
    r = cdp.evaluate(expr, await_promise=True)
    if "result" in r and "result" in r["result"]:
        result = r["result"]["result"].get("value")
        toast_test = result.get("toastTest", {})
        log(f"  去重测试: before={toast_test.get('beforeCount')}, after={toast_test.get('afterCount')}, "
            f"dedupEffective={toast_test.get('dedupEffective')}")
        return result
    return None


def collect_console_errors(cdp):
    """采集 console 错误日志（用于追溯 toast 来源）"""
    log("=== Step 7: 采集 console 日志 ===")
    # 启用 Console domain
    cdp.send("Console.enable")
    cdp.send("Runtime.enable")
    expr = r"""
    (() => {
      // 注入 console 拦截器（保留最近 50 条）
      if (!window.__lrcConsoleLog) {
        window.__lrcConsoleLog = [];
        const origLog = console.log;
        const origWarn = console.warn;
        const origError = console.error;
        const push = (level, args) => {
          try {
            const msg = args.map(a => (typeof a === 'object' ? JSON.stringify(a).substring(0, 300) : String(a))).join(' ');
            window.__lrcConsoleLog.push({ ts: Date.now(), level, msg: msg.substring(0, 500) });
            if (window.__lrcConsoleLog.length > 50) window.__lrcConsoleLog.shift();
          } catch(e) {}
        };
        console.log = function(...a) { push('log', a); origLog.apply(console, a); };
        console.warn = function(...a) { push('warn', a); origWarn.apply(console, a); };
        console.error = function(...a) { push('error', a); origError.apply(console, a); };
      }
      return { installed: true, count: window.__lrcConsoleLog.length };
    })()
    """
    r = cdp.evaluate(expr)
    log(f"  Console 拦截器: {r.get('result', {}).get('result', {}).get('value')}")
    return r


def trigger_loadDaoMetrics_and_observe(cdp):
    """触发 loadDaoMetrics 并观察 toast 是否循环"""
    log("=== Step 8: 触发 loadDaoMetrics 观察 toast 循环 ===")
    # 清空 console log
    cdp.evaluate("if (window.__lrcConsoleLog) window.__lrcConsoleLog = [];")
    # 记录 toast 初始状态
    expr_before = r"""
    (() => {
      const toastC = document.getElementById('toast-container');
      return {
        beforeToasts: toastC ? toastC.children.length : 0,
        beforeMessages: toastC ? Array.from(toastC.children).map(t => t.textContent.trim().substring(0, 100)) : [],
        beforeRetryCounters: (typeof _retryCounters !== 'undefined' && _retryCounters instanceof Map) ? Object.fromEntries(_retryCounters.entries()) : null,
        beforeDaoRetryCount: (typeof _daoRetryCount !== 'undefined') ? _daoRetryCount : null,
      };
    })()
    """
    r = cdp.evaluate(expr_before)
    before = r.get("result", {}).get("result", {}).get("value", {})
    log(f"  触发前: toasts={before.get('beforeToasts')}, daoRetry={before.get('beforeDaoRetryCount')}, "
        f"retryCounters={before.get('beforeRetryCounters')}")

    # 触发 loadDaoMetrics
    cdp.evaluate("if (typeof loadDaoMetrics === 'function') { loadDaoMetrics(); }")

    # 等待 13s（覆盖 2s/4s/8s 三次重试 + handleHttpError 2s retry）
    log("  等待 13s 观察 toast 循环...")
    time.sleep(13)

    expr_after = r"""
    (() => {
      const toastC = document.getElementById('toast-container');
      const out = {
        afterToasts: toastC ? toastC.children.length : 0,
        afterMessages: toastC ? Array.from(toastC.children).map(t => t.textContent.trim().substring(0, 100)) : [],
        afterRetryCounters: (typeof _retryCounters !== 'undefined' && _retryCounters instanceof Map) ? Object.fromEntries(_retryCounters.entries()) : null,
        afterDaoRetryCount: (typeof _daoRetryCount !== 'undefined') ? _daoRetryCount : null,
        afterDaoRetryTimer: (typeof _daoRetryTimer !== 'undefined') ? (_daoRetryTimer !== null) : null,
        consoleLog: window.__lrcConsoleLog ? window.__lrcConsoleLog.slice(-30) : [],
      };
      // 道同构度卡片状态
      const daoScore = document.getElementById('dao-ring-score');
      out.daoRingScore = daoScore ? daoScore.textContent.trim() : null;
      const daoFallback = document.querySelector('.dao-fallback-banner');
      out.daoFallbackText = daoFallback ? daoFallback.textContent.replace(/\s+/g, ' ').trim().substring(0, 200) : null;
      return out;
    })()
    """
    r = cdp.evaluate(expr_after)
    after = r.get("result", {}).get("result", {}).get("value", {})
    log(f"  触发后: toasts={after.get('afterToasts')}, daoRetry={after.get('afterDaoRetryCount')}, "
        f"retryCounters={after.get('afterRetryCounters')}")
    log(f"  道同构度: score={after.get('daoRingScore')}, fallback={after.get('daoFallbackText')}")
    log(f"  Console 日志数: {len(after.get('consoleLog', []))}")

    # 统计 503 相关 toast 出现次数
    console_log = after.get("consoleLog", [])
    lockbusy_toast_logs = [l for l in console_log if "后台合成" in l.get("msg", "")]
    log(f"  '后台合成' 相关日志数: {len(lockbusy_toast_logs)}")
    if lockbusy_toast_logs:
        log(f"  示例: {lockbusy_toast_logs[0]}")
        if len(lockbusy_toast_logs) > 1:
            log(f"  示例2: {lockbusy_toast_logs[-1]}")

    return {
        "before": before,
        "after": after,
        "lockbusy_log_count": len(lockbusy_toast_logs),
        "lockbusy_logs_sample": lockbusy_toast_logs[:5],
    }


def trigger_loadDashboard_and_observe(cdp):
    """触发 loadDashboard 观察 toast 循环"""
    log("=== Step 9: 触发 loadDashboard 观察 toast 循环 ===")
    cdp.evaluate("if (window.__lrcConsoleLog) window.__lrcConsoleLog = [];")
    expr_before = r"""
    (() => {
      const toastC = document.getElementById('toast-container');
      return {
        beforeToasts: toastC ? toastC.children.length : 0,
        beforeRetryCounters: (typeof _retryCounters !== 'undefined' && _retryCounters instanceof Map) ? Object.fromEntries(_retryCounters.entries()) : null,
        beforeDashboardRetryCount: (typeof _dashboardRetryCount !== 'undefined') ? _dashboardRetryCount : null,
      };
    })()
    """
    r = cdp.evaluate(expr_before)
    before = r.get("result", {}).get("result", {}).get("value", {})
    log(f"  触发前: toasts={before.get('beforeToasts')}, dashboardRetry={before.get('beforeDashboardRetryCount')}")

    cdp.evaluate("if (typeof loadDashboard === 'function') { loadDashboard(); }")

    log("  等待 16s 观察 toast 循环（覆盖 2s/4s/8s + handleHttpError 2s）...")
    time.sleep(16)

    expr_after = r"""
    (() => {
      const toastC = document.getElementById('toast-container');
      const out = {
        afterToasts: toastC ? toastC.children.length : 0,
        afterMessages: toastC ? Array.from(toastC.children).map(t => t.textContent.trim().substring(0, 100)) : [],
        afterRetryCounters: (typeof _retryCounters !== 'undefined' && _retryCounters instanceof Map) ? Object.fromEntries(_retryCounters.entries()) : null,
        afterDashboardRetryCount: (typeof _dashboardRetryCount !== 'undefined') ? _dashboardRetryCount : null,
        afterDashboardRetryTimer: (typeof _dashboardRetryTimer !== 'undefined') ? (_dashboardRetryTimer !== null) : null,
        consoleLog: window.__lrcConsoleLog ? window.__lrcConsoleLog.slice(-40) : [],
      };
      const dashErr = document.getElementById('dashboard-error');
      out.dashboardError = dashErr ? {
        visible: dashErr.classList.contains('show'),
        text: dashErr.textContent.replace(/\s+/g, ' ').trim().substring(0, 300),
      } : null;
      return out;
    })()
    """
    r = cdp.evaluate(expr_after)
    after = r.get("result", {}).get("result", {}).get("value", {})
    log(f"  触发后: toasts={after.get('afterToasts')}, dashboardRetry={after.get('afterDashboardRetryCount')}")
    log(f"  仪表盘错误: {after.get('dashboardError')}")

    console_log = after.get("consoleLog", [])
    lockbusy_logs = [l for l in console_log if "后台合成" in l.get("msg", "") or "LOCK_BUSY" in l.get("msg", "") or "503" in l.get("msg", "")]
    log(f"  lock_busy / 503 相关日志数: {len(lockbusy_logs)}")

    return {
        "before": before,
        "after": after,
        "lockbusy_log_count": len(lockbusy_logs),
        "lockbusy_logs_sample": lockbusy_logs[:8],
    }


def main():
    log("=== LRC Desktop v0.8.22 五层交互韧性审计 Round 3 ===")
    log(f"CDP: {CDP_HTTP}, Sidecar: {SIDECAR_HTTP}")

    ws_url, target_id, target_title = get_cdp_ws_url()
    if not ws_url:
        log("无法获取 CDP WebSocket URL，终止")
        sys.exit(1)
    log(f"CDP Target: id={target_id}, title={target_title}")
    log(f"WS URL: {ws_url}")

    evidence = {
        "audit_start": datetime.now(timezone.utc).isoformat(),
        "cdp_target_id": target_id,
        "cdp_target_title": target_title,
        "sidecar_endpoints": {},
        "frontend_state": {},
        "handleHttpError_503": {},
        "loadDaoMetrics_catch": {},
        "loadDashboard_catch": {},
        "toast_dedup_test": {},
        "loadDaoMetrics_trigger": {},
        "loadDashboard_trigger": {},
    }

    # Step 1: 探测 sidecar 端点
    evidence["sidecar_endpoints"] = probe_sidecar_endpoints()

    # Step 2-9: CDP 审计
    cdp = CDPSession(ws_url)
    try:
        # 先采集 console 日志
        collect_console_errors(cdp)
        # 等待 1s 让拦截器安装
        time.sleep(1)

        evidence["frontend_state"] = collect_frontend_state(cdp) or {}
        evidence["handleHttpError_503"] = inspect_handleHttpError_503(cdp) or {}
        evidence["loadDaoMetrics_catch"] = inspect_loadDaoMetrics_catch(cdp) or {}
        evidence["loadDashboard_catch"] = inspect_loadDashboard_catch(cdp) or {}
        evidence["toast_dedup_test"] = inspect_toast_dedup(cdp) or {}
        evidence["loadDaoMetrics_trigger"] = trigger_loadDaoMetrics_and_observe(cdp) or {}
        evidence["loadDashboard_trigger"] = trigger_loadDashboard_and_observe(cdp) or {}
    finally:
        cdp.close()

    evidence["audit_end"] = datetime.now(timezone.utc).isoformat()

    # 保存证据
    with open(EVIDENCE_PATH, "w", encoding="utf-8") as f:
        json.dump(evidence, f, ensure_ascii=False, indent=2)
    log(f"证据已保存: {EVIDENCE_PATH}")

    # 打印关键发现
    log("\n=== 关键发现摘要 ===")
    ep = evidence["sidecar_endpoints"]
    timeout_count = sum(1 for v in ep.values() if v.get("error") and "timed out" in v["error"].lower())
    log(f"  sidecar 端点超时数: {timeout_count}/4")
    fs = evidence["frontend_state"]
    log(f"  前端 monitor.lockBusy: {fs.get('monitor', {}).get('lockBusy')}")
    log(f"  前端 monitor.failCount: {fs.get('monitor', {}).get('failCount')}")
    log(f"  前端 monitor.isReachable: {fs.get('monitor', {}).get('isReachable')}")
    log(f"  前端 toasts.count: {fs.get('toasts', {}).get('count')}")
    log(f"  前端 retryCounters: {fs.get('retryCounters')}")
    dao_trig = evidence["loadDaoMetrics_trigger"]
    log(f"  loadDaoMetrics 触发后 toasts: {dao_trig.get('after', {}).get('afterToasts')}")
    log(f"  loadDaoMetrics 触发后 lockbusy 日志数: {dao_trig.get('lockbusy_log_count')}")
    dash_trig = evidence["loadDashboard_trigger"]
    log(f"  loadDashboard 触发后 toasts: {dash_trig.get('after', {}).get('afterToasts')}")
    log(f"  loadDashboard 触发后 lockbusy 日志数: {dash_trig.get('lockbusy_log_count')}")


if __name__ == "__main__":
    main()
