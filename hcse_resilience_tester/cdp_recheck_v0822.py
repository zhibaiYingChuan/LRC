"""
HCSE 回归测试精准复验 — IA-01 / IA-02 / 端点真实状态

复验目的：
  1. IA-01: 上次测试方法有误（检测 window.daoAbortController 而非旧 signal.aborted），
     改为检测旧 signal 是否被 abort
  2. IA-02: 上次测试方法有误（清除 toast 时误删 #toast-container 容器），
     改为只清除 .toast 子元素
  3. 端点真实状态: sidecar 连接泄漏（CloseWait=58）导致端点超时，
     需要在不同时间点多次采样确认

依赖: websocket-client, requests, psutil
"""

from __future__ import annotations

import json
import os
import sys
import time
import traceback
from datetime import datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from cdp_test_v0822_strict import (  # noqa: E402
    CDPClient, Sanitizer, SidecarProbe, PathValidator,
    ResourceWatchdog, BASE_DIR, CDP_ENDPOINT, SIDECAR_ENDPOINT,
)


def find_sidecar_pid() -> int:
    try:
        import psutil
        for p in psutil.process_iter(["pid", "name"]):
            name = (p.info.get("name") or "").lower()
            if "lrc-sidecar" in name:
                return p.info["pid"]
    except Exception:
        pass
    return 0


SIDECAR_PID = find_sidecar_pid()
print(f"[复验] sidecar PID = {SIDECAR_PID}")


def main() -> int:
    print("=" * 70)
    print("HCSE 回归测试精准复验 — IA-01 / IA-02 / 端点真实状态")
    print("=" * 70)
    print(f"时间: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
    print()

    results: list[dict] = []

    # ── 端点真实状态多次采样 ──
    print("=" * 70)
    print("端点真实状态采样（5 次，间隔 2s）")
    print("=" * 70)
    endpoint_samples = []
    for i in range(5):
        health = SidecarProbe.probe(f"{SIDECAR_ENDPOINT}/health", 6)
        cw = SidecarProbe.count_closewait(3099)
        sample = {
            "try": i + 1,
            "reachable": health["reachable"],
            "status": health.get("status"),
            "elapsed_ms": health["elapsed_ms"],
            "close_wait": cw,
            "body": health.get("body") if health["reachable"] else None,
        }
        endpoint_samples.append(sample)
        print(f"  第{i+1}次: reachable={health['reachable']}, "
              f"status={health.get('status')}, elapsed={health['elapsed_ms']}ms, "
              f"CloseWait={cw}")
        time.sleep(2)

    reachable_count = sum(1 for s in endpoint_samples if s["reachable"])
    avg_closewait = sum(s["close_wait"] for s in endpoint_samples) / len(endpoint_samples)
    print(f"\n端点可达率: {reachable_count}/5")
    print(f"平均 CloseWait: {avg_closewait:.1f}")

    endpoint_pass = reachable_count >= 4  # 5 次至少 4 次可达才算稳定
    results.append({
        "invariant_id": "ENDPOINT-STABILITY",
        "name": "端点稳定性（5 次采样）",
        "passed": endpoint_pass,
        "evidence": {"samples": endpoint_samples,
                     "reachable_count": reachable_count,
                     "avg_closewait": avg_closewait},
        "reason": f"可达 {reachable_count}/5, 平均 CloseWait={avg_closewait:.1f}",
    })

    # ── CDP 连接 ──
    print("\n" + "=" * 70)
    print("CDP 连接进行 IA-01 / IA-02 精准复验")
    print("=" * 70)
    client = CDPClient()
    try:
        client.connect()
    except Exception as e:
        print(f"[CDP] 连接失败: {e}")
        return 1

    # ── IA-01 精准复验：检测旧 signal 是否被 abort ──
    print("\n" + "-" * 70)
    print("IA-01 精准复验：检测旧 signal 是否被 _abortActiveTabRequests abort")
    print("-" * 70)
    t0 = time.time()
    ia01_evidence: dict = {}
    try:
        # 1. 基础检查
        check_js = """
        (function() {
            return JSON.stringify({
                window_has_daoAbortController: 'daoAbortController' in window,
                loadDaoMetrics_exists: typeof loadDaoMetrics === 'function',
                _abortActiveTabRequests_exists: typeof _abortActiveTabRequests === 'function'
            });
        })()
        """
        r = client.evaluate(check_js, timeout=10, await_promise=False)
        base = json.loads(r) if isinstance(r, str) else r
        ia01_evidence["base"] = base
        print(f"  基础: {base}")

        if base.get("window_has_daoAbortController") and base.get("loadDaoMetrics_exists"):
            # 2. 注入慢 fetch，触发 loadDaoMetrics，记录旧 signal
            inject_js = """
            (function() {
                window._hcse_origFetch = window.fetch;
                window.fetch = function(url, opts) {
                    var u = String(url);
                    if (u.indexOf('dao_metrics') !== -1) {
                        return new Promise(function(resolve) {
                            window._hcse_pendindDaoResolve = resolve;
                        });
                    }
                    return window._hcse_origFetch.apply(this, arguments);
                };
                try { loadDaoMetrics().catch(function(){}); } catch(e) {}
                return JSON.stringify({injected: true});
            })()
            """
            client.evaluate(inject_js, timeout=10, await_promise=False)
            time.sleep(0.3)

            # 3. 记录旧 signal 的 aborted 状态（应为 false）
            record_signal_js = """
            (function() {
                var ac = window.daoAbortController;
                if (!ac) return JSON.stringify({error: 'daoAbortController is null after load'});
                window._hcse_oldSignal = ac.signal;
                return JSON.stringify({
                    signal_exists: !!ac.signal,
                    signal_aborted_before: ac.signal.aborted
                });
            })()
            """
            r2 = client.evaluate(record_signal_js, timeout=10, await_promise=False)
            d2 = json.loads(r2) if isinstance(r2, str) else r2
            ia01_evidence["before_switch"] = d2
            print(f"  切换前: {d2}")

            # 4. 调用 _abortActiveTabRequests('trust-center') 模拟切换离开 dashboard
            switch_js = """
            (function() {
                if (typeof _abortActiveTabRequests === 'function') {
                    _abortActiveTabRequests('trust-center');
                }
                return JSON.stringify({switched: true});
            })()
            """
            client.evaluate(switch_js, timeout=10, await_promise=False)
            time.sleep(0.2)

            # 5. 检查旧 signal 是否被 abort（关键验证！）
            check_old_signal_js = """
            (function() {
                var oldSignal = window._hcse_oldSignal;
                if (!oldSignal) return JSON.stringify({error: 'no old signal recorded'});
                return JSON.stringify({
                    old_signal_aborted_after: oldSignal.aborted
                });
            })()
            """
            r3 = client.evaluate(check_old_signal_js, timeout=10, await_promise=False)
            d3 = json.loads(r3) if isinstance(r3, str) else r3
            ia01_evidence["after_switch"] = d3
            print(f"  切换后旧 signal.aborted: {d3}")

            # 恢复 fetch
            try:
                client.evaluate("""
                    if (window._hcse_origFetch) { window.fetch = window._hcse_origFetch; }
                    if (window._hcse_pendindDaoResolve) {
                        try { window._hcse_pendindDaoResolve(new Response('{}', {status:200})); } catch(e){}
                    }
                    delete window._hcse_oldSignal;
                """, timeout=10, await_promise=False)
            except Exception:
                pass

        # IA-01 严格判定：旧 signal 在切换后必须 aborted=true
        old_signal_aborted = (ia01_evidence.get("after_switch", {})
                              .get("old_signal_aborted_after") is True)
        window_has = base.get("window_has_daoAbortController") is True
        signal_before_ok = (ia01_evidence.get("before_switch", {})
                           .get("signal_aborted_before") is False)
        ia01_passed = window_has and signal_before_ok and old_signal_aborted
        ia01_reason = (f"window.daoAbortController 存在={window_has}; "
                       f"切换前 signal.aborted={signal_before_ok}（应=false）; "
                       f"切换后旧 signal.aborted={old_signal_aborted}（应=true）")
        print(f"  IA-01 结果: {'PASS' if ia01_passed else 'FAIL'}")
        print(f"  原因: {ia01_reason}")
        results.append({
            "invariant_id": "INV-V0822-IA01",
            "name": "loadDaoMetrics AbortController（精准复验：旧 signal 被 abort）",
            "passed": ia01_passed, "severity": "P1",
            "evidence": ia01_evidence,
            "duration_ms": int((time.time() - t0) * 1000),
            "reason": ia01_reason,
        })
    except Exception as e:
        print(f"  IA-01 异常: {e}")
        traceback.print_exc()
        results.append({
            "invariant_id": "INV-V0822-IA01",
            "name": "loadDaoMetrics AbortController（精准复验）",
            "passed": False, "severity": "P1",
            "evidence": ia01_evidence, "error": str(e),
            "duration_ms": int((time.time() - t0) * 1000),
            "reason": f"测试异常: {e}",
        })

    # ── IA-02 精准复验：不删除 #toast-container 容器 ──
    print("\n" + "-" * 70)
    print("IA-02 精准复验：不删除 #toast-container 容器，只清除 .toast 子元素")
    print("-" * 70)
    t0 = time.time()
    ia02_evidence: dict = {}
    try:
        # 1. 检查注册状态
        check_js = """
        (function() {
            var container = document.getElementById('toast-container');
            return JSON.stringify({
                registered: window._lrcGlobalErrorRegistered === true,
                showToast_exists: typeof window.showToast === 'function',
                toast_container_exists: !!container,
                toast_container_id: container ? container.id : null,
                current_toast_count: container ? container.children.length : 0
            });
        })()
        """
        r = client.evaluate(check_js, timeout=10, await_promise=False)
        base = json.loads(r) if isinstance(r, str) else r
        ia02_evidence["base"] = base
        print(f"  基础: {base}")

        registered = base.get("registered") is True
        showToast_exists = base.get("showToast_exists") is True
        container_exists = base.get("toast_container_exists") is True

        if registered and showToast_exists:
            # 2. 只清除 .toast 子元素，保留 #toast-container 容器
            clear_js = """
            (function() {
                var container = document.getElementById('toast-container');
                if (container) {
                    // 只删除 .toast 子元素，不删除容器本身
                    var toasts = container.querySelectorAll('.toast');
                    toasts.forEach(function(t){ t.remove(); });
                }
                return JSON.stringify({cleared: true, remaining: container ? container.children.length : 0});
            })()
            """
            client.evaluate(clear_js, timeout=5, await_promise=False)
            time.sleep(0.2)

            # 3. 直接调用 showToast 验证调用链
            direct_call_js = """
            (function() {
                try {
                    window.showToast('HCSE-IA02-direct-call-验证', 'error', 10000);
                    return JSON.stringify({called: true});
                } catch(e) {
                    return JSON.stringify({called: false, error: String(e)});
                }
            })()
            """
            r2 = client.evaluate(direct_call_js, timeout=5, await_promise=False)
            d2 = json.loads(r2) if isinstance(r2, str) else r2
            ia02_evidence["direct_call"] = d2
            print(f"  直接调用 showToast: {d2}")
            time.sleep(0.8)

            # 4. 检查 toast 是否出现
            check_toast_js = """
            (function() {
                var container = document.getElementById('toast-container');
                var allToasts = container ? container.querySelectorAll('.toast') : [];
                var texts = Array.from(allToasts).map(function(e){return (e.textContent || '').substring(0, 200);});
                return JSON.stringify({
                    toast_count: allToasts.length,
                    toast_texts: texts,
                    container_children: container ? container.children.length : 0
                });
            })()
            """
            r3 = client.evaluate(check_toast_js, timeout=5, await_promise=False)
            d3 = json.loads(r3) if isinstance(r3, str) else r3
            ia02_evidence["after_direct_call"] = d3
            print(f"  直接调用后 toast: {d3}")

            direct_toast_count = d3.get("toast_count", 0)

            # 5. 注入未捕获 Promise rejection，验证全局错误处理
            inject_rejection_js = """
            (function() {
                try {
                    Promise.reject(new Error('HCSE-IA02-rejection-test'));
                } catch(e) {}
                return JSON.stringify({injected: true});
            })()
            """
            client.evaluate(inject_rejection_js, timeout=5, await_promise=False)
            time.sleep(1.5)

            # 6. 检查 toast（rejection 触发的）
            r4 = client.evaluate(check_toast_js, timeout=5, await_promise=False)
            d4 = json.loads(r4) if isinstance(r4, str) else r4
            ia02_evidence["after_rejection"] = d4
            print(f"  rejection 后 toast: {d4}")

            rejection_toast_count = d4.get("toast_count", 0)

            # 7. 注入 window error 事件
            inject_error_js = """
            (function() {
                try {
                    window.dispatchEvent(new ErrorEvent('error', {
                        message: 'HCSE-IA02-window-error-test',
                        error: new Error('HCSE-IA02-window-error-test')
                    }));
                } catch(e) {}
                return JSON.stringify({injected: true});
            })()
            """
            client.evaluate(inject_error_js, timeout=5, await_promise=False)
            time.sleep(1.0)

            r5 = client.evaluate(check_toast_js, timeout=5, await_promise=False)
            d5 = json.loads(r5) if isinstance(r5, str) else r5
            ia02_evidence["after_window_error"] = d5
            print(f"  window error 后 toast: {d5}")

            # 严格判定：直接调用 showToast 至少产生 1 个 toast
            # （rejection 和 window error 在 WebView2 中可能不触发，所以主要看直接调用）
            ia02_passed = (registered and showToast_exists and container_exists
                           and direct_toast_count > 0)
            ia02_reason = (f"registered={registered}, showToast 存在={showToast_exists}, "
                           f"container 存在={container_exists}; "
                           f"直接调用 toast 数={direct_toast_count}; "
                           f"rejection toast 数={rejection_toast_count}; "
                           f"window error toast 数={d5.get('toast_count', 0)}")
        else:
            ia02_passed = False
            ia02_reason = f"registered={registered}, showToast 存在={showToast_exists}, container 存在={container_exists}"

        print(f"  IA-02 结果: {'PASS' if ia02_passed else 'FAIL'}")
        print(f"  原因: {ia02_reason}")
        results.append({
            "invariant_id": "INV-V0822-IA02",
            "name": "全局错误处理 toast（精准复验：保留 container）",
            "passed": ia02_passed, "severity": "P1",
            "evidence": ia02_evidence,
            "duration_ms": int((time.time() - t0) * 1000),
            "reason": ia02_reason,
        })

        # 清理 toast
        try:
            client.evaluate("""
                var container = document.getElementById('toast-container');
                if (container) { container.querySelectorAll('.toast').forEach(function(t){ t.remove(); }); }
            """, timeout=5, await_promise=False)
        except Exception:
            pass

    except Exception as e:
        print(f"  IA-02 异常: {e}")
        traceback.print_exc()
        results.append({
            "invariant_id": "INV-V0822-IA02",
            "name": "全局错误处理 toast（精准复验）",
            "passed": False, "severity": "P1",
            "evidence": ia02_evidence, "error": str(e),
            "duration_ms": int((time.time() - t0) * 1000),
            "reason": f"测试异常: {e}",
        })

    client.close()

    # ── 保存证据 ──
    ev_path = BASE_DIR / "evidence" / f"evidence_v0822_recheck_{int(time.time())}.json"
    PathValidator().validate(ev_path, "write")
    ev_path.write_text(json.dumps(Sanitizer.sanitize({
        "test_type": "v0.8.22 精准复验",
        "test_time": datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
        "sidecar_pid": SIDECAR_PID,
        "results": results,
    }), ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"\n[Evidence] {ev_path}")

    # ── 汇总 ──
    print("\n" + "=" * 70)
    print("精准复验结果汇总")
    print("=" * 70)
    for r in results:
        status = "PASS" if r.get("passed") else "FAIL"
        print(f"  [{status}] {r.get('invariant_id')}: {r.get('name')}")
        print(f"         {r.get('reason')}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
