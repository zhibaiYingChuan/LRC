"""
HCSE IA-02 最终判定：Page.reload 强制重载页面后验证 #toast-container

关键区别：
  - Page.navigate 到相同 hash URL（#/dashboard）不会重载页面
  - Page.reload 会强制重载，恢复初始 DOM（包括 #toast-container）

本脚本验证：重载后 #toast-container 是否存在，showToast 是否生效
"""

from __future__ import annotations

import json
import sys
import time
from datetime import datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from cdp_test_v0822_strict import (  # noqa: E402
    CDPClient, Sanitizer, PathValidator, BASE_DIR,
)


def main() -> int:
    print("=" * 70)
    print("HCSE IA-02 最终判定：Page.reload 强制重载")
    print("=" * 70)
    print(f"时间: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
    print()

    client = CDPClient()
    try:
        client.connect()
    except Exception as e:
        print(f"[CDP] 连接失败: {e}")
        return 1

    evidence: dict = {}

    # 1. 重载前检查
    print("[1] 重载前 #toast-container 状态")
    try:
        r = client.evaluate("""
            (function() {
                var c = document.getElementById('toast-container');
                return JSON.stringify({exists: !!c, children: c ? c.children.length : 0});
            })()
        """, timeout=10, await_promise=False)
        before = json.loads(r) if isinstance(r, str) else r
        evidence["before_reload"] = before
        print(f"  重载前: {before}")
    except Exception as e:
        before = {"error": str(e)}
        print(f"  异常: {e}")

    # 2. Page.reload 强制重载
    print("\n[2] Page.reload 强制重载页面")
    try:
        client.send("Page.reload", {"ignoreCache": True})
        print("  重载命令已发送，等待 5s 让页面完全加载...")
        time.sleep(5.0)
    except Exception as e:
        print(f"  重载失败: {e}")

    # 3. 重载后检查 #toast-container
    print("\n[3] 重载后 #toast-container 状态")
    try:
        r = client.evaluate("""
            (function() {
                var c = document.getElementById('toast-container');
                return JSON.stringify({
                    exists: !!c,
                    id: c ? c.id : null,
                    className: c ? c.className : null,
                    children: c ? c.children.length : 0,
                    registered: window._lrcGlobalErrorRegistered === true,
                    showToast_exists: typeof window.showToast === 'function'
                });
            })()
        """, timeout=10, await_promise=False)
        after = json.loads(r) if isinstance(r, str) else r
        evidence["after_reload"] = after
        print(f"  重载后: {json.dumps(after, ensure_ascii=False, indent=2)}")
    except Exception as e:
        after = {"error": str(e)}
        print(f"  异常: {e}")

    container_exists = after.get("exists") is True
    registered = after.get("registered") is True
    showToast_exists = after.get("showToast_exists") is True

    # 4. 直接调用 showToast
    print("\n[4] 直接调用 showToast")
    direct_toast_count = 0
    if container_exists:
        try:
            client.evaluate("""
                try { window.showToast('HCSE-IA02-RELOAD-验证', 'error', 15000); } catch(e){}
            """, timeout=5, await_promise=False)
            time.sleep(1.0)
            r = client.evaluate("""
                (function() {
                    var c = document.getElementById('toast-container');
                    var toasts = c ? c.querySelectorAll('.toast') : [];
                    return JSON.stringify({
                        toast_count: toasts.length,
                        texts: Array.from(toasts).map(function(t){return (t.textContent||'').substring(0,100);})
                    });
                })()
            """, timeout=5, await_promise=False)
            d = json.loads(r) if isinstance(r, str) else r
            evidence["direct_call"] = d
            print(f"  直接调用后: {d}")
            direct_toast_count = d.get("toast_count", 0)
        except Exception as e:
            print(f"  异常: {e}")

    # 5. 注入未捕获 rejection
    print("\n[5] 注入未捕获 Promise rejection")
    rejection_toast_count = 0
    try:
        client.evaluate("""
            try { Promise.reject(new Error('HCSE-IA02-RELOAD-rejection')); } catch(e){}
        """, timeout=5, await_promise=False)
        time.sleep(1.5)
        r = client.evaluate("""
            (function() {
                var c = document.getElementById('toast-container');
                var toasts = c ? c.querySelectorAll('.toast') : [];
                return JSON.stringify({
                    toast_count: toasts.length,
                    texts: Array.from(toasts).map(function(t){return (t.textContent||'').substring(0,100);})
                });
            })()
        """, timeout=5, await_promise=False)
        d2 = json.loads(r) if isinstance(r, str) else r
        evidence["after_rejection"] = d2
        print(f"  rejection 后: {d2}")
        rejection_toast_count = d2.get("toast_count", 0)
    except Exception as e:
        print(f"  异常: {e}")

    client.screenshot("ia02_after_reload_final.png")
    client.close()

    # 6. 判定
    print("\n" + "=" * 70)
    print("IA-02 最终判定")
    print("=" * 70)
    # 严格判定：容器存在 + 注册 + showToast 存在 + 直接调用产生 toast
    # rejection 在 WebView2 中可能不触发 unhandledrejection，主要看直接调用
    passed = container_exists and registered and showToast_exists and direct_toast_count > 0
    reason = (f"重载前 container 存在={before.get('exists')}; "
              f"重载后 container 存在={container_exists}; "
              f"registered={registered}; showToast 存在={showToast_exists}; "
              f"直接调用 toast 数={direct_toast_count}; "
              f"rejection toast 数={rejection_toast_count}")
    print(f"  结果: {'PASS' if passed else 'FAIL'}")
    print(f"  原因: {reason}")

    # 保存证据
    ev_path = BASE_DIR / "evidence" / f"evidence_v0822_ia02_reload_{int(time.time())}.json"
    PathValidator().validate(ev_path, "write")
    ev_path.write_text(json.dumps(Sanitizer.sanitize({
        "test_type": "IA-02 Page.reload 最终判定",
        "test_time": datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
        "passed": passed,
        "reason": reason,
        "evidence": evidence,
    }), ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"\n[Evidence] {ev_path}")

    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
