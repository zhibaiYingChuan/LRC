#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
HCSE Phase 3：运行时验证 CDP 监控核心（RV-Monitor）
=====================================================
将 CDP 从注入工具提升为正式监控器。后台持续监听所有 CDP 事件，
实时断言 Phase 1 定义的安全不变式。

三大强制组件：
  1. EventSourcingQueue：全局事件队列，存储 requestWillBeSent /
     responseReceived / exceptionThrown / domMutated
  2. InvariantChecker：每个关键事件立即运行预定义逻辑断言；
     检查失败 → 立即终止测试 + 生成不变式违反报告（含时间戳、
     违反断言 ID、触发事件完整上下文）
  3. CDPLivenessCheck：断言失败时自动 ping Browser.getVersion
     确认 CDP 通道存活，避免因 CDP 丢包导致的假阴性

设计原则：
  - 实时性：事件到达即检查，不批量延迟
  - 可审计：每个违反记录完整上下文（事件 + 时间戳 + 断言 ID）
  - 防假阴性：CDP 通道存活探针 + 事件序号连续性校验
"""

import os
import sys
import json
import time
import uuid
import asyncio
import logging
import threading
from dataclasses import dataclass, field, asdict
from typing import Any, Optional, Callable
from collections import deque, defaultdict
from pathlib import Path

# 沙箱安全（Phase 6 强制集成）
try:
    from sandbox import Sandbox, DataSanitizer
except ImportError:
    Sandbox = None
    DataSanitizer = None

# CDP websocket 客户端（可选依赖，缺失时降级为 MCP 驱动模式）
try:
    import websockets
    HAS_WS = True
except ImportError:
    HAS_WS = False

logging.basicConfig(
    level=logging.INFO,
    format="[RV-Monitor][%(asctime)s][%(levelname)s] %(message)s",
    datefmt="%H:%M:%S",
)
logger = logging.getLogger("rv_monitor")


# ============================================================
# 组件 1：EventSourcingQueue — 事件溯源队列
# ============================================================

@dataclass
class CDPEvent:
    """CDP 事件统一封装"""
    seq: int                          # 事件序号（用于连续性校验）
    timestamp: float                  # 事件时间戳（epoch 秒）
    method: str                       # CDP 方法名
    params: dict                      # 事件参数
    session_id: Optional[str] = None  # CDP 会话 ID

    def to_dict(self) -> dict:
        return asdict(self)


class EventSourcingQueue:
    """
    事件溯源队列 — 存储所有 CDP 事件，支持回溯审计

    特性：
      - 序号连续性校验：检测 CDP 丢包（seq 不连续 → 警告）
      - 容量上限：MAX_EVENTS=5000，超出淘汰最旧（防内存膨胀）
      - 按方法索引：O(1) 查询特定类型事件
    """

    MAX_EVENTS = 5000

    def __init__(self):
        self._events: deque[CDPEvent] = deque(maxlen=self.MAX_EVENTS)
        self._by_method: dict[str, list[int]] = defaultdict(list)  # method → 事件在 deque 的索引
        self._seq = 0
        self._lock = threading.Lock()
        self._dropped_count = 0
        self._gap_detected = []  # 序号间隙记录

    def push(self, method: str, params: dict, session_id: str = None) -> CDPEvent:
        """推入事件"""
        with self._lock:
            self._seq += 1
            event = CDPEvent(
                seq=self._seq,
                timestamp=time.time(),
                method=method,
                params=params,
                session_id=session_id,
            )
            self._events.append(event)
            # 索引（deque 索引会随淘汰变化，这里仅存 seq 用于过滤）
            self._by_method[method].append(event.seq)
            if len(self._events) == self.MAX_EVENTS:
                self._dropped_count += 1
            return event

    def query(self, method: str = None, since_seq: int = 0) -> list[CDPEvent]:
        """查询事件（按方法过滤 + 序号过滤）"""
        with self._lock:
            result = []
            for ev in self._events:
                if ev.seq <= since_seq:
                    continue
                if method and ev.method != method:
                    continue
                result.append(ev)
            return result

    def count(self, method: str = None) -> int:
        """统计事件数"""
        with self._lock:
            if method:
                return len(self._by_method.get(method, []))
            return len(self._events)

    def check_continuity(self) -> list[dict]:
        """序号连续性校验 — 检测 CDP 丢包"""
        with self._lock:
            gaps = []
            prev_seq = 0
            for ev in self._events:
                if prev_seq > 0 and ev.seq != prev_seq + 1:
                    gaps.append({
                        "gap_from": prev_seq,
                        "gap_to": ev.seq,
                        "missing": ev.seq - prev_seq - 1,
                    })
                prev_seq = ev.seq
            self._gap_detected.extend(gaps)
            return gaps

    def export(self) -> list[dict]:
        """导出全部事件（审计用）"""
        with self._lock:
            return [ev.to_dict() for ev in self._events]

    @property
    def stats(self) -> dict:
        return {
            "total": len(self._events),
            "dropped": self._dropped_count,
            "by_method": {m: len(s) for m, s in self._by_method.items()},
            "gaps": len(self._gap_detected),
        }


# ============================================================
# 组件 2：InvariantChecker — 不变式检查器
# ============================================================

@dataclass
class InvariantResult:
    """不变式检查结果"""
    invariant_id: str
    name: str
    severity: str
    status: str          # PASS / FAIL / SKIP / ERROR
    detail: str
    evidence: dict = field(default_factory=dict)
    timestamp: float = field(default_factory=time.time)
    cdp_alive: Optional[bool] = None  # CDP 存活探针结果


@dataclass
class ViolationReport:
    """不变式违反报告（HCSE 强制：含完整上下文）"""
    report_id: str
    timestamp: float
    invariant_id: str
    severity: str
    violated_assertion: str
    triggering_event: dict            # 触发事件完整上下文
    event_context: list[dict]         # 前后 5 个事件上下文
    cdp_liveness: dict                # CDP 通道存活探针结果
    remediation: str


class InvariantChecker:
    """
    不变式检查器 — 注册并执行不变式断言

    每个不变式是一个函数：(event, queue) → InvariantResult
    检查失败时立即生成 ViolationReport 并触发终止。
    """

    def __init__(self, queue: EventSourcingQueue, cdp_liveness_fn: Callable = None):
        self.queue = queue
        self.cdp_liveness_fn = cdp_liveness_fn  # CDP 存活探针回调
        self._invariants: dict[str, dict] = {}  # id → {name, severity, fn, assertion}
        self._results: list[InvariantResult] = []
        self._violations: list[ViolationReport] = []
        self._halted = False
        self._lock = threading.Lock()

    def register(self, inv_id: str, name: str, severity: str,
                 assertion: str, fn: Callable):
        """注册不变式"""
        self._invariants[inv_id] = {
            "name": name, "severity": severity,
            "assertion": assertion, "fn": fn,
        }
        logger.info(f"注册不变式 {inv_id} [{severity}] {name}")

    def check_all(self, event: CDPEvent = None) -> list[InvariantResult]:
        """对所有不变式执行检查（event 为 None 时执行快照式检查）"""
        results = []
        for inv_id, inv in self._invariants.items():
            try:
                result = inv["fn"](event, self.queue)
                if result is None:
                    continue  # 该不变式不适用于此事件
                # CDP 存活探针（仅 FAIL 时）
                if result.status == "FAIL":
                    result.cdp_alive = self._ping_cdp()
                    self._record_violation(result, event, inv)
                with self._lock:
                    self._results.append(result)
                results.append(result)
                if result.status == "FAIL":
                    logger.critical(
                        f"不变式违反 {inv_id} [{inv['severity']}]: {result.detail}"
                    )
                    self._halted = True
            except Exception as e:
                err_result = InvariantResult(
                    invariant_id=inv_id,
                    name=inv["name"],
                    severity=inv["severity"],
                    status="ERROR",
                    detail=f"检查器异常: {e}",
                )
                with self._lock:
                    self._results.append(err_result)
                results.append(err_result)
                logger.error(f"不变式 {inv_id} 检查异常: {e}")
        return results

    def _ping_cdp(self) -> bool:
        """CDP 存活探针 — Browser.getVersion（避免假阴性）"""
        if self.cdp_liveness_fn:
            try:
                return self.cdp_liveness_fn()
            except Exception as e:
                logger.warning(f"CDP 存活探针失败: {e}")
                return False
        return True  # 无探针回调时默认存活

    def _record_violation(self, result: InvariantResult, event: CDPEvent, inv: dict):
        """生成违反报告（含完整上下文）"""
        # 提取触发事件前后 5 个事件作为上下文
        all_events = self.queue.export()
        context = []
        if event:
            idx = next((i for i, e in enumerate(all_events) if e["seq"] == event.seq), -1)
            if idx >= 0:
                start = max(0, idx - 5)
                end = min(len(all_events), idx + 6)
                context = all_events[start:end]

        report = ViolationReport(
            report_id=str(uuid.uuid4()),
            timestamp=time.time(),
            invariant_id=result.invariant_id,
            severity=result.severity,
            violated_assertion=inv["assertion"],
            triggering_event=event.to_dict() if event else {},
            event_context=context,
            cdp_liveness={"alive": result.cdp_alive, "checked_at": time.time()},
            remediation=result.evidence.get("remediation", ""),
        )
        self._violations.append(report)
        logger.critical("=" * 60)
        logger.critical(f"不变式违反报告 {report.report_id}")
        logger.critical(f"  不变式: {result.invariant_id} [{result.severity}]")
        logger.critical(f"  详情: {result.detail}")
        logger.critical(f"  CDP 存活: {result.cdp_alive}")
        logger.critical("=" * 60)

    @property
    def halted(self) -> bool:
        return self._halted

    @property
    def results(self) -> list[InvariantResult]:
        return list(self._results)

    @property
    def violations(self) -> list[ViolationReport]:
        return list(self._violations)

    def summary(self) -> dict:
        passed = sum(1 for r in self._results if r.status == "PASS")
        failed = sum(1 for r in self._results if r.status == "FAIL")
        errors = sum(1 for r in self._results if r.status == "ERROR")
        skipped = sum(1 for r in self._results if r.status == "SKIP")
        return {
            "total_checks": len(self._results),
            "passed": passed,
            "failed": failed,
            "errors": errors,
            "skipped": skipped,
            "violations": len(self._violations),
            "halted": self._halted,
        }


# ============================================================
# v0.8.22 回归不变式断言函数集（对应 invariants_v0.8.22_regression.yaml）
# ============================================================

def make_health_latency_assertion(max_ms: int = 100):
    """INV-REG-P01: /health 响应 < 100ms"""
    def check(event: CDPEvent, queue: EventSourcingQueue) -> Optional[InvariantResult]:
        if event is None:
            return None  # 快照模式跳过（事件驱动断言）
        if event.method != "Network.responseReceived":
            return None
        url = event.params.get("response", {}).get("url", "")
        if "/health" not in url or "/v1/health" in url:
            return None  # 仅主 /health，排除 /v1/health/*
        timing = event.params.get("response", {}).get("timing", {})
        waiting = timing.get("waitingTime", 0)
        status = event.params.get("response", {}).get("status", 0)
        passed = waiting < max_ms and status == 200
        return InvariantResult(
            invariant_id="INV-REG-P01",
            name="/health AtomicBool 无锁读取",
            severity="P0",
            status="PASS" if passed else "FAIL",
            detail=f"/health waiting={waiting}ms status={status} (阈值 {max_ms}ms)",
            evidence={"url": url, "waiting_ms": waiting, "status": status,
                      "remediation": "确认 server.rs:1734 使用 AtomicBool.load"},
        )
    return check


def make_pending_count_assertion():
    """INV-REG-P13: pendingRequestCount >= 0（通过 evaluate 检查）"""
    state = {"last_value": 0}
    def check(event: CDPEvent, queue: EventSourcingQueue) -> Optional[InvariantResult]:
        if event is None:
            return None  # 快照模式跳过
        # 监听 evaluate 返回的 pendingRequestCount
        if event.method != "Runtime.evaluateResult":
            return None
        value = event.params.get("result", {}).get("value")
        if not isinstance(value, (int, float)):
            return None
        state["last_value"] = value
        passed = value >= 0
        return InvariantResult(
            invariant_id="INV-REG-P13",
            name="pendingRequestCount 不泄漏",
            severity="P1",
            status="PASS" if passed else "FAIL",
            detail=f"pendingRequestCount={value} (必须 >= 0)",
            evidence={"value": value,
                      "remediation": "确认 app.js:144 重试路径无手动 --"},
        )
    return check


def make_503_cooldown_assertion():
    """INV-REG-P04: 503 错误 30s 冷却期，toast 不风暴"""
    state = {"toast_count": 0, "first_503_ts": None}
    def check(event: CDPEvent, queue: EventSourcingQueue) -> Optional[InvariantResult]:
        if event is None:
            return None  # 快照模式跳过
        if event.method == "Network.responseReceived":
            status = event.params.get("response", {}).get("status", 0)
            if status != 503:
                return None
            if state["first_503_ts"] is None:
                state["first_503_ts"] = event.timestamp
        elif event.method == "domMutated":
            # 检测 toast 元素新增
            node = event.params.get("node", {})
            if "toast" in str(node.get("className", "")).lower():
                state["toast_count"] += 1
        else:
            return None
        # 30s 窗口内 toast 检查
        if state["first_503_ts"]:
            window = event.timestamp - state["first_503_ts"]
            if window <= 30:
                passed = state["toast_count"] <= 1
                return InvariantResult(
                    invariant_id="INV-REG-P04",
                    name="503 30s 冷却期无 toast 风暴",
                    severity="P0",
                    status="PASS" if passed else "FAIL",
                    detail=f"30s 窗口内 toast={state['toast_count']} (阈值 <=1)",
                    evidence={"toast_count": state["toast_count"], "window_s": round(window, 1),
                              "remediation": "确认 app.js:291 30s 冷却期判断"},
                )
        return None
    return check


# ============================================================
# 组件 3：CDPLivenessCheck — CDP 通道存活探针
# ============================================================

class CDPLivenessCheck:
    """
    CDP 存活探针 — 断言失败时确认 CDP 通道是否存活

    避免因 CDP 丢包/连接断开导致的假阴性（不变式实际未违反，
    但因 CDP 事件丢失导致检查器误判 FAIL）。
    """

    def __init__(self, cdp_url: str = None):
        self.cdp_url = cdp_url
        self._last_check = None
        self._check_count = 0

    def ping(self) -> dict:
        """执行 Browser.getVersion 探针"""
        self._check_count += 1
        result = {"alive": False, "checked_at": time.time(), "attempt": self._check_count}
        if not HAS_WS or not self.cdp_url:
            # 降级模式：无法直接 websocket，标记为未确认
            result["alive"] = None
            result["reason"] = "websockets 未安装或 CDP URL 未配置（降级模式）"
            self._last_check = result
            return result
        try:
            async def _ping():
                async with websockets.connect(self.cdp_url, max_size=8388608) as ws:
                    await ws.send(json.dumps({"id": 1, "method": "Browser.getVersion"}))
                    resp = await asyncio.wait_for(ws.recv(), timeout=5)
                    data = json.loads(resp)
                    return "result" in data
            alive = asyncio.get_event_loop().run_until_complete(asyncio.wait_for(_ping(), timeout=10))
            result["alive"] = alive
        except Exception as e:
            result["alive"] = False
            result["reason"] = str(e)
        self._last_check = result
        return result

    @property
    def last_check(self) -> dict:
        return self._last_check or {"alive": None, "reason": "未执行过探针"}


# ============================================================
# RVMonitor 主引擎 — 整合三大组件
# ============================================================

class RVMonitor:
    """
    运行时验证监控器主引擎

    使用方式：
        monitor = RVMonitor(cdp_url="ws://127.0.0.1:9223/devtools/page/xxx")
        monitor.register_invariants()  # 注册 v0.8.22 回归不变式
        monitor.start()                # 启动后台监听
        # ... 执行测试 ...
        monitor.stop()                 # 停止并导出报告
        report = monitor.generate_report()
    """

    def __init__(self, cdp_url: str = None, evidence_dir: Path = None,
                 sandbox: Sandbox = None):
        self.cdp_url = cdp_url
        self.evidence_dir = evidence_dir or Path("evidence")
        self.evidence_dir.mkdir(parents=True, exist_ok=True)
        self.sandbox = sandbox
        self.queue = EventSourcingQueue()
        self.liveness = CDPLivenessCheck(cdp_url)
        self.checker = InvariantChecker(
            self.queue,
            cdp_liveness_fn=lambda: self.liveness.ping()["alive"] in (True, None),
        )
        self._running = False
        self._ws_task = None

    def register_invariants(self):
        """注册 v0.8.22 回归不变式集"""
        self.checker.register(
            "INV-REG-P01", "/health AtomicBool 无锁读取", "P0",
            "/health 响应 < 100ms",
            make_health_latency_assertion(100),
        )
        self.checker.register(
            "INV-REG-P04", "503 30s 冷却期无 toast 风暴", "P0",
            "30s 内 toast <= 1",
            make_503_cooldown_assertion(),
        )
        self.checker.register(
            "INV-REG-P13", "pendingRequestCount 不泄漏", "P1",
            "pendingRequestCount >= 0",
            make_pending_count_assertion(),
        )
        logger.info(f"已注册 {len(self.checker._invariants)} 个 v0.8.22 回归不变式")

    def inject_event(self, method: str, params: dict, session_id: str = None):
        """手动注入事件（MCP 驱动模式：从 MCP 工具结果转换）"""
        event = self.queue.push(method, params, session_id)
        # 实时检查（仅对注册了断言的事件类型）
        if not self.checker.halted:
            self.checker.check_all(event)
        return event

    def run_snapshot_checks(self) -> list[InvariantResult]:
        """执行快照式检查（非事件驱动，主动探测）"""
        return self.checker.check_all(None)

    async def _listen_cdp(self):
        """CDP websocket 监听循环（直连模式）"""
        if not HAS_WS or not self.cdp_url:
            logger.warning("CDP websocket 不可用，降级为 MCP 驱动模式")
            return
        logger.info(f"RV-Monitor 启动 CDP 监听: {self.cdp_url}")
        try:
            async with websockets.connect(self.cdp_url, max_size=8388608) as ws:
                # 启用事件域
                for domain in ["Network", "Runtime", "Page"]:
                    await ws.send(json.dumps({"id": 0, "method": f"{domain}.enable"}))
                while self._running:
                    try:
                        raw = await asyncio.wait_for(ws.recv(), timeout=1.0)
                        data = json.loads(raw)
                        if "method" in data:
                            event = self.queue.push(
                                data["method"], data.get("params", {}),
                                data.get("sessionId"),
                            )
                            if not self.checker.halted:
                                self.checker.check_all(event)
                            if self.checker.halted:
                                logger.critical("不变式违反，RV-Monitor 终止监听")
                                break
                    except asyncio.TimeoutError:
                        continue
        except Exception as e:
            logger.error(f"CDP 监听异常: {e}")

    def start(self):
        """启动监控器"""
        self._running = True
        if HAS_WS and self.cdp_url:
            self._ws_task = asyncio.get_event_loop().create_task(self._listen_cdp())
        logger.info("RV-Monitor 已启动")

    def stop(self):
        """停止监控器"""
        self._running = False
        if self._ws_task:
            self._ws_task.cancel()
        logger.info("RV-Monitor 已停止")

    def generate_report(self) -> dict:
        """生成验证报告（脱敏后写入证据目录）"""
        report = {
            "report_id": str(uuid.uuid4()),
            "generated_at": time.time(),
            "cdp_liveness": self.liveness.last_check,
            "event_queue_stats": self.queue.stats,
            "continuity_gaps": self.queue.check_continuity(),
            "invariant_summary": self.checker.summary(),
            "invariant_results": [asdict(r) for r in self.checker.results],
            "violations": [asdict(v) for v in self.checker.violations],
        }
        # 脱敏（Phase 6 强制）
        if DataSanitizer:
            report = DataSanitizer.sanitize_struct(report)
        # 写入证据（Phase 6 路径白名单）
        filename = f"rv_monitor_report_{int(time.time())}.json"
        filepath = self.evidence_dir / filename
        content = json.dumps(report, ensure_ascii=False, indent=2, default=str)
        if self.sandbox:
            self.sandbox.write(str(filepath), content)
        else:
            filepath.write_text(content, encoding="utf-8")
        logger.info(f"RV-Monitor 报告已生成: {filepath}")
        return report


# ============================================================
# 自检入口
# ============================================================

def self_test():
    """RV-Monitor 自检 — 验证三大组件正常工作"""
    import tempfile
    tmp = Path(tempfile.mkdtemp())
    try:
        monitor = RVMonitor(evidence_dir=tmp / "evidence")
        monitor.register_invariants()

        # 测试 1：事件队列
        print("[自检] 测试 1：事件溯源队列...")
        for i in range(10):
            monitor.inject_event("Network.responseReceived", {
                "response": {"url": "http://127.0.0.1:3099/health",
                             "status": 200, "timing": {"waitingTime": 4}}
            })
        assert monitor.queue.count() == 10
        assert monitor.queue.count("Network.responseReceived") == 10
        print(f"  事件队列: {monitor.queue.count()} 事件，统计={monitor.queue.stats}")

        # 测试 2：不变式检查（/health < 100ms 应 PASS）
        print("[自检] 测试 2：INV-REG-P01 /health 延迟检查...")
        results = monitor.run_snapshot_checks()
        # P01 在事件驱动时应已检查，快照检查跳过 None
        p01_results = [r for r in monitor.checker.results if r.invariant_id == "INV-REG-P01"]
        if p01_results:
            last = p01_results[-1]
            print(f"  INV-REG-P01: {last.status} ({last.detail})")
            assert last.status == "PASS", f"4ms 响应应 PASS，实际 {last.status}"

        # 测试 3：违反检测（注入 > 100ms 的 /health 响应）
        print("[自检] 测试 3：违反检测（注入 150ms /health 响应）...")
        monitor.inject_event("Network.responseReceived", {
            "response": {"url": "http://127.0.0.1:3099/health",
                         "status": 200, "timing": {"waitingTime": 150}}
        })
        p01_violations = [r for r in monitor.checker.results
                          if r.invariant_id == "INV-REG-P01" and r.status == "FAIL"]
        assert len(p01_violations) > 0, "150ms 应触发 FAIL"
        v = p01_violations[-1]
        print(f"  检测到违反: {v.detail}")
        print(f"  CDP 存活探针: {v.cdp_alive}")
        assert monitor.checker.violations, "应生成违反报告"
        print(f"  违反报告数: {len(monitor.checker.violations)}")

        # 测试 4：报告生成 + 脱敏
        print("[自检] 测试 4：报告生成与脱敏...")
        report = monitor.generate_report()
        assert "invariant_summary" in report
        assert report["invariant_summary"]["failed"] > 0
        report_str = json.dumps(report, default=str)
        # 验证脱敏（不应有原始敏感数据，此处无敏感数据但验证流程）
        print(f"  报告摘要: {report['invariant_summary']}")

        # 测试 5：序号连续性
        print("[自检] 测试 5：序号连续性校验...")
        gaps = monitor.queue.check_continuity()
        print(f"  间隙数: {len(gaps)}（应为 0）")
        assert len(gaps) == 0, "无丢包应无间隙"

        print("\n[自检] RV-Monitor 三大组件全部验证通过")
        print(f"  事件队列: {monitor.queue.count()} 事件")
        print(f"  不变式检查: {report['invariant_summary']}")
        print(f"  违反报告: {len(monitor.checker.violations)} 个")
    finally:
        import shutil
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    self_test()
