#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""absorb 写质量评测（最小可用版）。

测什么：
  1. 判定精确率 / 召回：该沉淀的结论是否写入，噪声是否被拒；
  2. 内容命中率：写入内容是否保留预期关键词；
  3. 误记代价：噪声/闲聊/删除意图是否被写成记忆（假阳性率）；
  4. 幂等性：同一结论重复沉淀是否被去重（不产生重复记忆）。

隔离性：
  - 复用 LongMemEval 评测器的 stdio 客户端，启动独立 sidecar；
  - 数据目录为临时目录，跑完删除；不接触 3099 / 3111 的生产或开发实例；
  - 重复沉淀探针会真实写库，但只写入临时目录。

用法：
  python eval/write_quality/scripts/run_write_quality.py \
    --exe target/release/code-memory-server.exe

  # 只做 dry_run 判定，不写库
  python eval/write_quality/scripts/run_write_quality.py --exe ... --skip-duplicate-probe
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import sys
import tempfile
import time
from pathlib import Path

WRITE_QUALITY_ROOT = Path(__file__).resolve().parent.parent
LONGMEMEVAL_SCRIPTS = WRITE_QUALITY_ROOT.parent / "longmemeval" / "scripts"
sys.path.insert(0, str(LONGMEMEVAL_SCRIPTS))

from lrc_fair_eval_v3 import LRCStdioClient  # noqa: E402


PROJECT = "lrc-eval-write-quality"
WRITE_REASONS = {"written", "would_write"}


class AbsorbClient(LRCStdioClient):
    """在通用 stdio 客户端上加一个 absorb 调用。"""

    def absorb(self, text: str, session_id: str, dry_run: bool) -> dict:
        result = self._call(
            "tools/call",
            {
                "name": "absorb",
                "arguments": {
                    "text": text,
                    "session_id": session_id,
                    "project": PROJECT,
                    "host": "eval",
                    "source": "auto_distill",
                    "dry_run": dry_run,
                },
            },
            timeout=180,
        )
        if "error" in result:
            raise RuntimeError(f"absorb 调用失败: {result['error']}")
        content = result.get("result", {}).get("content", [])
        if not content:
            raise RuntimeError("absorb 返回空内容")
        return parse_absorb_text(content[0].get("text", ""))


def parse_absorb_text(text: str) -> dict:
    """解析 MCP absorb 的人类可读返回。

    形如：
      沉淀完成（host=eval source=auto_distill）
      候选 2 / 写入 0 / 跳过 2 / 失败 0
      （dry_run：未写库）
      - [would_write] ...
      - [dropped] 纯链接
    """
    outcome = {
        "candidates": 0,
        "written": 0,
        "skipped": 0,
        "failed": 0,
        "dry_run": "dry_run" in text,
        "items": [],
        "dropped": [],
    }
    m = re.search(r"候选 (\d+) / 写入 (\d+) / 跳过 (\d+) / 失败 (\d+)", text)
    if not m:
        raise RuntimeError(f"无法解析 absorb 返回：{text[:200]}")
    outcome.update(
        candidates=int(m.group(1)),
        written=int(m.group(2)),
        skipped=int(m.group(3)),
        failed=int(m.group(4)),
    )
    for line in text.splitlines():
        item = re.match(r"- \[([a-z_]+)\] (.*)", line.strip())
        if not item:
            continue
        reason, content = item.group(1), item.group(2)
        if reason == "dropped":
            outcome["dropped"].append(content)
        else:
            outcome["items"].append({"reason": reason, "content": content})
    return outcome


def load_cases(path: Path) -> list[dict]:
    cases = []
    for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        try:
            cases.append(json.loads(line))
        except json.JSONDecodeError as exc:
            raise SystemExit(f"{path.name}:{line_no} JSON 解析失败：{exc}")
    if not cases:
        raise SystemExit(f"{path} 没有用例")
    return cases


def contains_all(text: str, keywords: list[str]) -> bool:
    return all(k in text for k in keywords)


def contains_any(text: str, keywords: list[str]) -> bool:
    return any(k in text for k in keywords)


def evaluate_case(client: AbsorbClient, case: dict, run_id: str) -> dict:
    session_id = f"wq-{run_id}-{case['id']}"
    outcome = client.absorb(case["text"], session_id, dry_run=True)
    written_items = [x for x in outcome["items"] if x["reason"] in WRITE_REASONS]
    predicted = bool(written_items)
    expect = bool(case["expect_absorb"])

    expected_keywords = case.get("expect_keywords") or []
    forbid_keywords = case.get("forbid_keywords") or []
    combined = "\n".join(x["content"] for x in written_items)
    content_hit = contains_all(combined, expected_keywords) if expected_keywords else predicted
    contaminated = contains_any(combined, forbid_keywords) if forbid_keywords else False

    return {
        "id": case["id"],
        "category": case["category"],
        "expect_absorb": expect,
        "predicted_absorb": predicted,
        "correct": predicted == expect,
        "content_hit": content_hit,
        "contaminated": contaminated,
        "candidates": outcome["candidates"],
        "written_items": written_items,
        "dropped": outcome["dropped"],
    }


def duplicate_probe(client: AbsorbClient, case: dict, run_id: str) -> dict:
    """同一结论连续沉淀两次：第二次应被去重。"""
    session_id = f"dup-{run_id}-{case['id']}"
    first = client.absorb(case["text"], session_id, dry_run=False)
    second = client.absorb(case["text"], session_id, dry_run=False)
    first_written = first["written"]
    second_written = second["written"]
    second_reasons = [x["reason"] for x in second["items"]]
    return {
        "id": case["id"],
        "first_written": first_written,
        "second_written": second_written,
        "second_reasons": second_reasons,
        "idempotent": first_written > 0 and second_written == 0,
        "applicable": first_written > 0,
    }


def summarize(case_results: list[dict], dup_results: list[dict]) -> dict:
    positives = [x for x in case_results if x["expect_absorb"]]
    negatives = [x for x in case_results if not x["expect_absorb"]]
    tp = sum(1 for x in positives if x["predicted_absorb"])
    fn = len(positives) - tp
    fp = sum(1 for x in negatives if x["predicted_absorb"])
    tn = len(negatives) - fp

    precision = tp / (tp + fp) if tp + fp else 0.0
    recall = tp / (tp + fn) if tp + fn else 0.0
    f1 = 2 * precision * recall / (precision + recall) if precision + recall else 0.0
    specificity = tn / (tn + fp) if tn + fp else 0.0
    misabsorb_rate = fp / len(negatives) if negatives else 0.0
    content_hits = sum(1 for x in positives if x["predicted_absorb"] and x["content_hit"])
    content_hit_rate = content_hits / tp if tp else 0.0
    contaminated = sum(1 for x in case_results if x["contaminated"])

    applicable = [x for x in dup_results if x["applicable"]]
    idempotent = sum(1 for x in applicable if x["idempotent"])
    idempotency_rate = idempotent / len(applicable) if applicable else 0.0

    return {
        "n_cases": len(case_results),
        "n_positive": len(positives),
        "n_negative": len(negatives),
        "tp": tp,
        "fn": fn,
        "fp": fp,
        "tn": tn,
        "precision": round(precision, 4),
        "recall": round(recall, 4),
        "f1": round(f1, 4),
        "specificity": round(specificity, 4),
        "misabsorb_rate": round(misabsorb_rate, 4),
        "content_hit_rate": round(content_hit_rate, 4),
        "contaminated_cases": contaminated,
        "duplicate_probe_applicable": len(applicable),
        "duplicate_probe_idempotent": idempotent,
        "idempotency_rate": round(idempotency_rate, 4),
        "false_positive_ids": [x["id"] for x in negatives if x["predicted_absorb"]],
        "false_negative_ids": [x["id"] for x in positives if not x["predicted_absorb"]],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="LRC absorb 写质量评测")
    parser.add_argument("--exe", required=True, help="code-memory-server 可执行文件路径")
    parser.add_argument(
        "--cases",
        default=str(WRITE_QUALITY_ROOT / "cases" / "absorb_cases.jsonl"),
        help="用例文件（JSONL）",
    )
    parser.add_argument(
        "--out-dir",
        default=str(WRITE_QUALITY_ROOT / "results"),
        help="结果输出目录",
    )
    parser.add_argument("--skip-duplicate-probe", action="store_true", help="跳过真实写入的幂等探针")
    parser.add_argument("--data-dir", default="", help="自定义数据目录（默认临时目录，跑完删除）")
    args = parser.parse_args()

    exe = Path(args.exe).resolve()
    if not exe.is_file():
        raise SystemExit(f"sidecar 不存在：{exe}")

    cases = load_cases(Path(args.cases))
    run_id = time.strftime("%Y%m%d_%H%M%S")
    temp_data = None
    if args.data_dir:
        data_dir = Path(args.data_dir).resolve()
        data_dir.mkdir(parents=True, exist_ok=True)
    else:
        temp_data = tempfile.mkdtemp(prefix="lrc_write_quality_")
        data_dir = Path(temp_data)

    client = AbsorbClient(str(exe), str(data_dir))
    case_results: list[dict] = []
    dup_results: list[dict] = []
    try:
        client.start()
        for i, case in enumerate(cases, 1):
            result = evaluate_case(client, case, run_id)
            case_results.append(result)
            mark = "✓" if result["correct"] else "✗"
            print(f"[{i}/{len(cases)}] {mark} {result['id']} ({result['category']})", flush=True)

        if not args.skip_duplicate_probe:
            positives = [x for x in cases if x["expect_absorb"]]
            for i, case in enumerate(positives, 1):
                result = duplicate_probe(client, case, run_id)
                dup_results.append(result)
                mark = "✓" if result["idempotent"] else "✗"
                print(
                    f"[dup {i}/{len(positives)}] {mark} {result['id']} "
                    f"first={result['first_written']} second={result['second_written']}",
                    flush=True,
                )
    finally:
        client.stop()
        if temp_data:
            shutil.rmtree(temp_data, ignore_errors=True)

    metrics = summarize(case_results, dup_results)
    report = {
        "protocol": "lrc-write-quality-v1",
        "run_id": run_id,
        "exe": str(exe),
        "cases_file": str(Path(args.cases).resolve()),
        "duplicate_probe": not args.skip_duplicate_probe,
        "metrics": metrics,
        "cases": case_results,
        "duplicate_probe_results": dup_results,
        "caveats": [
            "用例为人工标注的 20 条最小集合，不代表全部真实会话分布。",
            "判定使用 absorb dry_run 的 would_write / dropped 结果，不依赖 LLM judge。",
            "重复沉淀探针会真实写库，但仅写入临时数据目录，跑完即删除。",
            "本评测不覆盖合成污染（synthesis）与长期错忆代价，需后续单独设计。",
        ],
    }

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    out_file = out_dir / f"write_quality_{run_id}.json"
    out_file.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")

    print("\n=== absorb 写质量 ===")
    print(f"用例 {metrics['n_cases']}（正例 {metrics['n_positive']} / 负例 {metrics['n_negative']}）")
    print(f"精确率 {metrics['precision']:.2%}  召回率 {metrics['recall']:.2%}  F1 {metrics['f1']:.4f}")
    print(f"特异度 {metrics['specificity']:.2%}  误吸收率 {metrics['misabsorb_rate']:.2%}  内容命中率 {metrics['content_hit_rate']:.2%}")
    print(
        f"重复沉淀幂等率 {metrics['idempotency_rate']:.2%}"
        f"（{metrics['duplicate_probe_idempotent']}/{metrics['duplicate_probe_applicable']}）"
    )
    if metrics["false_positive_ids"]:
        print("误吸收：" + ", ".join(metrics["false_positive_ids"]))
    if metrics["false_negative_ids"]:
        print("漏吸收：" + ", ".join(metrics["false_negative_ids"]))
    print(f"报告：{out_file}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
