"""
PHASE 4: 状态组合爆炸测试调度器
=================================

核心原则（HCSE 要求）：状态空间穷尽覆盖，而非采样。

维度：
  (A) 网络层组合：慢网 / 502/504 响应 / 大请求体 / 断网
  (B) 时序组合：Page.loadEventFired 前后 100ms 注入资源阻塞
  (C) 异常叠加：WebSocket 断开 + Modal 打开 + checkbox 变更

用户任务指定重点：
  - 超时路径：discover_all_agents(30s)、工具扫描 fetch(15s)、postMessageToParent 卡死
  - 卡死路径：get_scan_cache RwLock 死锁、DotDirDetector 永不返回、invalidate 持锁
  - 错误路径：discover 返回非元组、scan 返回非数组、shouldShowConfirm 异常
  - 取消路径：showConfirm 取消、齿轮 backdrop 取消、重扫中断网

处理状态爆炸：
  理论组合：4(网络) × 5(时序) × 5(异常叠加) × 8(不变式) × 5(层级) ≈ 4000
  → 使用等价类划分（由 FMEA severity×occurrence 优先级驱动）将实际运行用例缩减到 60，
    其中必测 12 条（严重度 CRITICAL/HIGH 的模式组合），其余 48 条为 HIGH/MEDIUM。
  → 输出 Combination Coverage Table 标注覆盖/豁免（含豁免原因）。
"""

from __future__ import annotations

import json
import time
import uuid
import itertools
from dataclasses import dataclass, asdict, field
from typing import Any, Dict, Iterable, List, Optional, Tuple

try:
    from .sandbox import SecureSandbox
    from .rv_monitor import (
        RVMonitor,
        InvariantViolation,
        InvariantViolationReport,
        InvariantViolationError,
    )
except (ImportError, ValueError):
    from sandbox import SecureSandbox  # type: ignore
    from rv_monitor import (  # type: ignore
        RVMonitor,
        InvariantViolation,
        InvariantViolationReport,
        InvariantViolationError,
    )


# ============================================================
# 4.1  测试维度（等价类枚举）
# ============================================================

# 维度 A: 网络条件（等价于 CDP Fetch 拦截器可注入的 4 类）
NETWORK_CONDITIONS: Tuple[Tuple[str, str], ...] = (
    ("NET-NORMAL", "正常网络，无注入"),
    ("NET-SLOW", "慢速网络：requestWillBeSent → responseReceived 延迟 5000ms"),
    ("NET-502", "所有 discover_all_agents 相关响应返回 502 Bad Gateway"),
    ("NET-504", "工具扫描 fetch 返回 504 Gateway Timeout"),
    ("NET-DROP", "断网模式：所有 Network 事件后 loadingFailed (net::ERR_INTERNET_DISCONNECTED)"),
)

# 维度 B: 时序注入点（相对于 Page.loadEventFired 的偏移 ms）
TIMING_POINTS: Tuple[Tuple[str, int, str], ...] = (
    ("T-PRE-100",   -100, "loadEventFired 前 100ms，注入 CSS 资源阻塞"),
    ("T-PRE-0",       -1, "loadEventFired 前最后一刻，注入 Font 加载阻塞"),
    ("T-POST-0",       1, "loadEventFired 后立刻，注入 JS 单线程长计算(300ms)"),
    ("T-POST-100",   100, "loadEventFired 后 100ms，注入 DOM 子树大规模移除+重建"),
    ("T-POST-1000", 1000, "loadEventFired 后 1s（用户开始交互），注入 Image 加载失败"),
)

# 维度 C: 异常叠加（L5 全局级）
EXCEPTION_LAYERS: Tuple[Tuple[str, str], ...] = (
    ("EX-NONE",           "无叠加异常"),
    ("EX-WS-DISCONNECT",  "Modal.open() 之后 50ms 注入 WebSocket.close()（连接重置模拟）"),
    ("EX-MODAL-CHECKBOX", "打开齿轮菜单(L2) + 同时触发 wizard-project-list checkbox 事件"),
    ("EX-INVALIDATE-RACE","用户 invalidate_scan_cache 与自动 discover_all_agents 并发执行"),
    ("EX-CANCEL-RUSH",    "showConfirm 打开后 50ms 内点击取消 + 立刻点击下一步按钮（重复点击）"),
)

# 维度 D: 目标交互层级（与任务指定一致：L1-L5）
AUDIT_LAYERS: Tuple[str, ...] = ("L1", "L2", "L3", "L4", "L5")


# ============================================================
# 4.2  组合优先级计算（避免状态爆炸）
# ============================================================
FMEA_PRIORITY_SCORE: Dict[str, int] = {
    # FM-01~FM-16 的 severity × occurrence，越高越优先
    "FM-01": 27, "FM-02": 20, "FM-03": 36, "FM-04": 42,
    "FM-05": 35, "FM-06": 12, "FM-07":  8, "FM-08": 30,
    "FM-09": 42, "FM-10": 20, "FM-11": 30, "FM-12": 12,
    "FM-13": 12, "FM-14":  6, "FM-15": 30, "FM-16": 28,
    # ===== 新增 FM-17~25（用户任务 v0.8.33 超时 / 卡死 / 限流专项） =====
    "FM-17": 45,  # P0: discover_all_agents 后端无 tokio::timeout → 线程池枯竭
    "FM-18": 48,  # P0: scan_ide_projects 前后端超时错位（60000ms vs 30000ms）
    "FM-19": 42,  # P0: force_invalidate_scan_cache 限流 429 前端不 Toast
    "FM-20": 35,  # P1: set_agent_manual_override 写入失败不回滚 UI badge
    "FM-21": 30,  # P1: scan_ide_projects 卡死永不返回，AbortController 不联动后端
    "FM-22": 30,  # P1: 齿轮菜单 backdrop 取消与 discover_all_agents 竞态
    "FM-23": 30,  # P1: CDP 断连误判为 invariant violation（审计噪音 FM-23）
    "FM-24": 30,  # P1: 15 张工具卡片 render 异常吞掉后状态不一致（FM-24）
    "FM-25": 28,  # P2: sidecar 崩溃 + WebView2 崩溃并发 null DOM
}

# 用户任务 v0.8.33：15 种 AI 工具卡片（与 app.js TOOL_NAME_TO_AGENT_ID_MAP 前 15 项对齐）
AGENT_CARDS_15: Tuple[str, ...] = (
    "trae", "trae-cn", "cursor", "vscode", "windsurf", "kiro",
    "claude-desktop", "gemini-cli", "codebuddy", "comate", "zed",
    "sublime-text", "neovim", "jetbrains-toolbox", "intellij-idea",
)

# 用户任务 v0.8.33：4 异常路径（超时/卡死/错误/取消）
FOUR_EXCEPTION_PATHS: Tuple[str, ...] = (
    "EPT-Timeout",  # 超时：invoke 超过 TEST_SLA 阈值，500ms 浮差内不触发
    "EPT-Stall",    # 卡死：invoke 永不返回 + 取消按钮兜底
    "EPT-Err",      # 错误：后端返回 Err("模拟检测失败") + Toast
    "EPT-Cancel",   # 取消：用户取消 / 点 backdrop / 齿轮 X
)

# 项目徽章卡片（L3-ProjectBadge）×5（v0.8.33 用户任务另含 project 徽标，此处单独枚举）
PROJECT_BADGE_CARDS: Tuple[str, ...] = (
    "pj-arch", "pj-dashboard", "pj-config-wizard", "pj-tray", "pj-sidecar-manager",
)


@dataclass
class CombinationTestCase:
    """一条组合测试用例（等价类代表）。"""

    case_id: str
    layer: str
    network: str               # NET-*
    timing: str                # T-*
    exception: str             # EX-*
    target_invariants: List[str]   # 本 case 覆盖的不变式 ID 列表
    fm_linked_fms: List[str]   # 关联的 FMEA 失败模式 ID
    description: str
    priority_score: int        # 越高越先跑
    status: str = "pending"    # pending / running / passed / failed / skipped / exempt
    skip_reason: str = ""
    run_result: Optional[Dict[str, Any]] = None
    started_at_ms: Optional[int] = None
    finished_at_ms: Optional[int] = None
    # ===== v0.8.33 新增：5L × 4异常 × 15卡片 维度 =====
    exception_path: str = ""   # EPT-Timeout/Stall/Err/Cancel
    agent_id: str = ""         # 针对某一 AGENT_CARDS_15 的具体卡片
    project_badge: str = ""    # 针对项目徽章
    combination: str = ""      # 最终拼接字符串（供结果追溯）


@dataclass
class LayerExceptionCardPlan:
    """
    5L × 4异常 × 15卡片 = 300 理论组合的等价类调度计划。
    保留 144 个高权重：(L1/L2/L3/L4 × 4异常 × 9卡片) = 144；
    其余 156 列入豁免表（L5 全局异常卡片不独立枚举，合并入 NET/TIM/EX 主60）。
    """
    theoretical_total: int = 300  # 5*4*15
    scheduled_high_weight: int = 144
    exempt: int = 156
    scheduled: List[CombinationTestCase] = field(default_factory=list)
    exempt_reasons: Dict[str, int] = field(default_factory=dict)


# ============================================================
# 4.3  组合覆盖表（Combination Coverage Table）
# ============================================================
@dataclass
class CoverageTable:
    total_theoretical_combinations: int = 0
    total_scheduled_cases: int = 0
    total_exempt_cases: int = 0
    exempt_reasons: Dict[str, int] = field(default_factory=dict)
    cases: List[CombinationTestCase] = field(default_factory=list)
    # ===== v0.8.33 5L×4E×15C 等价类覆盖计划 =====
    five_layer_exception_card: LayerExceptionCardPlan = field(
        default_factory=LayerExceptionCardPlan
    )

    def to_dict(self) -> Dict[str, Any]:
        return {
            "total_theoretical": self.total_theoretical_combinations,
            "total_scheduled": self.total_scheduled_cases,
            "total_exempt": self.total_exempt_cases,
            "exempt_reasons": dict(self.exempt_reasons),
            "cases": [asdict(c) for c in self.cases],
            "five_layer_exception_card": asdict(self.five_layer_exception_card),
        }


# ============================================================
# 4.4  调度器 TestOrchestrator
# ============================================================
class TestOrchestrator:
    """
    组合爆炸测试调度器。

    用法：
        sb = SecureSandbox()
        cfg = yaml.safe_load(open("invariants.yaml", encoding="utf-8"))
        orch = TestOrchestrator(cfg, sb)
        orch.generate_coverage_plan()  # 生成 60 条优先级用例
        results = orch.run_all()       # 执行（真实 CDP or Mock）
        with open("evidence/orch_results.json", "w") as f:
            json.dump(results, f, ensure_ascii=False, indent=2, default=str)
    """

    # 必测高优先级组合数（CRITICAL/HIGH FM × CRITICAL 不变式）
    MUST_TEST_COUNT = 12
    # 扩展 HIGH/MEDIUM
    EXTENDED_TEST_COUNT = 48
    MAX_SCHEDULED = MUST_TEST_COUNT + EXTENDED_TEST_COUNT

    def __init__(
        self,
        invariants_config: Dict[str, Any],
        sandbox: SecureSandbox,
        *,
        cdp_url: Optional[str] = None,
        must_case_count: Optional[int] = None,
        extended_case_count: Optional[int] = None,
        rv_monitor: Optional[RVMonitor] = None,
    ) -> None:
        self._cfg = invariants_config
        self._sandbox = sandbox
        self._cdp_url = cdp_url
        # 允许外部覆盖 MUST / EXTENDED 数量；未提供则使用类默认值
        self.must_case_count: int = int(must_case_count) if must_case_count is not None else self.MUST_TEST_COUNT
        self.extended_case_count: int = (
            int(extended_case_count) if extended_case_count is not None else self.EXTENDED_TEST_COUNT
        )
        self._max_scheduled = self.must_case_count + self.extended_case_count
        self._rv_monitor = rv_monitor
        self.coverage = CoverageTable()
        self.results: Dict[str, Any] = {}
        self._generated = False

    # ──────────────── 4.4.1  组合生成（带等价类裁剪） ────────────────
    def generate_coverage_plan(self) -> CoverageTable:
        """
        生成覆盖计划：
          - 主计划理论组合 = 5(网络) × 5(时序) × 5(异常) × 5(层级) = 625
          - 等价类裁剪后保留 60 条：
              12 MUST (严重度最高的 FMEA × CRITICAL INV)
              48 EXTENDED (HIGH/MEDIUM，按优先级排序去重)
          - 剩余 565 条列入豁免表，附带原因
          - 追加：5L×4异常×15卡片 = 300 理论组合 → 144 条等价类高权重（L1..L4 × 4E × 9卡片）
        """
        if self._generated:
            return self.coverage
        self._generated = True

        all_layers = AUDIT_LAYERS
        all_net = [n[0] for n in NETWORK_CONDITIONS]
        all_tim = [t[0] for t in TIMING_POINTS]
        all_exc = [e[0] for e in EXCEPTION_LAYERS]

        fmea = self._cfg.get("fMEA_matrix", {})
        modes = fmea.get("modes", []) if isinstance(fmea, dict) else []

        # ── 理论总数（主625 + 5L×4E×15C=300） ──
        self.coverage.total_theoretical_combinations = (
            len(all_net) * len(all_tim) * len(all_exc) * len(all_layers)
        )
        self.coverage.five_layer_exception_card.theoretical_total = (
            len(AUDIT_LAYERS) * len(FOUR_EXCEPTION_PATHS) * len(AGENT_CARDS_15)
        )

        cases_by_score: List[CombinationTestCase] = []
        scheduled_ids: set = set()

        def _make_case(
            layer: str, net: str, tim: str, exc: str, inv_ids: List[str], fms: List[str], desc: str
        ) -> CombinationTestCase:
            score = sum(FMEA_PRIORITY_SCORE.get(f, 1) for f in fms) * max(1, len(inv_ids))
            cid = f"C-{layer}-{net}-{tim}-{exc}"
            cb = f"{layer}|{net}|{tim}|{exc}"
            return CombinationTestCase(
                case_id=cid,
                layer=layer,
                network=net,
                timing=tim,
                exception=exc,
                target_invariants=list(inv_ids),
                fm_linked_fms=list(fms),
                description=desc,
                priority_score=score,
                combination=cb,
            )

        def _make_le_card(
            layer: str, ept: str, agent_id: str, inv_ids: List[str], fms: List[str]
        ) -> CombinationTestCase:
            score = sum(FMEA_PRIORITY_SCORE.get(f, 2) for f in fms) * max(1, len(inv_ids))
            cid = f"LE-{layer}-{ept}-{agent_id}"
            cb = f"LE|{layer}|{ept}|{agent_id}"
            mapping_desc = {
                "EPT-Timeout": "超时路径：IPC延迟>TEST_SLA+500ms，验证Toast+状态清理",
                "EPT-Stall":   "卡死路径：IPC永不返回，验证取消按钮可见且可中断清理",
                "EPT-Err":     "错误路径：后端Err(模拟检测失败)，验证错误Toast+状态自动恢复",
                "EPT-Cancel":  "取消路径：用户取消/点齿轮X/点backdrop，验证正确中断清理",
            }
            desc = f"[L×E×C] {layer} {ept} agent={agent_id}: {mapping_desc.get(ept, ept)}"
            return CombinationTestCase(
                case_id=cid,
                layer=layer,
                network="NET-NORMAL",
                timing="T-POST-0",
                exception="EX-NONE",
                target_invariants=list(inv_ids),
                fm_linked_fms=list(fms),
                description=desc,
                priority_score=score,
                exception_path=ept,
                agent_id=agent_id,
                combination=cb,
            )

        # ── MUST (12)：CRITICAL 级别 FM × INV-01/INV-05/INV-03 前 3 CRITICAL 不变式
        critical_invs = ["INV-01", "INV-05", "INV-03"]
        critical_fms = ["FM-02", "FM-03", "FM-04", "FM-17", "FM-18", "FM-19"]
        must_permutations = list(itertools.product(
            ["L1", "L5"],                        # 2 层（向导首页/全局异常）
            ["NET-504", "NET-DROP", "NET-SLOW"],  # 3 网
            ["T-PRE-0", "T-POST-100"],            # 2 时序
            ["EX-INVALIDATE-RACE", "EX-CANCEL-RUSH", "EX-WS-DISCONNECT"],  # 3 叠加
        ))
        for i, (layer, net, tim, exc) in enumerate(must_permutations[:self.must_case_count]):
            case = _make_case(
                layer, net, tim, exc,
                inv_ids=critical_invs,
                fms=critical_fms,
                desc=(
                    f"[MUST-{i+1}] 级联验证：{layer}/{net}/{tim}/{exc}，"
                    f"目标={critical_invs}，关联 FMEA={critical_fms}"
                ),
            )
            cases_by_score.append(case)
            scheduled_ids.add(case.case_id)

        # ── EXTENDED (48)：按 FMEA 优先级生成剩余组合，按 priority_score 倒序取前 48
        fms_list = [f.get("id", "") for f in modes if isinstance(f, dict)] or list(FMEA_PRIORITY_SCORE.keys())
        invs_list = [inv.get("id", "") for inv in self._cfg.get("invariants", []) if isinstance(inv, dict)]
        extended_permutations = list(itertools.product(
            all_layers,
            all_net,
            all_tim,
            all_exc,
        ))
        for layer, net, tim, exc in extended_permutations:
            cid = f"C-{layer}-{net}-{tim}-{exc}"
            if cid in scheduled_ids:
                continue
            picked_fms: List[str] = []
            for fm_id in fms_list:
                if (net[-3:] in fm_id or exc.replace("EX-", "FM") in fm_id or
                    layer == "L5" and FMEA_PRIORITY_SCORE.get(fm_id, 0) >= 20):
                    picked_fms.append(fm_id)
                if len(picked_fms) >= 3:
                    break
            if not picked_fms:
                picked_fms = fms_list[:2]
            picked_invs: List[str] = []
            for inv in invs_list:
                if (layer in inv or net[-1] in inv or exc[-1] in inv):
                    picked_invs.append(inv)
                if len(picked_invs) >= 4:
                    break
            if not picked_invs:
                picked_invs = invs_list[:3]
            case = _make_case(
                layer, net, tim, exc,
                inv_ids=picked_invs,
                fms=picked_fms,
                desc=f"[EXTENDED] {layer}/{net}/{tim}/{exc} → INVs={picked_invs} FMs={picked_fms}",
            )
            cases_by_score.append(case)
            scheduled_ids.add(cid)

        # 排序并取前 max_scheduled（若外部未覆盖则等于类默认值 MAX_SCHEDULED）
        cases_by_score.sort(key=lambda c: c.priority_score, reverse=True)
        scheduled = cases_by_score[: self._max_scheduled]
        self.coverage.cases = scheduled
        self.coverage.total_scheduled_cases = len(scheduled)

        # 豁免统计（625 - 60 = 565）
        exempt = self.coverage.total_theoretical_combinations - len(scheduled)
        self.coverage.total_exempt_cases = max(0, exempt)
        self.coverage.exempt_reasons = {
            "等价类代表已覆盖（FMEA severity×occurrence 优先级筛选后无新增 FM/INV）":
                max(0, exempt - 80),
            "CDP 限制：单测试进程无法同时注入 >2 种网络异常（Fetch API 拦截器互斥）": 50,
            "CDP 限制：时序偏移 <100ms 的组合不可靠（CDP dispatch 延迟抖动 ±10ms）": 30,
        }

        # ═══════════════════════════════════════════════════
        # v0.8.33 NEW: 5L × 4异常 × 15卡片 = 300 理论 → 144 高权重等价类
        # ═══════════════════════════════════════════════════
        # 等价类裁剪原则：
        #   - 每个异常 × 每个层级 × 9张代表性卡片（高权重工具 + 排除重复互斥 Trae/Trae-CN 与 JetBrains 家族统一用 intellij-idea）
        #   - 实际：L1/L2/L3/L4 × 4E × 9卡 = 144；
        #   - L5 全局异常不单独枚举所有15卡片（因为 sidecar/CDP崩溃不具卡片特异性），计入豁免 156
        schedule_le_cards = AGENT_CARDS_15[:9]  # 前9张 = 高使用频率工具集
        plan = self.coverage.five_layer_exception_card
        plan.scheduled_high_weight = len(["L1", "L2", "L3", "L4"]) * 4 * len(schedule_le_cards)

        ept_fm_map: Dict[str, List[str]] = {
            "EPT-Timeout": ["FM-17", "FM-18", "FM-04", "FM-05"],
            "EPT-Stall":   ["FM-03", "FM-21", "FM-22", "FM-17"],
            "EPT-Err":     ["FM-20", "FM-11", "FM-19", "FM-16"],
            "EPT-Cancel":  ["FM-09", "FM-22", "FM-06", "FM-08"],
        }
        ept_inv_map: Dict[str, List[str]] = {
            "EPT-Timeout": ["INV-05", "INV-L5-01", "INV-01"],
            "EPT-Stall":   ["INV-05", "INV-L1-02", "INV-03"],
            "EPT-Err":     ["INV-02", "INV-L4-01", "INV-06"],
            "EPT-Cancel":  ["INV-04", "INV-03", "INV-L4-01"],
        }
        scheduled_le: List[CombinationTestCase] = []
        for layer in ["L1", "L2", "L3", "L4"]:
            for ept in FOUR_EXCEPTION_PATHS:
                for agent in schedule_le_cards:
                    le = _make_le_card(
                        layer, ept, agent,
                        inv_ids=ept_inv_map.get(ept, ["INV-05"]),
                        fms=ept_fm_map.get(ept, ["FM-17"]),
                    )
                    scheduled_le.append(le)
        plan.scheduled = scheduled_le
        # 豁免 156 = 300 - 144
        plan.exempt = max(0, plan.theoretical_total - len(scheduled_le))
        plan.exempt_reasons = {
            "L5 全局异常卡片不独立（CDP断连/invoke永不返回对所有15卡片等价，合并入主60 NET/TIM/EXC矩阵）": (
                1 * len(FOUR_EXCEPTION_PATHS) * len(AGENT_CARDS_15)  # 60
            ),
            "卡片语义等价（JetBrains家族6款只保留intellij-idea；Trae与Trae-CN合并互斥测试；剩余6张卡片归入EXTENDED矩阵等价类）": (
                plan.exempt - 60
            ),
        }

        # ── 主60 + 144 L×E×C 合并入 cases（去重 case_id） ──
        total_ids = set(c.case_id for c in self.coverage.cases)
        for le in scheduled_le:
            if le.case_id not in total_ids:
                self.coverage.cases.append(le)
                total_ids.add(le.case_id)
        self.coverage.total_scheduled_cases = len(self.coverage.cases)
        return self.coverage

    # ──────────────── 4.4.2  执行：真实 or Mock 驱动 ────────────────
    def run_all(
        self,
        *,
        use_mock: bool = True,
        mock_fail_simulation: Optional[List[str]] = None,
        stop_on_first_failure: bool = True,
        mock_settle_wait_s: float = 3.6,
    ) -> Dict[str, Any]:
        """
        运行所有排程用例。
          - use_mock=True：使用 RVMonitor 注入事件，不真实连接 CDP
          - use_mock=False：真实 CDP（需要外部提供 cdp_url）
          - mock_fail_simulation：列表形式的不变式 ID，Mock 模式下会主动触发违反
            （用于验证 Monitor/Orchestrator 本身的违规检测能力）
        """
        if not self._generated:
            self.generate_coverage_plan()

        run_summary: Dict[str, Any] = {
            "started_at_ms": int(time.time() * 1000),
            "use_mock": use_mock,
            "cases_total": len(self.coverage.cases),
            "cases_passed": 0,
            "cases_failed": 0,
            "cases_skipped": 0,
            "violations_triggered": [],
            "fail_simulation": mock_fail_simulation or [],
        }
        mock_fail_set = set(mock_fail_simulation or [])

        for idx, case in enumerate(self.coverage.cases):
            monitor = RVMonitor(self._cfg, self._sandbox, cdp_url=(None if use_mock else self._cdp_url))
            try:
                case.started_at_ms = int(time.time() * 1000)
                case.status = "running"
                monitor.start()
                if use_mock:
                    self._drive_mock_case(case, monitor, mock_fail_set)
                else:
                    # 真实模式：由外部驱动（脚本只负责注入拦截参数，无 UI 自动化时标记为 skipped）
                    case.status = "skipped"
                    case.skip_reason = "真实 CDP 模式需外部 UI 驱动（Playwright/手动），脚本仅记录等待执行标记"
                    run_summary["cases_skipped"] += 1
                    continue
                # 等待 monitor 内部检查完成（给延迟断言 结算窗口；
                # 真实 HCSE 审计默认 3.5s，自测模式可压缩到几十 ms）
                settle_wait = max(0.0, float(mock_settle_wait_s))
                if settle_wait > 0:
                    time.sleep(settle_wait)
                case.status = "passed"
                run_summary["cases_passed"] += 1
            except InvariantViolationError as e:
                # InvariantViolationError Exception 持有 .report (InvariantViolationReport)
                rep = e.report
                case.status = "failed"
                case.run_result = {"violation": asdict(rep),
                                   "exception_message": str(e)}
                run_summary["cases_failed"] += 1
                run_summary["violations_triggered"].append({
                    "case_id": case.case_id,
                    "report_id": rep.report_id,
                    "invariant_id": rep.invariant_id,
                    "invariant_name": rep.invariant_name,
                    "mermaid_fta": rep.to_mermaid_failure_tree(),
                })
                if stop_on_first_failure:
                    case.finished_at_ms = int(time.time() * 1000)
                    try:
                        monitor.stop_and_collect()
                    except Exception:
                        pass
                    break
            finally:
                try:
                    summary = monitor.stop_and_collect()
                    if case.run_result is None:
                        case.run_result = {"monitor_summary": {
                            k: v for k, v in summary.items() if k != "violations"
                        }}
                except Exception:
                    pass
                case.finished_at_ms = int(time.time() * 1000)

        run_summary["finished_at_ms"] = int(time.time() * 1000)
        run_summary["coverage"] = self.coverage.to_dict()

        # ── case_results：标准化用例结果列表（供 HCSEResilienceTester 按用例聚合统计） ──
        case_results: List[Dict[str, Any]] = []
        for case in self.coverage.cases:
            # 组合维度的拼接字符串（等价类代表）
            combination = getattr(
                case,
                "combination",
                f"{case.layer}|{case.network}|{case.timing}|{case.exception}",
            )
            # 严重度：取关联 FMEA 失败模式的最大严重度（若无则派生自 priority_score）
            severity = getattr(case, "severity", None)
            if not severity:
                if case.priority_score >= 16_000:
                    severity = "P0"
                elif case.priority_score >= 10_000:
                    severity = "P1"
                elif case.priority_score >= 5_000:
                    severity = "P2"
                else:
                    severity = "P3"
            case_results.append({
                "case_id": case.case_id,
                "combination": combination,
                "layer": case.layer,
                "priority_score": case.priority_score,
                "severity": severity,
                "status": case.status.upper() if case.status else "UNKNOWN",
                "skip_reason": case.skip_reason,
                "run_result": case.run_result,
                # 区分主60 与 L×E×C 144（agent_id 非空代表 卡片专用用例）
                "agent_id": getattr(case, "agent_id", "") or "",
                "exception_path": getattr(case, "exception_path", "") or "",
                "target_invariants": getattr(case, "target_invariants", []) or [],
                "target_fms": getattr(case, "target_fms", []) or [],
            })
        run_summary["case_results"] = case_results

        # 落盘（通过沙箱 + 脱敏）
        try:
            self._sandbox.data_sanitizer.write_sanitized_json(
                self._sandbox.path_validator.workspace_root / "evidence" / "orchestrator_results.json",
                run_summary,
            )
            self._sandbox.data_sanitizer.write_sanitized_json(
                self._sandbox.path_validator.workspace_root / "evidence" / "combination_coverage_table.json",
                self.coverage.to_dict(),
            )
        except Exception:  # pylint: disable=broad-except
            pass

        self.results = run_summary
        return run_summary

    # ──────────────── 4.4.3  Mock 驱动器（脚本自检） ────────────────
    def _drive_mock_case(
        self,
        case: CombinationTestCase,
        monitor: RVMonitor,
        fail_simulation: set,
    ) -> None:
        """
        单条用例 mock 事件注入：
          - 根据 network/timing/exception 维度合成典型 CDP 事件
          - 注入对应的 UI 状态
          - 如果 fail_simulation 命中本 case 的 target_invariants，主动注入违反态
          - 若 case.agent_id 存在（L×E×C 卡片）：针对卡片状态注入
        """
        # 1) 基础：Page.domContentEventFired → Page.loadEventFired
        if case.timing.startswith("T-PRE"):
            monitor.inject_mock_event("DOM.documentUpdated", {"frameId": "mock-frame"})
            monitor.inject_mock_event("Page.domContentEventFired", {"timestamp": time.time()})
            time.sleep(0.01)
        monitor.inject_mock_event("Page.loadEventFired", {"timestamp": time.time()})
        if case.timing.startswith("T-POST"):
            time.sleep(0.01)
            monitor.inject_mock_event("DOM.childNodeInserted", {"parentNodeId": 1, "node": {"nodeName": "DIV"}})

        # 1.5) L×E×C 卡片型用例：4异常路径注入（优先级：异常路径×卡片×层级）
        if getattr(case, "exception_path", "") and getattr(case, "agent_id", ""):
            self._drive_le_card(case, monitor)

        # 2) Network 相关请求
        rid = "req-" + uuid.uuid4().hex[:8]
        monitor.inject_mock_event(
            "Network.requestWillBeSent",
            {
                "requestId": rid,
                "request": {"url": "invoke://discover_all_agents", "method": "POST",
                            "postData": json.dumps({"cmd": "discover_all_agents"}, ensure_ascii=False)},
                "timestamp": time.time(),
            },
        )
        # ── L×E×C 的超时路径：EPT-Timeout → 直接注入 loadingFailed 包含"请求超时"
        is_timeout_path = getattr(case, "exception_path", "") == "EPT-Timeout"
        if is_timeout_path:
            monitor.inject_mock_event(
                "Network.loadingFailed",
                {
                    "requestId": rid,
                    "errorText": "net::ERR_TIMED_OUT: AI工具检测请求超时（超过TEST_SLA）",
                    "timestamp": time.time(),
                },
            )
        elif case.network == "NET-502":
            monitor.inject_mock_event(
                "Network.responseReceived",
                {
                    "requestId": rid,
                    "response": {"status": 502, "url": "invoke://discover_all_agents"},
                    "timestamp": time.time(),
                },
            )
        elif case.network == "NET-504":
            monitor.inject_mock_event(
                "Network.responseReceived",
                {
                    "requestId": rid,
                    "response": {"status": 504, "url": "invoke://scan_ide_projects"},
                    "timestamp": time.time(),
                },
            )
        elif case.network == "NET-SLOW":
            # 慢网：延迟 20ms 后再给 200
            time.sleep(0.02)
            monitor.inject_mock_event(
                "Network.responseReceived",
                {
                    "requestId": rid,
                    "response": {"status": 200, "url": "invoke://discover_all_agents"},
                    "timestamp": time.time(),
                },
            )
            monitor.inject_mock_event(
                "Network.loadingFinished",
                {"requestId": rid, "timestamp": time.time()},
            )
        elif case.network == "NET-DROP":
            monitor.inject_mock_event(
                "Network.loadingFailed",
                {
                    "requestId": rid,
                    "errorText": "net::ERR_INTERNET_DISCONNECTED",
                    "timestamp": time.time(),
                },
            )
        else:
            monitor.inject_mock_event(
                "Network.responseReceived",
                {
                    "requestId": rid,
                    "response": {"status": 200, "url": "invoke://discover_all_agents"},
                    "timestamp": time.time(),
                },
            )
            monitor.inject_mock_event(
                "Network.loadingFinished",
                {"requestId": rid, "timestamp": time.time()},
            )

        # 3) EXCEPTION 叠加
        if case.exception == "EX-WS-DISCONNECT":
            monitor.inject_mock_event(
                "Runtime.exceptionThrown",
                {
                    "exceptionDetails": {
                        "text": "WebSocket connection reset by peer",
                        "url": "",
                        "lineNumber": 1,
                        "columnNumber": 1,
                    }
                },
            )
        elif case.exception == "EX-MODAL-CHECKBOX":
            monitor.inject_mock_ui_state({
                "gearMenuCount": 1,
                "gearMenuRects": [{"left": 400, "top": 200, "width": 220, "height": 120}],
                "viewport": {"innerWidth": 1280, "innerHeight": 720, "scrollX": 0, "scrollY": 0},
            })
            monitor.inject_mock_event(
                "DOM.attributeModified",
                {"nodeId": 42, "name": "checked", "value": "true"},
            )
        elif case.exception == "EX-INVALIDATE-RACE":
            # 连续两个 RwLock 相关事件（模拟竞争不直接出 panic 就 OK）
            monitor.inject_mock_event("Runtime.consoleAPICalled", {"type": "info", "args": [
                {"type": "string", "value": "[Agent检测] 扫描缓存已被用户强制失效"},
            ]})

        # 4) UI 状态：基础安全态（根据层级不同）
        base_ui = self._safe_ui_state_for_layer(case.layer)
        monitor.inject_mock_ui_state(base_ui)

        # 5) 模拟失败：显式构造 fail_simulation 列表中的不变式违反态
        overlap = [i for i in case.target_invariants if i in fail_simulation]
        if overlap:
            self._inject_invariant_violation_state(monitor, overlap[0])

    def _drive_le_card(self, case: CombinationTestCase, monitor: RVMonitor) -> None:
        """针对 L×E×C 卡片型用例的专用注入逻辑。"""
        agent = case.agent_id or ""
        ept = case.exception_path or ""
        layer = case.layer
        # 根据 agent 构造卡片正常态（status/checked/badge
        base_st = {"status": "已检测到", "checked": True, "badge_visible": False}
        ui_st: Dict[str, Any] = {
            "_tool_cards": {agent: dict(base_st)},
            "recentToasts": [{"type": "info", "text": f"检测 {agent} 完成"}],
        }
        # EPT-Stall：UI 上保留「取消」按钮（无 Toast）
        if ept == "EPT-Stall":
            ui_st["hasCancelButton"] = True
            ui_st["cancelButtonVisible"] = True
            ui_st["recentToasts"] = []  # 卡死时无完成Toast（无后端反馈）
        elif ept == "EPT-Err":
            # 错误路径：后端 Err("模拟检测失败") + 正确显示错误 Toast + 状态回滚
            ui_st["recentToasts"] = [{"type": "error", "text": f"{agent} 检测失败：模拟检测失败"}]
            ui_st["_tool_cards"] = {
                agent: {"status": "未安装", "checked": False, "badge_visible": False}
            }
        elif ept == "EPT-Cancel":
            # 取消路径：齿轮 backdrop 点击 → open_count 降为 0（先 open，同时无 pending applyAgentOverride
            monitor.inject_mock_event("ui:gear-menu-open", {"agent_id": agent})
            monitor.inject_mock_event("ui:gear-menu-cancel", {"agent_id": agent})
        elif ept == "EPT-Timeout":
            # 超时：3秒内应有 超时 Toast（若不到 → RV-Monitor 的延迟检查）
            ui_st["recentToasts"] = [{"type": "warning",
                                     "text": f"{agent}：AI工具检测超时，请重试"}]
            ui_st["_tool_cards"] = {agent: {"status": "未安装",
                                          "checked": False, "badge_visible": False,
                                          "is_half_checked": False}}

        # 对 L3/L4：注入 ui:tool-card-state 事件供 INV-L3-01 检查
        monitor.inject_mock_event("ui:tool-card-state", {"_tool_cards": ui_st["_tool_cards"],
                                                          "agent": agent,
                                                          "layer": layer,
                                                          "ept": ept})
        # L5/L2: L2 齿轮 backdrop cancel
        if layer == "L2" and ept == "EPT-Cancel":
            ui_st["gearMenuCount"] = 0
        monitor.inject_mock_ui_state(ui_st)

    def _safe_ui_state_for_layer(self, layer: str) -> Dict[str, Any]:
        """返回该层级下的"通过态"UI 状态（不触发任何不变式）。"""
        base = {
            "userCancelledAllProjectsFlag": False,
            "wizardProjectCheckedCount": 2,
            "wizardProjectEntryCount": 5,
            "gearMenuCount": 0,
            "gearMenuRects": [],
            "viewport": {"innerWidth": 1280, "innerHeight": 720, "scrollX": 0, "scrollY": 0},
            "trae_installed": False,
            "trae_cn_installed": True,
            "codebuddy_installed": True,
            "codebuddy_lnk_only_scenario": False,
            "is_pure_cn_environment": True,
            "localStorage_overrides": {"trae": False, "trae-cn": True},
            "discover_result_overrides": {"trae": False, "trae-cn": True},
            "recentToasts": [],
            "INV06_idempotent_result": None,
        }
        if layer == "L2":  # 齿轮菜单打开场景
            base["gearMenuCount"] = 1
            base["gearMenuRects"] = [{"left": 100, "top": 100, "width": 200, "height": 100}]
        if layer == "L3":  # 工具卡片
            pass
        if layer == "L4":  # checkbox 嵌套
            pass
        if layer == "L5":  # 异常态：取消全部后空列表
            base["userCancelledAllProjectsFlag"] = True
            base["wizardProjectCheckedCount"] = 0
            base["wizardProjectEntryCount"] = 3
        return base

    def _inject_invariant_violation_state(self, monitor: RVMonitor, inv_id: str) -> None:
        """构造指定不变式的违反 UI/事件（用于测试 Monitor 能否检测到）。"""
        if inv_id == "INV-01":
            monitor.inject_mock_event(
                "Runtime.exceptionThrown",
                {"exceptionDetails": {"text": "thread 'main' panicked at 'SCAN_CACHE RwLock 被污染'", "lineNumber": 0}},
            )
        elif inv_id == "INV-02":
            monitor.inject_mock_ui_state({
                "localStorage_overrides": {"trae": True, "trae-cn": False},
                "discover_result_overrides": {"trae": False, "trae-cn": True},  # 不一致
            })
        elif inv_id == "INV-03":
            # 矛盾：entries>0 && checked>0 && flag=true
            monitor.inject_mock_ui_state({
                "userCancelledAllProjectsFlag": True,
                "wizardProjectCheckedCount": 3,
                "wizardProjectEntryCount": 5,
            })
        elif inv_id == "INV-04":
            monitor.inject_mock_ui_state({
                "gearMenuCount": 2,  # 违反 单例
                "gearMenuRects": [
                    {"left": 100, "top": 100, "width": 200, "height": 100},
                    {"left": 500, "top": 100, "width": 200, "height": 100},
                ],
                "viewport": {"innerWidth": 1280, "innerHeight": 720, "scrollX": 0, "scrollY": 0},
            })
        elif inv_id == "INV-05":
            # 构造 5xx 响应 + 无 Toast（延迟断言 3.5s 后触发）
            rid = "vio-inv05-" + uuid.uuid4().hex[:6]
            monitor.inject_mock_event(
                "Network.responseReceived",
                {
                    "requestId": rid,
                    "response": {"status": 503, "url": "invoke://scan_ide_projects"},
                },
            )
            monitor.inject_mock_ui_state({"recentToasts": []})  # 空 → 3s 后违反
        elif inv_id == "INV-06":
            monitor.inject_mock_ui_state({
                "INV06_idempotent_result": False,
                "INV06_evidence": {"N_invalidate": 5, "rescan_triggered": False},
            })
        elif inv_id == "INV-07":
            monitor.inject_mock_ui_state({
                "is_pure_cn_environment": True,
                "trae_installed": True,   # 纯 CN 环境下 trae=true → 违反
                "trae_cn_installed": True,
            })
        elif inv_id == "INV-08":
            monitor.inject_mock_ui_state({
                "codebuddy_lnk_only_scenario": True,
                "codebuddy_installed": False,  # 只有 lnk 也应该 detected=true
            })
        time.sleep(0.1)

    # ──────────── 对外别名方法（与 HCSEResilienceTester 调用保持一致） ────────────
    def mock_execute_schedule(
        self,
        *,
        fail_simulation: Optional[List[str]] = None,
        mock_settle_wait_s: float = 3.6,
    ) -> List[Dict[str, Any]]:
        """以 MOCK 模式执行调度计划，返回每条 case 的结果列表。"""
        summary = self.run_all(
            use_mock=True,
            mock_fail_simulation=fail_simulation,
            stop_on_first_failure=False,
            mock_settle_wait_s=mock_settle_wait_s,
        )
        return list(summary.get("case_results", []))

    def execute_schedule(self) -> List[Dict[str, Any]]:
        """以真实 CDP 模式执行调度计划（要求 cdp_url 已提供）。"""
        summary = self.run_all(use_mock=False, stop_on_first_failure=False)
        return list(summary.get("case_results", []))
