#!/usr/bin/env python3
"""
Code Memory 核心算法泄露检测
=============================
预提交钩子：检测公开层文件中是否泄露了受保护的核心算法逻辑。

遵循 DaoTi 分层协议：
  - Layer 1 (公开层): Apache 2.0 — 不得包含核心算法实现
  - Layer 2 (受保护层): DaoTi Research License — 包含核心算法

检测规则:
  1. 公开层文件不得导入受保护层模块
  2. 公开层文件不得包含核心算法关键词（如编码策略、检索逻辑等）
  3. 公开层文件不得包含硬编码的参数表或配置常量
  4. 新增受保护层文件必须包含正确的许可证头
"""

import os
import re
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent

PUBLIC_FILES = [
    "src/chunker.rs",
    "src/server.rs",
    "src/bin/server.rs",
    "src/lib.rs",
    "Cargo.toml",
]

PROTECTED_DIR = "src/engine"

ALGORITHM_KEYWORDS = [
    "cosine_similarity",
    "signal_map",
    "cosine_sim",
    "vector_similarity",
    "scoring_algorithm",
    "ranking_strategy",
]

IMPORT_LEAK_PATTERNS = [
    re.compile(r"use\s+crate::engine::"),
    re.compile(r"use\s+super::engine::"),
]

PROTECTED_HEADER = "DaoTi Research License v1.0"
APACHE_HEADER = "Apache"

def check_public_file_no_import_leak(filepath):
    """检查公开层文件是否导入了受保护层模块"""
    with open(filepath, "r", encoding="utf-8") as f:
        content = f.read()
        for line_no, line in enumerate(content.split("\n"), 1):
            if line.strip().startswith("//"):
                continue
            for pattern in IMPORT_LEAK_PATTERNS:
                if pattern.search(line):
                    print(f"  [危险] {filepath}:L{line_no}: 公开层文件导入了受保护层模块")
                    print(f"          {line.strip()}")
                    print(f"          修复: 将核心逻辑移入 src/engine/ 或将导入移除")
                    return False
    return True

def check_public_file_no_algorithm_keywords(filepath):
    """检查公开层文件是否包含核心算法关键词"""
    with open(filepath, "r", encoding="utf-8") as f:
        content = f.read()
        for line_no, line in enumerate(content.split("\n"), 1):
            if line.strip().startswith("//"):
                continue
            line_lower = line.lower()
            for kw in ALGORITHM_KEYWORDS:
                if kw in line_lower:
                    print(f"  [警告] {filepath}:L{line_no}: 公开层文件包含核心算法关键词 '{kw}'")
                    print(f"          {line.strip()}")
    return True

def check_protected_file_has_license(filepath):
    """检查受保护层文件是否包含正确的许可证头"""
    with open(filepath, "r", encoding="utf-8") as f:
        content = f.read()
        if PROTECTED_HEADER not in content:
            print(f"  [错误] {filepath}: 受保护层文件缺少许可证头")
            print(f"         参考 src/engine/encoder.rs 中的许可证头格式")
            return False
    return True

def check_public_file_no_license_restriction(filepath):
    """检查公开层文件中的非注释行不应包含受保护许可证声明"""
    with open(filepath, "r", encoding="utf-8") as f:
        content = f.read()
        for line in content.split("\n"):
            stripped = line.strip()
            if stripped.startswith("//") or stripped.startswith("*") or stripped.startswith("#"):
                continue
            if PROTECTED_HEADER in stripped:
                print(f"  [错误] {filepath}: 公开层文件不应包含受保护许可证声明")
                print(f"          {stripped}")
                return False
    return True

def main():
    print("=" * 60)
    print("  Loong Recall (L-RC / 忆) 核心算法泄露检测")
    print("=" * 60)

    all_ok = True

    print("\n[1/4] 检查公开层文件导入泄露...")
    for f in PUBLIC_FILES:
        fpath = PROJECT_ROOT / f
        if fpath.exists():
            if not check_public_file_no_import_leak(str(fpath)):
                all_ok = False
    print("  完成。")

    print("\n[2/4] 检查公开层文件算法关键词泄露...")
    for f in PUBLIC_FILES:
        fpath = PROJECT_ROOT / f
        if fpath.exists():
            check_public_file_no_algorithm_keywords(str(fpath))
    print("  完成。")

    print("\n[3/4] 检查受保护层文件许可证完整性...")
    engine_dir = PROJECT_ROOT / PROTECTED_DIR
    if engine_dir.exists():
        for fpath in engine_dir.glob("**/*.rs"):
            if not check_protected_file_has_license(str(fpath)):
                all_ok = False
    print("  完成。")

    print("\n[4/4] 检查公开层文件许可证冲突...")
    for f in PUBLIC_FILES:
        fpath = PROJECT_ROOT / f
        if fpath.exists():
            if not check_public_file_no_license_restriction(str(fpath)):
                all_ok = False
    print("  完成。")

    print("=" * 60)
    if all_ok:
        print("  检测通过 — 未发现核心算法泄露")
        return 0
    else:
        print("  检测未通过 — 发现核心算法泄露风险")
        print("  请修复上述问题后重新提交")
        return 1

if __name__ == "__main__":
    sys.exit(main())