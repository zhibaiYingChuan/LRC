#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""LongMemEval 232 复现包静态校验（CI 门禁，无网络、无二进制）。

校验内容：
  1. 配置 / 期望值 / 分片清单 JSON 可解析，关键字段自洽；
  2. 分片清单：250 选中、18 abstention、232 有效，题型分布与期望值一致；
  3. 三份历史日志按实例数加权后，与 expected 的 232 指标在容差内一致；
  4. 评测脚本可编译、不含本机绝对路径，PowerShell 脚本不使用 $args 自动变量；
  5. expected 引用的日志文件真实存在。

用法：
  python eval/longmemeval/scripts/validate_assets.py

退出码：0 = 通过；1 = 校验失败。
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

EVAL_ROOT = Path(__file__).resolve().parent.parent
SCRIPTS = EVAL_ROOT / "scripts"

# 允许的聚合误差：日志保留 4 位小数，加权后误差远小于该值
AGG_TOLERANCE = 5e-4

# 本机绝对路径 / 编辑器文件链接，入库后不可复现
LOCAL_PATH_PATTERNS = [
    re.compile(r"\b[A-Za-z]:\\"),
    re.compile(r"file:///[A-Za-z]:/"),
]


def fail(problems: list[str], msg: str) -> None:
    problems.append(msg)


def load_json(path: Path, problems: list[str]) -> dict:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        fail(problems, f"[JSON] {path.relative_to(EVAL_ROOT)} 无法解析：{exc}")
        return {}


def parse_log(path: Path) -> dict:
    """从评测日志提取实例数、四项总体指标与分题型结果。"""
    text = path.read_text(encoding="utf-8", errors="replace")
    patterns = {
        "n": r"评估实例数:\s*(\d+)",
        "session_recall": r"Session Recall@10:\s*([0-9.]+)",
        "turn_recall": r"Turn Recall@10:\s*([0-9.]+)",
        "session_mrr": r"Session MRR:\s*([0-9.]+)",
        "turn_mrr": r"Turn MRR:\s*([0-9.]+)",
    }
    out: dict = {}
    for key, pat in patterns.items():
        m = re.search(pat, text)
        if not m:
            raise ValueError(f"缺少字段 {key}")
        out[key] = float(m.group(1))

    # 按问题类型表：类型名 + 数量 + 4 个指标（类型名只含小写字母与连字符）
    per_type: dict[str, dict] = {}
    row = re.compile(
        r"^\s{2}([a-z][a-z-]+)\s+(\d+)\s+([0-9.]+)\s+([0-9.]+)\s+([0-9.]+)\s+([0-9.]+)\s*$",
        re.MULTILINE,
    )
    for m in row.finditer(text):
        per_type[m.group(1)] = {
            "n": int(m.group(2)),
            "session_recall": float(m.group(3)),
            "turn_recall": float(m.group(4)),
            "session_mrr": float(m.group(5)),
            "turn_mrr": float(m.group(6)),
        }
    out["per_type"] = per_type
    return out


def check_split(split: dict, expected: dict, problems: list[str]) -> None:
    if split.get("total_selected") != 250:
        fail(problems, f"[分片] total_selected 应为 250，实际 {split.get('total_selected')}")
    if split.get("abstentions") != 18:
        fail(problems, f"[分片] abstentions 应为 18，实际 {split.get('abstentions')}")
    if split.get("evaluated") != 232:
        fail(problems, f"[分片] evaluated 应为 232，实际 {split.get('evaluated')}")

    instances = split.get("instances", [])
    if len(instances) != 250:
        fail(problems, f"[分片] instances 应为 250 条，实际 {len(instances)}")

    evaluated = [x for x in instances if not x.get("abstention")]
    if len(evaluated) != 232:
        fail(problems, f"[分片] 非 abstention 实例应为 232 条，实际 {len(evaluated)}")

    n_shards = split.get("n_shards", 0)
    if n_shards != 3:
        fail(problems, f"[分片] n_shards 应为 3，实际 {n_shards}")
    for item in instances:
        shard = item.get("shard")
        if not isinstance(shard, int) or not 0 <= shard < max(n_shards, 1):
            fail(problems, f"[分片] index={item.get('index')} 的 shard 非法：{shard}")
            break
        if shard != item.get("index", 0) % max(n_shards, 1):
            fail(problems, f"[分片] index={item.get('index')} 未按 index % n_shards 分片")
            break

    type_counts = split.get("question_types", {})
    expected_types = {k: v.get("n") for k, v in expected.get("per_type", {}).items()}
    if type_counts != expected_types:
        fail(problems, f"[分片] 题型分布与 expected 不一致：{type_counts} != {expected_types}")


def check_expected(split: dict, expected: dict, problems: list[str]) -> None:
    if expected.get("mode") != "fast":
        fail(problems, f"[期望] mode 应为 fast，实际 {expected.get('mode')}")
    if expected.get("n_evaluated") != split.get("evaluated"):
        fail(
            problems,
            f"[期望] n_evaluated={expected.get('n_evaluated')} 与分片 evaluated={split.get('evaluated')} 不一致",
        )

    logs = expected.get("provenance", {}).get("shard_logs", [])
    if len(logs) != split.get("n_shards"):
        fail(problems, f"[期望] shard_logs 数量应为 {split.get('n_shards')}，实际 {len(logs)}")

    shards = []
    for rel in logs:
        path = EVAL_ROOT / rel
        if not path.is_file():
            fail(problems, f"[期望] 引用的日志不存在：{rel}")
            continue
        try:
            shards.append(parse_log(path))
        except ValueError as exc:
            fail(problems, f"[日志] {rel} 解析失败：{exc}")

    if len(shards) != len(logs):
        return

    total = sum(s["n"] for s in shards)
    if total != split.get("evaluated"):
        fail(problems, f"[日志] 分片实例数合计 {total} != {split.get('evaluated')}")

    for key, expected_key in [
        ("session_recall", "session_recall_at_10"),
        ("turn_recall", "turn_recall_at_10"),
        ("session_mrr", "session_mrr"),
        ("turn_mrr", "turn_mrr"),
    ]:
        weighted = sum(s[key] * s["n"] for s in shards) / total if total else 0.0
        want = float(expected.get(expected_key, 0.0))
        if abs(weighted - want) > AGG_TOLERANCE:
            fail(
                problems,
                f"[日志] {expected_key} 加权 {weighted:.4f} 与期望 {want:.4f} 不一致"
                f"（容差 {AGG_TOLERANCE}）",
            )

    # 分题型结果同样逐项比对，防止 headline 正确但短板数字漂移。
    agg: dict[str, dict] = {}
    for shard in shards:
        for qtype, values in shard.get("per_type", {}).items():
            bucket = agg.setdefault(
                qtype,
                {"n": 0, "session_recall": 0.0, "turn_recall": 0.0,
                 "session_mrr": 0.0, "turn_mrr": 0.0},
            )
            bucket["n"] += values["n"]
            for key in ("session_recall", "turn_recall", "session_mrr", "turn_mrr"):
                bucket[key] += values[key] * values["n"]

    expected_types = expected.get("per_type", {})
    if set(agg) != set(expected_types):
        fail(problems, f"[日志] 分题型集合与 expected 不一致：{sorted(agg)} != {sorted(expected_types)}")

    per_type_total = sum(v["n"] for v in agg.values())
    if per_type_total != total:
        fail(problems, f"[日志] 分题型实例数合计 {per_type_total} != 总体 {total}")

    for qtype, want in expected_types.items():
        got = agg.get(qtype)
        if not got:
            continue
        if got["n"] != want.get("n"):
            fail(problems, f"[日志] {qtype} 实例数 {got['n']} != expected {want.get('n')}")
        for key, expected_key in [
            ("session_recall", "session_recall_at_10"),
            ("turn_recall", "turn_recall_at_10"),
            ("session_mrr", "session_mrr"),
            ("turn_mrr", "turn_mrr"),
        ]:
            weighted = got[key] / got["n"] if got["n"] else 0.0
            target = float(want.get(expected_key, 0.0))
            if abs(weighted - target) > AGG_TOLERANCE:
                fail(
                    problems,
                    f"[日志] {qtype} {expected_key} 加权 {weighted:.4f} 与期望 {target:.4f} 不一致",
                )


def check_config(split: dict, config: dict, problems: list[str]) -> None:
    sha = config.get("dataset_sha256", "")
    if not re.fullmatch(r"[0-9a-f]{64}", sha or ""):
        fail(problems, f"[配置] dataset_sha256 不是 64 位小写十六进制：{sha!r}")
    if sha != split.get("dataset_sha256"):
        fail(problems, "[配置] dataset_sha256 与分片清单不一致")

    if config.get("max_instances") != split.get("max_instances"):
        fail(problems, "[配置] max_instances 与分片清单不一致")
    if config.get("n_shards") != split.get("n_shards"):
        fail(problems, "[配置] n_shards 与分片清单不一致")
    if config.get("mode") != "fast":
        fail(problems, f"[配置] mode 应为 fast，实际 {config.get('mode')}")

    must_not = set(config.get("build", {}).get("must_not_enable", []))
    if not {"ml", "cuda"}.issubset(must_not):
        fail(problems, f"[配置] build.must_not_enable 必须包含 ml 与 cuda，实际 {sorted(must_not)}")

    # 下载脚本里的 SHA 是另一份真源，必须与配置一致，否则“校验通过”只对了一半。
    downloader = SCRIPTS / "download_data.py"
    if downloader.is_file():
        match = re.search(
            r'"longmemeval_s_cleaned\.json"\s*:\s*"([0-9a-f]{64})"',
            downloader.read_text(encoding="utf-8"),
        )
        if not match or match.group(1) != sha:
            fail(problems, "[配置] download_data.py 的 longmemeval_s_cleaned.json SHA 与配置不一致")


def check_scripts(problems: list[str]) -> None:
    for path in sorted(SCRIPTS.glob("*.py")):
        try:
            compile(path.read_text(encoding="utf-8"), str(path), "exec")
        except SyntaxError as exc:
            fail(problems, f"[脚本] {path.name} 语法错误：{exc}")

    ps1 = SCRIPTS / "run_fast_232.ps1"
    if not ps1.is_file():
        fail(problems, "[脚本] 缺少 run_fast_232.ps1")
    else:
        raw = ps1.read_bytes()
        # Windows PowerShell 5.1 对无 BOM 的 UTF-8 按 ANSI 解码；脚本含中文，
        # 无 BOM 会直接解析失败（REPRODUCE 明确支持 powershell.exe -File 路径）。
        if not raw.startswith(b"\xef\xbb\xbf"):
            fail(problems, "[脚本] run_fast_232.ps1 必须是 UTF-8 with BOM（Windows PowerShell 5 兼容）")
        text = raw.decode("utf-8-sig")
        if re.search(r"\$args\b", text):
            fail(problems, "[脚本] run_fast_232.ps1 使用了 PowerShell 自动变量 $args")


def check_local_paths(problems: list[str]) -> None:
    suffixes = {".py", ".ps1", ".md", ".json", ".txt"}
    paths = [p for p in EVAL_ROOT.rglob("*") if p.is_file() and p.suffix.lower() in suffixes]
    # 根目录评测文档同样是对外入口，不能带本机绝对路径。
    for name in ("BENCHMARKS.md", "REPRODUCE.md", "CLAIMS.md"):
        path = EVAL_ROOT.parent.parent / name
        if path.is_file():
            paths.append(path)
    for path in sorted(paths):
        text = path.read_text(encoding="utf-8", errors="ignore")
        for pat in LOCAL_PATH_PATTERNS:
            if pat.search(text):
                fail(problems, f"[路径] {path} 含本机绝对路径")
                break


def main() -> int:
    problems: list[str] = []

    split = load_json(EVAL_ROOT / "splits" / "fast_232.json", problems)
    expected = load_json(EVAL_ROOT / "expected" / "fast_232.json", problems)
    config = load_json(EVAL_ROOT / "configs" / "fast_232.json", problems)

    if split and expected:
        check_split(split, expected, problems)
    if split and expected:
        check_expected(split, expected, problems)
    if split and config:
        check_config(split, config, problems)

    check_scripts(problems)
    check_local_paths(problems)

    if problems:
        print("LongMemEval 复现包校验未通过：")
        for p in problems:
            print("  " + p)
        return 1

    print("LongMemEval 复现包校验通过：232 口径、分片清单、日志加权与期望值一致。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
