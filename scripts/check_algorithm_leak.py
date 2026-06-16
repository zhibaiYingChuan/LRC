#!/usr/bin/env python3
"""
Loong Recall — 公开层核心算法泄露检测脚本

扫描 Apache 2.0 许可证覆盖的公开层文件，
检测是否包含 DaoTi Research License 保护的算法内容。
"""

import re
import sys
from pathlib import Path

# ─── 公开层文件（Apache 2.0 许可） ───
PUBLIC_FILES = [
    "src/chunker.rs",
    "src/server.rs",
    "src/bin/server.rs",
    "src/lib.rs",
]

# ─── 泄露检测规则 ───
# 每条规则：(名称, 正则模式, 严重级别: error|warn)
RULES = [
    # ── 道体/道枢哲学术语 ──
    ("道枢映射", r"道枢|道体|道同构|Dao[_\s]*(pivot|ti|isomorphism)|dao_evolution", "error"),
    # ── 八卦/洛书编码体系 ──
    ("八卦编码", r"乾卦|坤卦|震卦|巽卦|坎卦|离卦|艮卦|兑卦|八卦|Bagua|trigram", "error"),
    # ── 几何坐标空间 ──
    ("几何坐标空间", r"几何坐标|geometric[_\s]*coordinate|memory[_\s]*topology|拓扑演化", "error"),
    # ── 洛书/镜像梯形 ──
    ("洛书算法", r"luoshu|洛书|mirror[_\s]*trapezoid|镜像梯形", "error"),
    # ── ROI 剪枝 / 可逆组合 ──
    ("剪枝算法", r"ROI[_\s]*prun|剪枝|可逆组合|reversible[_\s]*composit", "error"),
    # ── 模型底层架构 ──
    ("底层架构变造", r"底层架构|underlying[_\s]*architecture|gauge[_\s]*field|规范场|退化基态", "error"),
    # ── 文档引用（指向受保护文档） ──
    ("受保护文档引用", r"dao-pivot-mapping|ALGORITHM_OVERVIEW|COMMUNITY_GOVERNANCE", "warn"),
    # ── 中英混合算法注释（≥3个中文术语） ──
    ("算法注释", r'(?://.*(?:编码|检索|算法|引擎|记忆|演化).*){3,}', "warn"),
]


def scan_file(filepath: Path, verbose: bool = False) -> list[dict]:
    """扫描单个文件，返回违规列表"""
    findings = []
    try:
        content = filepath.read_text(encoding="utf-8")
    except FileNotFoundError:
        return [{"file": str(filepath), "rule": "FILE_MISSING", "level": "warn",
                 "line": 0, "detail": "文件不存在，跳过检查"}]
    except Exception as e:
        return [{"file": str(filepath), "rule": "READ_ERROR", "level": "warn",
                 "line": 0, "detail": f"无法读取: {e}"}]

    lines = content.split("\n")
    for i, line in enumerate(lines, 1):
        # 跳过纯代码行中的结构体字段名（如 .bagua_entropy, bagua_category）
        # 这些是 engine 模块的数据结构，不是算法泄露
        if re.search(r'\.\s*(bagua_|dao_|luoshu_)', line, re.IGNORECASE):
            continue
        # 跳过模块重导出语句（如 pub use engine::luoshu_encoder...）
        if re.search(r'(pub\s+)?use\s+\S+::(luoshu_|bagua)', line, re.IGNORECASE):
            continue

        for rule_name, pattern, level in RULES:
            match = re.search(pattern, line, re.IGNORECASE)
            if match:
                findings.append({
                    "file": str(filepath),
                    "rule": rule_name,
                    "level": level,
                    "line": i,
                    "match": match.group(),
                    "context": line.strip()[:120],
                })

    if verbose and not findings:
        print(f"  {filepath}: 干净")

    return findings


def main():
    repo_root = Path(__file__).resolve().parent.parent
    verbose = "--verbose" in sys.argv or "-v" in sys.argv

    all_findings = []
    errors = 0
    warnings = 0

    for rel_path in PUBLIC_FILES:
        filepath = repo_root / rel_path
        findings = scan_file(filepath, verbose=verbose)
        all_findings.extend(findings)

    # ─── 输出结果 ───
    if all_findings:
        for f in all_findings:
            tag = "[ERROR]" if f["level"] == "error" else "[WARN] "
            if f["rule"] in ("FILE_MISSING", "READ_ERROR"):
                print(f"  {tag} {f['file']}: {f['detail']}")
            else:
                print(f"  {tag} {f['rule']:12s} | {f['file']}:{f['line']} | 匹配: {f['match'][:40]}")
            if f["level"] == "error":
                errors += 1
            else:
                warnings += 1

    total = errors + warnings
    if total == 0:
        print("  通过: 公开层文件无核心算法泄露")
        return 0
    else:
        print(f"\n  检测结果: {errors} 错误, {warnings} 警告")
        if errors > 0:
            print("  公开层文件包含受保护的算法内容，请移除或移至 engine/ 模块。")
            return 1
        return 0


if __name__ == "__main__":
    sys.exit(main())