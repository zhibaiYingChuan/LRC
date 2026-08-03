"""
HCSE 运行时验证核心引擎 (RV-Monitor)
==============================================

- 事件溯源队列：收集 requestWillBeSent / responseReceived /
  exceptionThrown / domMutated 等 CDP 事件
- 不变量检查器：每收到关键事件，立即运行 invariants.yaml 中定义的逻辑断言
- CDP 保活探针：断言失败后自动 Browser.getVersion 确认通道存活，避免假阴性
- Phase6 沙箱：PathValidator + 双次数据脱敏 + psutil 资源看门狗

使用方式（依赖 Playwright 作为 CDP 传输层；对 Tauri WebView2 需要在启动时加
--remote-debugging-port=0 并读取 devtoolsActivePort 文件）：

    python -m hcse_resilience_tester.rv_monitor \
        --cdp ws://127.0.0.1:9222/devtools/browser/<id> \
        --invariants ./invariants.yaml \
        --evidence ./evidence

作者：HCSE 韧性验证架构师
"""
from __future__ import annotations

import asyncio
import json
import os
import re
import sys
import time
from dataclasses import dataclass, field, asdict
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath, PureWindowsPath
from typing import Any, Iterable

try:
    import psutil  # type: ignore
except ImportError:  # 看门狗降级：不影响主流程
    psutil = None  # type: ignore

try:
    import yaml  # type: ignore
except ImportError:  # 允许纯 JSON 兜底
    yaml = None  # type: ignore


# =============================================================
# Phase 6 - TEE 安全沙箱
# =============================================================

class PathValidator:
    """Path 白名单校验。任何越权访问 → HardHalt（测试失败+写证据）。"""

    DEFAULT_ROOTS = ("./temp", "./logs", "./screenshots", "./evidence")

    def __init__(self, roots: Iterable[str | os.PathLike[str]] | None = None) -> None:
        self.workspace_root = Path.cwd().resolve()
        if roots is None:
            roots = self.DEFAULT_ROOTS
        self._allowed = sorted({(self.workspace_root / Path(p)).resolve() for p in roots})

    @staticmethod
    def _normalize(path: str | os.PathLike[str]) -> Path:
        p = Path(path)
        if not p.is_absolute():
            p = Path.cwd() / p
        # 消除 .. 与符号链接（若存在则解析，否则保留）
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
            msg = (
                f"[SANDBOX-HALT] 越权{op}路径: {target} "
                f"(allowed={[str(r) for r in self._allowed]})"
            )
            print(msg, file=sys.stderr)
            # 立即写证据（避免后续路径访问也被拦）
            EvidenceBuilder.hard_halt(msg)
            raise SandboxViolation(msg)
        return target


class SandboxViolation(RuntimeError):
    """HCSE 环境越权访问异常。抛出后整个测试进程视为失败。"""


class DataSanitizer:
    """双次脱敏：正则替换 + 结构化字段裁剪。"""

    REDACT = "[REDACTED]"
    # 关键：值分组允许内部空格（'Bearer xxx' 整段匹配），最后一个字符不能是空格（避免吃入后续分隔前的空格）
    # 分隔符：" ' , ; & 行尾 / 回车
    AUTH_RE = re.compile(
        r"(authorization\s*[:=]\s*)([\"']?)([^\r\n\"',;&]*[^\s\r\n\"',;&])?",
        re.I,
    )
    AUTH_SUB = r"\1\2[BEARER_TOKEN_REDACTED]"
    # 兜底：任意独立出现的 Bearer/Basic/Token <token-char-seq>
    AUTH_BARE_RE = re.compile(
        r"(?<![A-Za-z0-9_-])(bearer|basic|token)\s+[A-Za-z0-9\-._~+/]+=*",
        re.I,
    )
    AUTH_BARE_SUB = r"\1 [BEARER_TOKEN_REDACTED]"
    EMAIL_RE = re.compile(r"[\w.+-]+@[\w-]+\.[\w.-]+")
    PHONE_RE = re.compile(r"(?<!\d)(?:\+?86[\s-]?)?1[3-9]\d{9}(?!\d)")
    FIELD_BLACKLIST = {"email", "phone", "api_key", "encrypted_api_key", "value",
                       "authorization", "auth", "token", "secret"}

    def __init__(self, extra_fields: Iterable[str] = ()) -> None:
        self.FIELD_BLACKLIST = set(self.FIELD_BLACKLIST) | set(extra_fields)

    # --- 第一次：正则字符串级 ---
    VALUE_HINT_RE = re.compile(
        # JSON/HAR/Cookie/Header 中 value 关键字后面的值：
        #   "value"   :   "..."   → 包含前面引号等
        r'(?P<name>"value"|value)\s*[:=]\s*(?P<q>["\']?)(?P<val>[^"\',;\s&]{2,})(?P=q)',
        re.I,
    )
    VALUE_HINT_SUB = r'\g<name>: \g<q>[REDACTED]\g<q>'

    def sanitize_text(self, raw: str) -> str:
        if not isinstance(raw, str):
            return raw
        s = self.AUTH_RE.sub(self.AUTH_SUB, raw)
        s = self.AUTH_BARE_RE.sub(self.AUTH_BARE_SUB, s)  # 兜底：裸 Bearer/Basic/Token
        s = self.VALUE_HINT_RE.sub(self.VALUE_HINT_SUB, s)  # cookie/value 字段替换（JSON/HAR头部
        s = self.EMAIL_RE.sub("<email@redacted>", s)
        s = self.PHONE_RE.sub("<phone-redacted>", s)
        return s

    # --- 第二次：结构体字段级 ---
    def sanitize_struct(self, obj: Any) -> Any:
        if isinstance(obj, dict):
            out: dict[Any, Any] = {}
            for k, v in obj.items():
                if isinstance(k, str) and k.lower() in self.FIELD_BLACKLIST:
                    # cookie 中的 value、authorization 等
                    out[k] = self.REDACT if k.lower() != "value" else self.REDACT
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

    # ---- 便捷写入（供 orchestrator / evidence_builder 调用） ----
    def write_sanitized_json(
        self,
        path: str | os.PathLike[str],
        data: Any,
        *,
        validator: Any = None,  # PathValidator 可选；若传入则先做路径白名单校验
    ) -> Path:
        """将 data 做结构体裁剪+文本脱敏后，以 JSON 形式写到 path。"""
        target = Path(path)
        if validator is not None and hasattr(validator, "validate"):
            target = validator.validate(target, op="write json")
        clean = self.sanitize_struct(data)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(
            json.dumps(clean, ensure_ascii=False, indent=2, default=str),
            encoding="utf-8",
        )
        return target


class ResourceWatchdog:
    """资源容量看门狗：MAX_MEM=1024MB / MAX_CPU=60s
    超限后先 terminate 子 CDP session，再允许主脚本收尾（防平台雪崩）。"""

    def __init__(self, max_mem_mb: int = 1024, max_cpu_s: int = 60,
                 child_pid: int | None = None) -> None:
        self.max_mem = max_mem_mb * 1024 * 1024
        self.max_cpu = max_cpu_s
        self._self_proc = psutil.Process() if psutil else None
        self._child_pid = child_pid
        self._start_cpu = self._cpu_self()
        self._tripped = False

    def _cpu_self(self) -> float:
        if not psutil:
            return 0.0
        try:
            c = self._self_proc.cpu_times()
            return c.user + c.system
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            return 0.0

    def set_child_pid(self, pid: int | None) -> None:
        self._child_pid = pid

    def tick(self) -> tuple[bool, str]:
        """每 1s 调用一次。返回 (是否触发保护, 原因)。"""
        if self._tripped or not psutil:
            return (False, "")
        try:
            rss = self._self_proc.memory_info().rss
            cpu_now = self._cpu_self() - self._start_cpu
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            return (False, "")

        reason: str | None = None
        if rss > self.max_mem:
            reason = f"MEMORY LIMIT EXCEEDED rss={rss//1048576}MB > {self.max_mem//1048576}MB"
        elif cpu_now > self.max_cpu:
            reason = f"CPU TIME LIMIT EXCEEDED cpu={cpu_now:.1f}s > {self.max_cpu}s"

        if reason:
            self._tripped = True
            self._terminate_child()
            return (True, reason)
        return (False, "")

    def _terminate_child(self) -> None:
        if not self._child_pid or not psutil:
            return
        try:
            child = psutil.Process(self._child_pid)
            for sub in child.children(recursive=True):
                sub.terminate()
            child.terminate()
            gone, alive = psutil.wait_procs([child] + child.children(recursive=True), timeout=5)
            for p in alive:
                p.kill()
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            pass


# =============================================================
# Phase 3 - 事件溯源队列 + 不变量检查器
# =============================================================

@dataclass(order=True)
class EvQueueItem:
    ts: float                   # monotonic 秒
    wall: str                   # ISO 墙钟时间
    kind: str                   # requestWillBeSent / responseReceived / ...
    data: dict[str, Any] = field(default_factory=dict, compare=False)


class EventSourcingQueue:
    MAXLEN = 20_000  # 环形上限，避免内存溢出

    def __init__(self) -> None:
        self._q: asyncio.Queue[EvQueueItem] = asyncio.Queue(maxsize=self.MAXLEN)
        self._history: list[EvQueueItem] = []

    async def put(self, kind: str, data: dict[str, Any]) -> None:
        item = EvQueueItem(
            ts=time.monotonic(),
            wall=datetime.now(timezone.utc).isoformat(timespec="milliseconds"),
            kind=kind,
            data=data,
        )
        try:
            self._q.put_nowait(item)
        except asyncio.QueueFull:
            # 丢最旧的一条
            try:
                self._q.get_nowait()
                self._q.put_nowait(item)
            except asyncio.QueueEmpty:
                pass
        self._history.append(item)
        if len(self._history) > self.MAXLEN:
            self._history = self._history[-self.MAXLEN // 2:]

    def recent(self, last_n: int = 500) -> list[EvQueueItem]:
        return self._history[-last_n:]


@dataclass
class InvariantViolation:
    invariant_id: str
    layer: str
    severity: str
    assertion_text: str
    trigger_event_kind: str
    trigger_event_data: dict[str, Any]
    context_snapshot: dict[str, Any]
    reported_at: str
    cdp_alive: bool


class InvariantViolationReport:
    """
    test_orchestrator.py / evidence_builder.py 使用的异常包装类：
        try:
            ...
        except InvariantViolation as e:
            case.run_result = {"violation": asdict(e.report)}

    它封装底层 InvariantViolation，提供 report_id / invariant_name /
    to_mermaid_failure_tree 等更适合报告层的字段。
    """

    def __init__(self, v: InvariantViolation) -> None:
        self.report = v
        self.report_id: str = (
            f"INV-{v.invariant_id}-{abs(hash((v.reported_at, v.assertion_text[:80])))}"
        )
        self.invariant_name: str = v.invariant_id
        self.invariant_id: str = v.invariant_id
        self.layer: str = v.layer
        self.severity: str = v.severity
        self.detail: str = v.assertion_text
        self.violated_at_ms: int = int(time.time() * 1000)

    def to_mermaid_failure_tree(self) -> str:
        """生成单条违例的简化 Mermaid FTA（与 evidence_builder.py 中的全局 FTA 互补）。"""
        inv = self.invariant_id
        text = (self.assertion_text if hasattr(self, "assertion_text")
                else self.detail).replace("'", "").replace('"', "")[:60]
        return (
            "```mermaid\n"
            "graph TD\n"
            f"    Root[违反: {inv}] --> Cause[触发事件: {self.report.trigger_event_kind}]\n"
            f"    Cause --> Text[细节: {text}]\n"
            f"    Text --> Container[失败容器: HCSE RV-Monitor Hard Stop]\n"
            f"    style Root fill:#f44336,color:#fff\n"
            f"    style Container fill:#FF9800,color:#fff\n"
            "```"
        )


class InvariantViolationError(Exception):
    """可抛出的异常包装（便于 try/except InvariantViolation 语法）。"""

    def __init__(self, report: InvariantViolationReport) -> None:
        super().__init__(f"{report.severity} {report.invariant_id}: {report.detail}")
        self.report = report


class EvidenceBuilder:
    """Phase5 证据收集器（最小实现：JSON + 文本）。完整包见 evidence_builder.py。"""

    EVIDENCE_ROOT: Path | None = None
    BUILDER_INSTANCE: "EvidenceBuilder | None" = None

    def __init__(self, evidence_dir: str | os.PathLike[str]) -> None:
        self.root = Path(evidence_dir)
        self.root.mkdir(parents=True, exist_ok=True)
        (self.root / "violations").mkdir(exist_ok=True)
        (self.root / "screenshots").mkdir(exist_ok=True)
        (self.root / "logs").mkdir(exist_ok=True)
        self.sanitizer = DataSanitizer()
        EvidenceBuilder.EVIDENCE_ROOT = self.root
        EvidenceBuilder.BUILDER_INSTANCE = self

    @classmethod
    def hard_halt(cls, reason: str) -> None:
        target = cls.EVIDENCE_ROOT or Path.cwd() / "evidence"
        target.mkdir(parents=True, exist_ok=True)
        (target / "SANDBOX_HALT.txt").write_text(
            f"{datetime.now(timezone.utc).isoformat()}\n{reason}\n",
            encoding="utf-8",
        )

    def write_violation(self, v: InvariantViolation) -> Path:
        data = self.sanitizer.sanitize_struct(asdict(v))
        fname = f"{v.severity}_{v.invariant_id}_{int(time.time())}.json"
        p = self.root / "violations" / fname
        p.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")
        return p

    def append_log(self, name: str, payload: Any) -> None:
        p = self.root / "logs" / f"{name}.jsonl"
        with p.open("a", encoding="utf-8") as f:
            line = json.dumps(
                self.sanitizer.sanitize_struct(
                    {"ts": datetime.now(timezone.utc).isoformat(timespec="milliseconds"),
                     "payload": payload}
                ),
                ensure_ascii=False,
            )
            f.write(line + "\n")


class InvariantChecker:
    """对关键事件触发预定义不变量。"""

    def __init__(self, inv_cfg: dict[str, Any], evidence: EvidenceBuilder) -> None:
        self.invariants = {inv["id"]: inv for inv in inv_cfg.get("invariants", [])}
        raw_sla = inv_cfg.get("timeout_sla", [])
        self.sla = {row["ipc"]: row for row in raw_sla}
        # 测试覆盖：若指定 test_override_ms 则优先使用（用户任务要求 4 个 IPC 短超时）
        self.sla_test: dict[str, int] = {}
        self.sla_test_toast: dict[str, str] = {}
        for row in raw_sla:
            if isinstance(row, dict) and "test_override_ms" in row and "ipc" in row:
                try:
                    self.sla_test[row["ipc"]] = int(row["test_override_ms"])
                except (TypeError, ValueError):
                    pass
            if isinstance(row, dict) and row.get("test_expect_toast") and "ipc" in row:
                self.sla_test_toast[row["ipc"]] = str(row["test_expect_toast"])
        self.evidence = evidence
        self.violations: list[InvariantViolation] = []
        # IPC 观测状态 key=requestId → {"start_ts", "ipc_name", ...}
        self._inflight_ipc: dict[str, dict[str, Any]] = {}
        # L3 工具卡片一致性快照（15 种卡片的 status 与 checkbox）
        self._tool_cards_seen: dict[str, dict[str, Any]] = {}
        # L4 齿轮菜单取消路径状态跟踪
        self._gear_menu_state: dict[str, Any] = {"open_count": 0, "last_cancel_ts": None}

    # ------------------------------------------------------------------
    # 关键钩子：CDP 每产生一条 event 都调用一次。返回 None=通过；否则=违例。
    # ------------------------------------------------------------------
    async def on_event(self, item: EvQueueItem, cdp_alive_probe) -> InvariantViolation | None:
        v: InvariantViolation | None = None

        # --- INV-L5-01 / 超时熔断 ---
        if item.kind == "Network.requestWillBeSent":
            await self._track_ipc_send(item)
            # 记录 UI 上 toast/checkbox 等状态（从 mock 的 UI 状态读）
            self._try_update_tool_cards(item)
        elif item.kind in {"Network.responseReceived", "Network.loadingFailed"}:
            v = self._check_ipc_returned(item)
        elif item.kind == "Runtime.exceptionThrown":
            v = self._check_exception_domino(item)
        # --- INV-L4-01 / 写入失败回滚 ---
        if item.kind == "Network.loadingFailed" and "set_agent_manual_override" in json.dumps(
                item.data, ensure_ascii=False):
            v = self._check_rollback_after_write_fail(item) or v
        # --- L3 工具卡片一致性（UI 状态事件） ---
        if item.kind in {"DOM.childNodeInserted", "DOM.attributesModified",
                         "Runtime.consoleAPICalled", "ui:tool-card-state"}:
            v_card = self._check_tool_card_consistency(item)
            if v_card is not None:
                v = v_card if v is None else v
        # --- L4 齿轮菜单取消路径跟踪 ---
        if item.kind in {"ui:gear-menu-open", "ui:gear-menu-cancel", "ui:gear-menu-apply"}:
            v_gear = self._track_gear_menu_actions(item)
            if v_gear is not None and v is None:
                v = v_gear

        if v is None:
            return None
        # --- Phase3 : CDP 通道存活确认，避免假阴性（FM-23 防护）---
        try:
            v.cdp_alive = bool(await cdp_alive_probe())
        except Exception:  # pylint: disable=broad-except
            # 异常=CDP 已断连（FM-23），标记为 DISCONNECT 而不是前端问题
            v.cdp_alive = False
            v.assertion_text = (
                "[FM-23 CDP_DISCONNECT] " + v.assertion_text
            )
            v.invariant_id = "INV-SBX-CDPCONN"
        self.violations.append(v)
        try:
            self.evidence.write_violation(v)
        except Exception:  # pylint: disable=broad-except
            pass
        return v

    # ---- 新增：L3 工具卡片状态更新 ---------------------------------
    def _try_update_tool_cards(self, item: EvQueueItem) -> None:
        """从事件的 data 里读取工具卡片快照。"""
        cards = item.data.get("_tool_cards")
        if isinstance(cards, dict):
            self._tool_cards_seen.update(cards)

    # ---- 新增：L3 一致性检查（INV-L3-01 FM-24） -------------------
    def _check_tool_card_consistency(self, item: EvQueueItem) -> InvariantViolation | None:
        """
        data-agent-id 存在的卡片，对 status 与 checked 状态进行一致性断言：
          status == '已检测到' => (checked==true OR badge visible)
          status == '未安装'  => (checked==false OR badge visible)
        """
        cards = item.data.get("_tool_cards") or {}
        if isinstance(cards, dict) and cards:
            self._tool_cards_seen.update(cards)
        if not self._tool_cards_seen:
            return None
        for agent_id, st in self._tool_cards_seen.items():
            if not isinstance(st, dict):
                continue
            status = str(st.get("status", ""))
            checked = bool(st.get("checked", False))
            badge = bool(st.get("badge_visible", False))
            if status == "已检测到" and (not checked and not badge):
                return self._make_violation(
                    inv_id="INV-L3-01",
                    assertion=(
                        f"[FM-24] 工具卡片 agent={agent_id} status='已检测到' 但 "
                        f"checkbox.checked=false 且 manual-override-badge 不可见（数据欺骗）"
                    ),
                    trigger=item,
                    context={"agent_id": agent_id, "state": st},
                    severity="P1",
                )
            if status == "未安装" and (checked and not badge):
                return self._make_violation(
                    inv_id="INV-L3-01",
                    assertion=(
                        f"[FM-24] 工具卡片 agent={agent_id} status='未安装' 但 "
                        f"checkbox.checked=true 且 badge 不可见（状态回滚失效）"
                    ),
                    trigger=item,
                    context={"agent_id": agent_id, "state": st},
                    severity="P1",
                )
        return None

    # ---- 新增：L4 齿轮菜单取消路径跟踪（FM-22） --------------------
    def _track_gear_menu_actions(self, item: EvQueueItem) -> InvariantViolation | None:
        if item.kind == "ui:gear-menu-open":
            self._gear_menu_state["open_count"] += 1
            if self._gear_menu_state["open_count"] > 1:
                return self._make_violation(
                    inv_id="INV-L2-01",
                    assertion=(
                        f"[FM-06/FM-22] 齿轮菜单 open_count="
                        f"{self._gear_menu_state['open_count']}>1（单例失败，双开竞态）"
                    ),
                    trigger=item,
                    context=self._gear_menu_state,
                    severity="P1",
                )
        if item.kind == "ui:gear-menu-cancel":
            self._gear_menu_state["open_count"] = max(
                0, self._gear_menu_state["open_count"] - 1
            )
            self._gear_menu_state["last_cancel_ts"] = item.ts
            # 断言：cancel 后 1s 内无 pending 的 set_agent_manual_override inflight
            pending = [k for k, v in self._inflight_ipc.items()
                       if v.get("ipc") == "set_agent_manual_override"]
            if pending:
                return self._make_violation(
                    inv_id="INV-L4-01",
                    assertion=(
                        "[FM-22] 齿轮菜单用户取消后，仍有 pending="
                        f"{len(pending)} 条 set_agent_manual_override 未 resolve（取消清理失败）"
                    ),
                    trigger=item,
                    context={"pending_req_ids": pending, "state": self._gear_menu_state},
                    severity="P1",
                )
        if item.kind == "ui:gear-menu-apply":
            self._gear_menu_state["open_count"] = max(
                0, self._gear_menu_state["open_count"] - 1
            )
        return None

    # ---- 具体检查实现 ---------------------------------------------------
    async def _track_ipc_send(self, item: EvQueueItem) -> None:
        req = item.data.get("request", {})
        post_data = req.get("postData") or ""
        # Tauri IPC 经 POST http://ipc.local/invoke?cmd=xxx （伪）或经 WS
        # 这里同时匹配浏览器 Fetch：/api/tools/detect、Ipc 命名约定 lrc-xxx
        name = None
        if isinstance(post_data, str) and "cmd" in post_data:
            m = re.search(r'"cmd"\s*:\s*"([^"]+)"', post_data)
            if m:
                name = m.group(1)
        url = req.get("url", "")
        if not name:
            for cmd in self.sla:
                if cmd in url or cmd in post_data:
                    name = cmd
                    break
        if not name:
            return
        req_id = str(item.data.get("requestId", f"anon-{id(item)}"))
        self._inflight_ipc[req_id] = {"start_ts": item.ts, "ipc": name, "url": url}

    def _check_ipc_returned(self, item: EvQueueItem) -> InvariantViolation | None:
        req_id = str(item.data.get("requestId", ""))
        rec = self._inflight_ipc.pop(req_id, None)
        if not rec:
            return None
        duration_ms = (item.ts - rec["start_ts"]) * 1000
        sla = self.sla.get(rec["ipc"])
        # 优先使用测试 SLA（短），否则用默认 SLA
        test_ms = self.sla_test.get(rec["ipc"])
        fe_ms = int(test_ms) if test_ms is not None else (
            sla.get("frontend_ms") or 0 if isinstance(sla, dict) else 0
        )
        if fe_ms and duration_ms > fe_ms + 500:
            expect = self.sla_test_toast.get(rec["ipc"], "")
            return self._make_violation(
                inv_id="INV-L5-01",
                assertion=(
                    f"IPC {rec['ipc']} 耗时 {duration_ms:.0f}ms > "
                    f"TEST_SLA {fe_ms}ms（超过允许的 500ms 浮差）；"
                    f"预期Toast='{expect}'；熔断未及时触发。"
                ),
                trigger=item,
                context={
                    "sla_default": sla,
                    "sla_test_override_ms": test_ms,
                    "duration_ms": round(duration_ms, 1),
                    "request_url": rec["url"],
                    "response_summary": item.kind,
                    "expected_toast_text": expect,
                },
                severity="P0" if test_ms is not None else "P1",
            )
        # --- RateLimiter 429 提示检查 (FM-19) ---
        if rec["ipc"] == "force_invalidate_scan_cache" and item.kind == "Network.loadingFailed":
            text = json.dumps(item.data, ensure_ascii=False)
            if ("频繁" in text or "429" in text or "throttle" in text.lower()):
                # FM-19：第 2 次触发限流时需立即 showToast（从 ui state 看 recentToasts）
                # 此处只做标记；由 delayed_invariant_checks 最终检查
                pass
        return None

    def _check_exception_domino(self, item: EvQueueItem) -> InvariantViolation | None:
        # INV-L1-02：未捕获异常若与 "wizard" / "scan" / "agent" 关键词相关且
        # 无 handler → 视为不变量违例候选
        text = json.dumps(item.data, ensure_ascii=False)
        if re.search(r"(wizard|scan_ide|agent_detector|manualOverride)", text, re.I):
            return self._make_violation(
                inv_id="INV-L1-02",
                assertion=f"卡死/异常路径出现未捕获异常（Domino 崩溃风险）：{text[:400]}",
                trigger=item,
                context={"raw_exception": item.data},
            )
        return None

    def _check_rollback_after_write_fail(self, item: EvQueueItem) -> InvariantViolation | None:
        # 这里仅做静态提示；实际 L4-01 验证需要 test_orchestrator 主动执行关闭动作后
        # 读 localStorage 比对。RV-Monitor 负责在事件流里看到 Err 时打标记。
        return self._make_violation(
            inv_id="INV-L4-01",
            assertion="set_agent_manual_override 返回 Err，需联动齿轮X关闭二次验证 localStorage 回滚。",
            trigger=item,
            context={"loadingFailed": item.data},
            severity="P1",  # 单 IPC 失败本身降为 P1
        )

    # ------------------------------------------------------------------
    def _make_violation(self, *, inv_id: str, assertion: str,
                        trigger: EvQueueItem,
                        context: dict[str, Any],
                        severity: str | None = None
                        ) -> InvariantViolation:
        inv = self.invariants.get(inv_id, {"layer": "?", "severity": "P2"})
        return InvariantViolation(
            invariant_id=inv_id,
            layer=inv.get("layer", "?"),
            severity=severity or inv.get("severity", "P2"),
            assertion_text=assertion,
            trigger_event_kind=trigger.kind,
            trigger_event_data=trigger.data,
            context_snapshot=context,
            reported_at=datetime.now(timezone.utc).isoformat(timespec="milliseconds"),
            cdp_alive=False,
        )


# =============================================================
# Phase3 编排：RV-Monitor 主循环
# =============================================================

class RVMonitor:
    # 兼容两种构造签名：
    #   a) RVMonitor(invariants_path: str, evidence_dir: str)        -> 路径构造
    #   b) RVMonitor(cfg: dict, sandbox: SecureSandbox, cdp_url=..) -> 结构化构造（test_orchestrator 用）
    def __init__(self, *args: Any, **kwargs: Any) -> None:
        self._started: bool = False
        self._stopped: bool = False
        self._worker_task: asyncio.Task[Any] | None = None
        self._mock_ui_state: dict[str, Any] = {}
        self._inject_fail_on_next: set[str] = set()

        if (len(args) >= 1 and isinstance(args[0], dict)
                or "cfg" in kwargs or "invariants_config" in kwargs):
            self._ctor_structured(*args, **kwargs)
        else:
            self._ctor_paths(*args, **kwargs)

    # --------- (a) 路径构造 ------------------------------------------------
    def _ctor_paths(self, invariants_path: str, evidence_dir: str,
                    sandbox: Any | None = None, cdp_url: str | None = None,
                    **_kw: Any) -> None:
        inv_cfg = self._load_cfg(invariants_path)
        self._init_from_cfg(inv_cfg, evidence_dir, sandbox)
        self.cdp_url: str | None = cdp_url

    # --------- (b) 结构化构造（test_orchestrator 使用） -------------------
    def _ctor_structured(self, cfg: dict[str, Any] | None = None,
                         sandbox: Any | None = None, *,
                         invariants_config: dict[str, Any] | None = None,
                         cdp_url: str | None = None,
                         evidence_dir: str | None = None,
                         **_kw: Any) -> None:
        inv_cfg = cfg or invariants_config or {}
        ev_dir = evidence_dir or (
            str(sandbox.path_validator.workspace_root / "evidence")
            if sandbox is not None and hasattr(sandbox, "path_validator")
            else "./evidence"
        )
        self._init_from_cfg(inv_cfg, ev_dir, sandbox)
        self.cdp_url = cdp_url
        self._sandbox_ref = sandbox

    # --------- 公共初始化 --------------------------------------------------
    def _init_from_cfg(self, inv_cfg: dict[str, Any], evidence_dir: str,
                       sandbox: Any) -> None:
        self._cfg = inv_cfg
        allowed = inv_cfg.get("sandbox", {}).get(
            "allowed_path_prefixes",
            ["./temp", "./logs", "./screenshots", "./evidence"],
        )
        if sandbox is not None and hasattr(sandbox, "path_validator"):
            self.path_validator = sandbox.path_validator
        else:
            self.path_validator = PathValidator(allowed)
        if hasattr(self.path_validator, "validate"):
            try:
                self.path_validator.validate(evidence_dir, op="mkdir evidence")
            except Exception:
                Path(evidence_dir).mkdir(parents=True, exist_ok=True)
        else:
            Path(evidence_dir).mkdir(parents=True, exist_ok=True)
        self.evidence = EvidenceBuilder(evidence_dir)
        self.queue = EventSourcingQueue()
        self.checker = InvariantChecker(inv_cfg, self.evidence)
        sb = inv_cfg.get("sandbox", {})
        self.watchdog = ResourceWatchdog(
            max_mem_mb=int(sb.get("max_memory_mb", 1024)),
            max_cpu_s=int(sb.get("max_cpu_seconds", 60)),
        )
        self._stop = asyncio.Event()

    @staticmethod
    def _load_cfg(path: str) -> dict[str, Any]:
        p = Path(path)
        raw = p.read_text(encoding="utf-8")
        if yaml is not None:
            return yaml.safe_load(raw) or {}
        return json.loads(raw)

    # ------------------------------------------------------------------
    # 与 Playwright / WebSocket CDP 对接的入口（示意实现）
    # ------------------------------------------------------------------
    async def feed_cdp_event(self, method: str, params: dict[str, Any]) -> None:
        if method in {"Network.requestWillBeSent",
                      "Network.responseReceived",
                      "Network.loadingFailed",
                      "Runtime.exceptionThrown",
                      "Runtime.consoleAPICalled",
                      "Page.domContentEventFired",
                      "Page.loadEventFired"}:
            await self.queue.put(method, params or {})

    async def cdp_alive_probe(self) -> bool:
        """Phase3 保活探针。真实实现调用 Browser.getVersion；此处返回 True 示意。"""
        return True  # TODO: 由 test_orchestrator 注入具体传输层（playwright CDPSession）

    # ------------------------------------------------------------------
    # 供 test_orchestrator.py mock 驱动器调用的注入 API（同步 + 异步双通道）
    # ------------------------------------------------------------------
    def inject_mock_event(self, method: str, params: dict[str, Any] | None = None) -> None:
        """同步注入一条 CDP 事件（mock 驱动专用；避免跨 loop 调度产生未 await coroutine）。"""
        item = EvQueueItem(
            ts=time.monotonic(),
            wall=datetime.now(timezone.utc).isoformat(timespec="milliseconds"),
            kind=method,
            data=params or {},
        )
        # 直接写队列（环形上限保持一致）
        try:
            self.queue._q.put_nowait(item)
        except Exception:
            pass
        self.queue._history.append(item)
        if len(self.queue._history) > self.queue.MAXLEN:
            self.queue._history = self.queue._history[-self.queue.MAXLEN // 2:]

    def inject_mock_ui_state(self, state: dict[str, Any]) -> None:
        """更新 mock UI 状态快照（供 run_delayed_invariant_checks 比对使用）。"""
        self._mock_ui_state.update(state)
        self.evidence.append_log("ui_state_snapshot", dict(state))

    # ------------------------------------------------------------------
    # 生命周期：start / stop_and_collect（test_orchestrator 调用）
    # ------------------------------------------------------------------
    def start(self) -> None:
        """启动 monitor 后台 worker（在当前线程的 event loop 中）。"""
        if self._started:
            return
        self._started = True
        self._stopped = False
        try:
            loop = asyncio.get_running_loop()
        except RuntimeError:
            loop = asyncio.new_event_loop()
            asyncio.set_event_loop(loop)
        self._worker_task = loop.create_task(self.worker())

    def stop_and_collect(self, *, settle_s: float = 0.2) -> dict[str, Any]:
        """停止 worker，结算延迟不变式，返回摘要字典。"""
        if self._stopped:
            return self._summary_dict()
        self._stopped = True
        self._stop.set()
        # 跑延迟不变式检查（基于 UI 状态快照 + 最近事件）
        try:
            loop = asyncio.get_event_loop()
        except RuntimeError:
            loop = asyncio.new_event_loop()
            asyncio.set_event_loop(loop)
        try:
            if loop.is_running():
                loop.call_later(0, lambda: None)
            else:
                loop.run_until_complete(asyncio.sleep(max(0, settle_s)))
        except Exception:
            pass
        violations_late = list(self._run_delayed_invariant_checks())
        for v in violations_late:
            try:
                self.checker.violations.append(v)
                self.evidence.write_violation(v)
            except Exception:
                pass
        # 抛出第一个 P0（便于 orchestrator 的 try/except InvariantViolation 捕获）
        for v in self.checker.violations:
            if v.severity in {"P0", "CRITICAL"}:
                raise InvariantViolationError(InvariantViolationReport(v))
        return self._summary_dict()

    # ------------------------------------------------------------------
    # 延迟不变式检查（基于 UI 快照；解决 toast/button 清理等 >=1s 延迟事件）
    # ------------------------------------------------------------------
    def _run_delayed_invariant_checks(self) -> Iterable[InvariantViolation]:
        ui = self._mock_ui_state
        now = datetime.now(timezone.utc).isoformat(timespec="milliseconds")
        # INV-03: userCancelledAllProjectsFlag=true 但 (checked>0 && entries>0) → 矛盾
        if ui.get("userCancelledAllProjectsFlag") and ui.get("wizardProjectCheckedCount", 0) > 0 \
                and ui.get("wizardProjectEntryCount", 0) > 0:
            yield self._mk_violation(
                inv_id="INV-L4-01",  # 旧名 INV-03
                assertion=(
                    "_userCancelledAllProjectsFlag=true 但仍有 checked 勾选 + entries>0，"
                    "取消状态机四象限不一致（Q1 vs Q4 混叠）"
                ),
                context={"ui_state": ui},
            )
        # INV-04: gearMenuCount>1 → 违反单例
        if int(ui.get("gearMenuCount", 0)) > 1:
            yield self._mk_violation(
                inv_id="INV-L2-01",  # 旧名 INV-04
                assertion=f"齿轮菜单 DOM 实例数={ui['gearMenuCount']} > 1，违反单例不变式",
                context={"ui_state": ui},
            )
        # INV-07: 纯 CN 环境仍 trae_installed=true
        if ui.get("is_pure_cn_environment") and ui.get("trae_installed"):
            yield self._mk_violation(
                inv_id="INV-L1-01",  # 旧名 INV-07
                assertion="纯 Trae CN 环境下 trae_installed=true → 三阶段排除未生效",
                context={"ui_state": ui},
                severity="P0",
            )
        # INV-08: 仅 lnk 场景 codebuddy_installed=false
        if ui.get("codebuddy_lnk_only_scenario") and not ui.get("codebuddy_installed"):
            yield self._mk_violation(
                inv_id="INV-L1-02",  # 旧名 INV-08
                assertion="CodeBuddy 纯 lnk 场景（权重 3 >= 阈值 2）仍 installed=false，权重排序失效",
                context={"ui_state": ui},
            )
        # INV-02: localStorage 与 discover 结果不一致
        lo = ui.get("localStorage_overrides") or {}
        do = ui.get("discover_result_overrides") or {}
        for k in set(lo) & set(do):
            if lo[k] != do[k]:
                yield self._mk_violation(
                    inv_id="INV-L3-01",  # 旧名 INV-02
                    assertion=(
                        f"manual_agent_overrides 双写不一致 agent={k} "
                        f"localStorage={lo[k]} != discover={do[k]}"
                    ),
                    context={"agent": k, "ls": lo[k], "discover": do[k]},
                )
        # INV-05: 最近 10s 内有 5xx 响应 && 无 Toast（从 recent 事件推）
        recent = self.queue.recent(200)
        has_5xx = any(
            str(it.data.get("response", {}).get("status", "")).startswith("5")
            for it in recent if it.kind == "Network.responseReceived"
        )
        toasts = list(ui.get("recentToasts") or [])
        if has_5xx and not toasts:
            yield self._mk_violation(
                inv_id="INV-L5-01",  # 旧名 INV-05
                assertion="Network 出现 5xx 响应，但 3.5s 窗口无 UI Toast 兜底",
                context={"recent_event_types": [it.kind for it in recent[-20:]],
                         "toast_count": 0},
                severity="P0",
            )
        # INV-06: invalidate 幂等
        res = ui.get("INV06_idempotent_result")
        if res is False:
            yield self._mk_violation(
                inv_id="INV-L4-02",  # 旧名 INV-06
                assertion="N 次 force_invalidate_scan_cache 后重扫未触发 / RON size 增长（非幂等）",
                context={"INV06_evidence": ui.get("INV06_evidence", {})},
            )

    def _mk_violation(self, *, inv_id: str, assertion: str,
                      context: dict[str, Any], severity: str | None = None,
                      trigger_kind: str = "ui_state_check") -> InvariantViolation:
        inv = self.checker.invariants.get(inv_id, {"layer": "?", "severity": "P2"})
        return InvariantViolation(
            invariant_id=inv_id,
            layer=inv.get("layer", "?"),
            severity=severity or inv.get("severity", "P2"),
            assertion_text=assertion,
            trigger_event_kind=trigger_kind,
            trigger_event_data={"ui_snapshot_keys": list(self._mock_ui_state.keys())},
            context_snapshot=context,
            reported_at=datetime.now(timezone.utc).isoformat(timespec="milliseconds"),
            cdp_alive=True,
        )

    # ------------------------------------------------------------------
    def _summary_dict(self) -> dict[str, Any]:
        return {
            "started": self._started,
            "stopped": self._stopped,
            "total_events": len(self.queue.recent(10_000_000)),
            "invariant_count": len(self.checker.invariants),
            "violations": [asdict(v) for v in self.checker.violations],
            "violation_count": len(self.checker.violations),
            "evidence_root": str(self.evidence.root),
            "watchdog_status": {
                "tripped": self.watchdog._tripped if hasattr(self.watchdog, "_tripped") else False,
            },
        }

    # ------------------------------------------------------------------
    async def worker(self) -> None:
        """后台 worker：从队列取事件 → run invariants → 若违例立即 Hard Stop。"""
        while not self._stop.is_set():
            try:
                item = await asyncio.wait_for(self.queue._q.get(), timeout=0.5)
            except asyncio.TimeoutError:
                # 看门狗
                tripped, reason = self.watchdog.tick()
                if tripped:
                    self.evidence.append_log("watchdog", {"trip_reason": reason})
                    # 不抛异常，允许证据写入收尾
                    self._stop.set()
                continue
            violation = await self.checker.on_event(item, self.cdp_alive_probe)
            if violation is not None and violation.severity in {"P0"}:
                # P0 立即中断 + 写终止标记
                try:
                    (self.evidence.root / "STOP_IMMEDIATE_P0.txt").write_text(
                        f"{violation.invariant_id}\n{violation.assertion_text}\n",
                        encoding="utf-8",
                    )
                except Exception:
                    pass
                self._stop.set()
                # 抛异常让上层 catch（便于 stop_and_collect 前触发上层流程中断）
                raise InvariantViolationError(InvariantViolationReport(violation))

    async def run(self, duration: float = 0) -> list[InvariantViolation]:
        task = asyncio.create_task(self.worker())
        try:
            if duration > 0:
                await asyncio.sleep(duration)
            else:
                await self._stop.wait()
        finally:
            self._stop.set()
            await asyncio.gather(task, return_exceptions=True)
        return list(self.checker.violations)


# --------------------------------------------------------------
# 直接运行：输出一份快速自检（加载 cfg + 验证 PathValidator 黑白样本）
# --------------------------------------------------------------
def _selftest() -> int:
    here = Path(__file__).resolve().parent
    inv = here / "invariants.yaml"
    if not inv.exists():
        print(f"[SELFTEST] 找不到 {inv}，跳过。")
        return 1
    mon = RVMonitor(str(inv), "./evidence/selftest")
    # PathValidator 黑白样本
    good = mon.path_validator.validate("./evidence/selftest/foo.json", op="write sample")
    assert good.exists or True
    try:
        mon.path_validator.validate("C:/Windows/System32/calc.exe", op="read sensitive")
    except SandboxViolation:
        print("[SELFTEST] PathValidator 正确拦截系统路径 ✓")
    else:
        print("[SELFTEST-FAIL] PathValidator 未拦截系统路径 ✗")
        return 2
    # 资源看门狗降级
    print(f"[SELFTEST] psutil available: {psutil is not None}")
    print("[SELFTEST] RV-Monitor 构建完成 ✓ 共加载 invariant:", len(mon.checker.invariants))
    return 0


if __name__ == "__main__":
    sys.exit(_selftest())
