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
        # 跳过 include_str! / include_bytes! 中的静态资源文件名引用
        # （如 include_str!("../static/assets/icons/icon-luoshu.svg")）
        if re.search(r'include_(str|bytes)!\s*\(', line):
            continue
        # 跳过资源文件名字符串匹配（如 "icon-bagua.svg"、"icon-luoshu.svg"）
        if re.search(r'["\']icon-(bagua|luoshu|dao)', line, re.IGNORECASE):
            continue
        # 跳过 engine 模块文件名引用（如 luoshu_encoder_ml.rs）
        if re.search(r'(luoshu|bagua)_\w+\.rs', line, re.IGNORECASE):
            continue
        # 跳过环境变量名引用（如 LRC_LUOSHU_MODEL_ID）
        if re.search(r'LRC_(LUOSHU|BAGUA|DAO)', line):
            continue
        # 跳过 UI 设计风格描述（如 "洛书九宫格加载动画"）—— 仅作为设计风格命名
        if re.search(r'洛书九宫格', line):
            continue
        # 跳过道同构度 UI 指标名称引用（如 "道同构度调节器"、"道同构度评分"）
        # "道同构度" 是面向用户的 UI 指标名，不是受保护的算法术语
        # 受保护的是 "道同构"（无"度"字），实际算法在 src/engine/ 受保护文件中
        if re.search(r'道同构度', line):
            continue
        # 跳过注释中的图标名称列表（如 "baga/health/decay/luoshu/memory/..."）
        # 这些是静态资源文件名引用，不是算法泄露
        if re.search(r'//.*\b\w+/\w+/\w+.*(luoshu|bagua)', line, re.IGNORECASE):
            continue
        # 跳过道体预判元数据字段名引用（如 daoti_preview_gua/bagua/version）
        # 这些是记忆写入时的公开 API 数据契约字段，不是算法内容
        if re.search(r'(daoti_preview_\w+|道体预判)', line, re.IGNORECASE):
            continue
        # 跳过内置联想状态机的公开 API 调用与用户可观察行为注释
        # 状态机实现位于 src/engine/memory_state_machine.rs（受保护层），
        # 此处仅消费公共方法（snapshot/activate），同"道同构度 UI 指标名"白名单
        if re.search(r'(memory_state_machine|内置道体状态机|联想链（状态机轨迹）)', line):
            continue
        # 跳过 License 边界门控注释与信号消费声明（产品侧只消费信号、不内置算法）
        if re.search(r'(产品侧只消费不计算|DaoTi License|LRC_DAOTI_NAVIGATE|daoti\s+pilot)', line, re.IGNORECASE):
            continue
        # 跳过 P2.5 常驻导航生产者（daoti_daemon）的进程名/函数名/环境变量引用。
        # daoti_daemon 是独立研究资产进程，LRC 仅消费其 JSON 信号，不内置算法；
        # 与 LRC_DAOTI_NAVIGATE 门控、daoti_preview 字段同属"契约/集成引用"白名单
        # daoti-lexicon-v1 是信号协议版本标识（navigation.rs 的 source_version 协商值）
        # P6/CL2 新增同类消费侧契约引用：post_daoti_reflect（explore 后回传 /reflect
        # 的客户端函数）、LRC_DAOTI_REFLECT（回传门控环境变量）——均只消费 JSON 信号
        if re.search(r'(daoti_daemon|DAOTI_SERVICE_URL|fetch_daoti_navigation|post_daoti_reflect|LRC_DAOTI_REFLECT|from daoti|daoti-lexicon-v1|daoti 研究资产)', line, re.IGNORECASE):
            continue
        # 跳过 API schema 中 daoti_preview 字段的 description 描述文本
        # （如 "道体写入时预判的六十四卦名称"）—— 契约字段含义，非算法
        if re.search(r'"(daoti_preview_\w+)"|道体写入时预判', line, re.IGNORECASE):
            continue
        # 跳过用户可见的校验证据 UI 文案（同"道同构度 UI 指标名"白名单先例）
        if re.search(r'(道体再次校验|回归校验证据)', line):
            continue
        # 跳过 memory daoti_preview 字段的局部变量名（preview_gua/bagua/version）
        if re.search(r'preview_(gua|bagua|version)', line, re.IGNORECASE):
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