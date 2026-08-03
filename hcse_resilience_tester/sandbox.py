#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
HCSE Phase 6 — 可信沙箱模块（SecureSandbox）
=================================================

职责：
  1. 路径白名单 PathValidator（从 rv_monitor 复用或重导出）
  2. 双重数据脱敏 DataSanitizer（正则 + 结构体字段裁剪）
  3. 资源容量看门狗 ResourceWatchdog（MAX_MEM / MAX_CPU）
  4. 对外聚合类 SecureSandbox：组合以上三者，提供统一 API

本模块被 test_orchestrator.py 与 evidence_builder.py 通过以下形式引用：
    from .sandbox import SecureSandbox, DataSanitizer

为了与 rv_monitor.py 保持单一实现，本模块从 rv_monitor 导入已实现的
三个基础组件并在 SecureSandbox 中包装。若导入失败（作为独立脚本运行时），
则回退为同构内联实现（避免循环依赖导致脚本无法执行）。
"""

from __future__ import annotations

import json
import os
import re
import sys
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable, Optional

# ------------- 优先从 rv_monitor 导入，保持单一实现 ----------------
try:
    from .rv_monitor import (
        PathValidator,
        DataSanitizer,
        ResourceWatchdog,
        SandboxViolation,
    )
except (ImportError, ValueError):  # 直接运行：from sandbox import ...
    try:
        from rv_monitor import (  # type: ignore
            PathValidator,
            DataSanitizer,
            ResourceWatchdog,
            SandboxViolation,
        )
    except ImportError:
        # 最终兜底内联实现（与 rv_monitor.py 完全同构，保持行为等价）
        class SandboxViolation(RuntimeError):  # type: ignore[no-redef]
            pass

        class PathValidator:  # type: ignore[no-redef]
            """路径白名单：仅允许 ./temp ./logs ./screenshots ./evidence 前缀。"""

            def __init__(self, roots: Optional[Iterable[str]] = None) -> None:
                base = Path.cwd().resolve()
                if roots is None:
                    roots = ["./temp", "./logs", "./screenshots", "./evidence"]
                self._allowed = sorted({(base / Path(p)).resolve() for p in roots})
                self.workspace_root = base

            @staticmethod
            def _normalize(path: str | os.PathLike[str]) -> Path:
                p = Path(path)
                if not p.is_absolute():
                    p = Path.cwd() / p
                try:
                    return p.resolve()
                except OSError:
                    return p.absolute()

            def validate(self, path: str | os.PathLike[str], *, op: str = "read") -> Path:
                target = self._normalize(path)
                ok = any(
                    target == root or root in target.parents
                    for root in self._allowed
                )
                if not ok:
                    raise SandboxViolation(
                        f"[SANDBOX-HALT:{op}] 越权访问: {target} "
                        f"(allowed={[str(r) for r in self._allowed]})"
                    )
                target.parent.mkdir(parents=True, exist_ok=True)
                return target

        class DataSanitizer:  # type: ignore[no-redef]
            """Phase 6 双次脱敏。"""

            REDACT = "[REDACTED]"
            AUTH_RE = re.compile(
                r"(authorization\s*[:=]\s*)([\"']?)([^\r\n\"',;&]*[^\s\r\n\"',;&])?",
                re.I,
            )
            AUTH_SUB = r"\1\2[BEARER_TOKEN_REDACTED]"
            AUTH_BARE_RE = re.compile(
                r"(?<![A-Za-z0-9_-])(bearer|basic|token)\s+[A-Za-z0-9\-._~+/]+=*",
                re.I,
            )
            AUTH_BARE_SUB = r"\1 [BEARER_TOKEN_REDACTED]"
            VALUE_HINT_RE = re.compile(
                r'(?P<name>"value"|value)\s*[:=]\s*(?P<q>["\']?)(?P<val>[^"\',;\s&]{2,})(?P=q)',
                re.I,
            )
            VALUE_HINT_SUB = r'\g<name>: \g<q>[REDACTED]\g<q>'
            EMAIL_RE = re.compile(r"[\w.+-]+@[\w-]+\.[\w.-]+")
            PHONE_RE = re.compile(r"(?<!\d)(?:\+?86[\s-]?)?1[3-9]\d{9}(?!\d)")
            FIELD_BLACKLIST = {
                "email", "phone", "api_key", "encrypted_api_key", "value",
                "authorization", "auth", "token", "secret",
            }

            def __init__(self, extra_fields: Iterable[str] = ()) -> None:
                self.FIELD_BLACKLIST = set(self.FIELD_BLACKLIST) | set(extra_fields)

            def sanitize_text(self, raw: str) -> str:
                if not isinstance(raw, str):
                    return raw
                s = self.AUTH_RE.sub(self.AUTH_SUB, raw)
                s = self.AUTH_BARE_RE.sub(self.AUTH_BARE_SUB, s)
                s = self.VALUE_HINT_RE.sub(self.VALUE_HINT_SUB, s)
                s = self.EMAIL_RE.sub("<email@redacted>", s)
                s = self.PHONE_RE.sub("<phone-redacted>", s)
                # 显式 cookie 中的 value 关键字兜底
                s = re.sub(
                    r'(?P<prefix>["\s,;]value["\s:=]+["\']?)[^"\',;\s&]{2,}(?P<suffix>["\',;\s&]|$)',
                    lambda m: f"{m.group('prefix')}[REDACTED]{m.group('suffix')}",
                    s,
                )
                return s

            def sanitize_struct(self, obj: Any) -> Any:
                if isinstance(obj, dict):
                    out: dict[Any, Any] = {}
                    for k, v in obj.items():
                        if isinstance(k, str) and k.lower() in self.FIELD_BLACKLIST:
                            out[k] = self.REDACT
                            continue
                        out[k] = self.sanitize_struct(v)
                    return out
                if isinstance(obj, list):
                    return [self.sanitize_struct(v) for v in obj]
                if isinstance(obj, tuple):
                    return tuple(self.sanitize_struct(v) for v in obj)
                if isinstance(obj, str):
                    return self.sanitize_text(obj)
                return obj

            # ---- 便捷写入方法（供 orchestrator / evidence builder 使用） ----
            def write_sanitized_json(self, path: str | os.PathLike[str], data: Any,
                                     *, validator: Optional[PathValidator] = None) -> Path:
                target = Path(path)
                if validator:
                    target = validator.validate(target, op="write json")
                clean = self.sanitize_struct(data)
                target.write_text(
                    json.dumps(clean, ensure_ascii=False, indent=2, default=str),
                    encoding="utf-8",
                )
                return target

        class ResourceWatchdog:  # type: ignore[no-redef]
            """psutil 资源看门狗：超过阈值优先杀子 CDP 会话。"""

            def __init__(self, max_mem_mb: int = 1024, max_cpu_s: int = 60,
                         child_pid: Optional[int] = None) -> None:
                self.max_mem = max_mem_mb * 1024 * 1024
                self.max_cpu = max_cpu_s
                self._child_pid = child_pid
                try:
                    import psutil  # type: ignore
                    self._psutil = psutil
                    self._self = psutil.Process()
                except ImportError:
                    self._psutil = None  # type: ignore
                    self._self = None
                self._start_cpu = self._cpu_now()
                self._tripped = False

            def _cpu_now(self) -> float:
                if not self._self:
                    return 0.0
                try:
                    c = self._self.cpu_times()
                    return c.user + c.system
                except Exception:
                    return 0.0

            def set_child_pid(self, pid: Optional[int]) -> None:
                self._child_pid = pid

            def tick(self) -> tuple[bool, str]:
                if self._tripped or not self._psutil:
                    return False, ""
                try:
                    rss = self._self.memory_info().rss
                    cpu = self._cpu_now() - self._start_cpu
                except Exception:
                    return False, ""
                reason: Optional[str] = None
                if rss > self.max_mem:
                    reason = f"MEMORY LIMIT rss={rss // 1048576}MB > {self.max_mem // 1048576}MB"
                elif cpu > self.max_cpu:
                    reason = f"CPU TIME cpu={cpu:.1f}s > {self.max_cpu}s"
                if reason:
                    self._tripped = True
                    self._kill_child()
                    return True, reason
                return False, ""

            def _kill_child(self) -> None:
                if not self._child_pid or not self._psutil:
                    return
                try:
                    p = self._psutil.Process(self._child_pid)
                    for c in p.children(recursive=True):
                        try:
                            c.terminate()
                        except Exception:
                            pass
                    try:
                        p.terminate()
                    except Exception:
                        pass
                    gone, alive = self._psutil.wait_procs(
                        [p] + p.children(recursive=True), timeout=5)
                    for a in alive:
                        try:
                            a.kill()
                        except Exception:
                            pass
                except Exception:
                    pass


# =============================================================
# SecureSandbox：统一聚合 API（供 orchestrator / evidence 调用）
# =============================================================
@dataclass
class SandboxStatus:
    path_ok: bool = True
    sanitize_ok: bool = True
    watchdog_ok: bool = True
    last_error: str = ""


class SecureSandbox:
    """
    Phase 6 可信沙箱（TEE 级别）：对外暴露三组件 + 聚合自检。

    用法：
        sb = SecureSandbox()
        # 写文件（自动路径校验 + 脱敏）
        sb.write_json("./evidence/out.json", {"data": "xx"})
        # 每次 tick 调用 watchdog，超限自动杀子
        ok, reason = sb.watchdog.tick()
    """

    DEFAULT_ROOTS = ("./temp", "./logs", "./screenshots", "./evidence")

    def __init__(self, *, roots: Iterable[str] = DEFAULT_ROOTS,
                 max_mem_mb: int = 1024, max_cpu_s: int = 60,
                 child_pid: Optional[int] = None,
                 extra_sanitize_fields: Iterable[str] = ()) -> None:
        self.path_validator = PathValidator(roots)
        self.data_sanitizer = DataSanitizer(extra_sanitize_fields)
        self.watchdog = ResourceWatchdog(
            max_mem_mb=max_mem_mb, max_cpu_s=max_cpu_s, child_pid=child_pid
        )
        self.status = SandboxStatus()
        self._started_at = datetime.now(timezone.utc).isoformat(timespec="milliseconds")

    # ------ 路径白名单（Hard Halt 语义） ------
    def assert_safe_path(self, p: str | os.PathLike[str], *, op: str = "access") -> Path:
        try:
            safe = self.path_validator.validate(p, op=op)
            self.status.path_ok = True
            return safe
        except SandboxViolation as e:
            self.status.path_ok = False
            self.status.last_error = str(e)
            # Phase 6 要求：任何越权访问 → Hard Halt
            halt_root = Path.cwd() / "evidence"
            try:
                halt_root.mkdir(parents=True, exist_ok=True)
                (halt_root / "SANDBOX_HALT.txt").write_text(
                    f"{datetime.now(timezone.utc).isoformat()}\n{self.status.last_error}\n",
                    encoding="utf-8",
                )
            except Exception:
                pass
            print(f"[SANDBOX-HARD-HALT] {self.status.last_error}", file=sys.stderr)
            # 非零退出码：130 = HCSE 环境违规（保留 SIGINT=2 给用户 Ctrl-C）
            try:
                sys.exit(130)
            except SystemExit:
                raise
        raise RuntimeError("unreachable")  # pragma: no cover

    # ------ 脱敏写入 ------
    def write_json(self, path: str | os.PathLike[str], payload: Any) -> Path:
        safe = self.assert_safe_path(path, op="write_json")
        return self.data_sanitizer.write_sanitized_json(safe, payload)

    def write_text(self, path: str | os.PathLike[str], content: str,
                   *, sanitize: bool = True) -> Path:
        safe = self.assert_safe_path(path, op="write_text")
        text = self.data_sanitizer.sanitize_text(content) if sanitize else content
        safe.write_text(text, encoding="utf-8")
        return safe

    # ------ 资源看门狗 ------
    def resource_tick(self) -> tuple[bool, str]:
        tripped, reason = self.watchdog.tick()
        if tripped:
            self.status.watchdog_ok = False
            self.status.last_error = reason
        return tripped, reason

    # ------ 自检（用于 _selftest / CI 预飞） ------
    def self_test(self, *, write_samples: bool = True) -> tuple[bool, str]:
        """返回 (是否通过, 摘要)。"""
        failures: list[str] = []

        # (1) 路径黑白样本
        try:
            safe = self.path_validator.validate("./evidence/sbx-self-test.txt")
            if write_samples:
                safe.parent.mkdir(parents=True, exist_ok=True)
                safe.write_text(f"sbx self test {time.time()}", encoding="utf-8")
        except SandboxViolation:
            failures.append("PathValidator 拒绝合法路径 ./evidence/")
        try:
            self.path_validator.validate("C:/Windows/System32/sethc.exe", op="read windows")
        except SandboxViolation:
            pass  # 预期拦截
        else:
            failures.append("PathValidator 未拦截系统路径 C:/Windows/System32/...")

        # (2) 双重脱敏
        raw = {
            "request": {
                "headers": [
                    {"name": "authorization", "value": "Bearer eyJhbGciOi.test"},
                    {"name": "cookie", "value": "sessionId=abc123"},
                ],
                "body": {"email": "admin@example.com", "phone": "13800138000",
                         "extra": {"api_key": "sk-1234"}},
            },
            "response": {"cookies": [{"name": "sid", "value": "LEAK-VALUE"}]},
        }
        clean = self.data_sanitizer.sanitize_struct(raw)
        # 结构体级
        j = json.dumps(clean, ensure_ascii=False)
        for pii in ("admin@example.com", "13800138000", "sk-1234",
                    "eyJhbGciOi", "sessionId=abc123", "LEAK-VALUE"):
            if pii in j:
                failures.append(f"DataSanitizer 泄漏字段片段: {pii}")
        # 文本级
        text = (
            "Authorization: Bearer xyz-token, "
            "email: user@corp.com, phone: 13999999999"
        )
        s_text = self.data_sanitizer.sanitize_text(text)
        for pii in ("xyz-token", "user@corp.com", "13999999999"):
            if pii in s_text:
                failures.append(f"DataSanitizer.sanitize_text 泄漏: {pii}")

        # (3) 资源看门狗（只验证接口可调用，不真实消耗 1024MB）
        try:
            self.watchdog.tick()
        except Exception as e:  # pragma: no cover
            failures.append(f"ResourceWatchdog 异常: {e!r}")

        ok = not failures
        summary = (
            "SecureSandbox 自检通过"
            if ok
            else f"SecureSandbox 自检失败 {len(failures)} 项: " + "; ".join(failures)
        )
        self.status.sanitize_ok = "DataSanitizer" not in summary
        self.status.last_error = "" if ok else summary
        return ok, summary


# =============================================================
# 独立运行：打印自检摘要
# =============================================================
def _main() -> int:
    sb = SecureSandbox()
    ok, summary = sb.self_test()
    print(f"[sandbox] {'PASS' if ok else 'FAIL'} - {summary}")
    # 额外验证 DataSanitizer 默认构造
    ds = DataSanitizer()
    s = ds.sanitize_text("email: a@b.com, auth=Bearer XXX")
    assert "a@b.com" not in s, f"email leak: {s}"
    assert "XXX" not in s, f"auth leak: {s}"
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(_main())
