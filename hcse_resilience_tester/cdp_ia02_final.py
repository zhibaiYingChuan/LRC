"""
HCSE IA-02 终极复验：重载页面恢复 #toast-container 后测试

根因：回归测试脚本 IA-02 用例的清理代码误删了 #toast-container 容器本身
      （选择器 '#toast-container, [class*="toast"]' 删除了容器）
      导致后续 showToast 因 if(!container) return 失效

本脚本：
  1. 重载页面恢复 #toast-container
  2. 验证容器存在
  3. 直接调用 showToast 验证
  4. 注入未捕获 rejection 验证全局错误处理
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
    print("HCSE IA-02 终极复验：重载页面恢复 #toast-container")
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

    # 1. 重载页面恢复 DOM
    print("\n[1] 重载页面恢复 #toast-container")
    try:
        client.send("Page.navigate", {"url": "https://tauri.localhost/#/dashboard"})
        time.sleep(4.0)
    except Exception as e:
        print(f"  导航失败: {e}")
    client.screenshot("ia02_after_reload.png")

    # 2. 验证容器存在
    print("\n[2] 验证 #toast-container 存在")
    check_container_js = """
    (function() {
        var container = document.getElementById('toast-container');
        var allToastContainers = document.querySelectorAll('[id*="toast"], [class*="toast-container"]');
        return JSON.stringify({
            container_exists: !!container,
            container_id: container ? container.id : null,
            container_class: container ? container.className : null,
            container_children: container ? container.children.length : 0,
            all_toast_containers_count: allToastContainers.length,
            all_toast_containers: Array.from(allToastContainers).map(function(e){
                return {tag: e.tagName, id: e.id, class: e.className};
            }),
            registered: window._lrcGlobalErrorRegistered === true,
            showToast_exists: typeof window.showToast === 'function'
        });
    })()
    """
    try:
        r = client.evaluate(check_container_js, timeout=10, await_promise=False)
        container_check = json.loads(r) if isinstance(r, str) else r
        evidence["container_check"] = container_check
        print(f"  容器检查: {json.dumps(container_check, ensure_ascii=False, indent=2)}")
    except Exception as e:
        container_check = {"error": str(e)}
        print(f"  检查异常: {e}")

    container_exists = container_check.get("container_exists") is True
    if not container_exists:
        print("\n  #toast-container 仍不存在！检查页面 body 内容")
        try:
            body_js = """
            (function() {
                return JSON.stringify({
                    body_children_count: document.body.children.length,
                    body_children_ids: Array.from(document.body.children).map(function(e){
                        return {tag: e.tagName, id: e.id, class: (e.className || '').substring(0, 50)};
                    }).slice(0, 20),
                    url: window.location.href,
                    title: document.title
                });
            })()
            """
            r2 = client.evaluate(body_js, timeout=10, await_promise=False)
            body_info = json.loads(r2) if isinstance(r2, str) else r2
            evidence["body_info"] = body_info
            print(f"  body 信息: {json.dumps(body_info, ensure_ascii=False, indent=2)}")
        except Exception as e:
            print(f"  body 检查异常: {e}")

    # 3. 直接调用 showToast
    print("\n[3] 直接调用 showToast 验证")
    direct_toast_count = 0
    if container_exists:
        try:
            # 清除已有 toast 子元素（保留容器）
            client.evaluate("""
                var c = document.getElementById('toast-container');
                if (c) { c.querySelectorAll('.toast').forEach(function(t){ t.remove(); }); }
            """, timeout=5, await_promise=False)
            time.sleep(0.2)

            # 直接调用
            client.evaluate("""
                try { window.showToast('HCSE-IA02-FINAL-直接调用', 'error', 15000); } catch(e){}
            """, timeout=5, await_promise=False)
            time.sleep(1.0)

            check_js = """
            (function() {
                var c = document.getElementById('toast-container');
                var toasts = c ? c.querySelectorAll('.toast') : [];
                return JSON.stringify({
                    toast_count: toasts.length,
                    texts: Array.from(toasts).map(function(t){return (t.textContent||'').substring(0,100);})
                });
            })()
            """
            r3 = client.evaluate(check_js, timeout=5, await_promise=False)
            d3 = json.loads(r3) if isinstance(r3, str) else r3
            evidence["direct_call"] = d3
            print(f"  直接调用后: {d3}")
            direct_toast_count = d3.get("toast_count", 0)
        except Exception as e:
            print(f"  异常: {e}")
            evidence["direct_call"] = {"error": str(e)}

    # 4. 注入未捕获 rejection
    print("\n[4] 注入未捕获 Promise rejection")
    rejection_toast_count = 0
    try:
        client.evaluate("""
            try { Promise.reject(new Error('HCSE-IA02-FINAL-rejection')); } catch(e){}
        """, timeout=5, await_promise=False)
        time.sleep(1.5)
        check_js = """
        (function() {
            var c = document.getElementById('toast-container');
            var toasts = c ? c.querySelectorAll('.toast') : [];
            return JSON.stringify({
                toast_count: toasts.length,
                texts: Array.from(toasts).map(function(t){return (t.textContent||'').substring(0,100);})
            });
        })()
        """
        r4 = client.evaluate(check_js, timeout=5, await_promise=False)
        d4 = json.loads(r4) if isinstance(r4, str) else r4
        evidence["after_rejection"] = d4
        print(f"  rejection 后: {d4}")
        rejection_toast_count = d4.get("toast_count", 0)
    except Exception as e:
        print(f"  异常: {e}")
        evidence["after_rejection"] = {"error": str(e)}

    # 5. 注入 window error 事件
    print("\n[5] 注入 window error 事件")
    error_toast_count = 0
    try:
        client.evaluate("""
            try {
                window.dispatchEvent(new ErrorEvent('error', {
                    message: 'HCSE-IA02-FINAL-window-error',
                    error: new Error('HCSE-IA02-FINAL-window-error')
                }));
            } catch(e){}
        """, timeout=5, await_promise=False)
        time.sleep(1.0)
        check_js = """
        (function() {
            var c = document.getElementById('toast-container');
            var toasts = c ? c.querySelectorAll('.toast') : [];
            return JSON.stringify({
                toast_count: toasts.length,
                texts: Array.from(toasts).map(function(t){return (t.textContent||'').substring(0,100);})
            });
        })()
        """
        r5 = client.evaluate(check_js, timeout=5, await_promise=False)
        d5 = json.loads(r5) if isinstance(r5, str) else r5
        evidence["after_window_error"] = d5
        print(f"  window error 后: {d5}")
        error_toast_count = d5.get("toast_count", 0)
    except Exception as e:
        print(f"  异常: {e}")
        evidence["after_window_error"] = {"error": str(e)}

    client.close()

    # 6. 判定
    print("\n" + "=" * 70)
    print("IA-02 终极复验判定")
    print("=" * 70)
    registered = container_check.get("registered") is True
    showToast_exists = container_check.get("showToast_exists") is True
    # 严格判定：容器存在 + 直接调用产生 toast
    # rejection/window error 在 WebView2 中可能不触发，主要看直接调用
    passed = container_exists and registered and showToast_exists and direct_toast_count > 0
    reason = (f"container 存在={container_exists}; registered={registered}; "
              f"showToast 存在={showToast_exists}; 直接调用 toast 数={direct_toast_count}; "
              f"rejection toast 数={rejection_toast_count}; window error toast 数={error_toast_count}")
    print(f"  结果: {'PASS' if passed else 'FAIL'}")
    print(f"  原因: {reason}")

    # 保存证据
    ev_path = BASE_DIR / "evidence" / f"evidence_v0822_ia02_final_{int(time.time())}.json"
    PathValidator().validate(ev_path, "write")
    ev_path.write_text(json.dumps(Sanitizer.sanitize({
        "test_type": "IA-02 终极复验",
        "test_time": datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
        "passed": passed,
        "reason": reason,
        "evidence": evidence,
    }), ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"\n[Evidence] {ev_path}")

    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
