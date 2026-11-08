#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""仓库卫生门禁（Repo Hygiene Gate）

目的：阻止「本地开发/测试文件」与「敏感信息」进入版本库。

背景（为什么需要这道门禁）：
  .gitignore 只能阻止**未跟踪**文件被加入，对**已经跟踪**的文件完全无效。
  历史上 docs/ 与 scripts/ 用黑名单逐条忽略，每新增一个本地文档就要补一条规则，
  漏补一次就会出现在 `git status` 里，看起来像「仓库又混进了开发文件」。
  v0.10.2 起 docs/ 与 scripts/ 改为白名单，但 tests/、integrations/、static/ 等
  目录仍无约束，且白名单本身也可能被误加（例如把含服务器地址的运维文档放行）。

本脚本做三件事：
  1. 路径白名单：docs/ 与 scripts/ 下只允许显式列出的交付文件。
  2. 路径黑名单：禁止备份、日志、临时目录、测试产物、本地营销/研究资产入库。
  3. 内容扫描：禁止服务器 IP、SSH 私钥路径、本机绝对路径、令牌前缀、私钥块。

用法：
  python scripts/check_repo_hygiene.py            # 检查
  python scripts/check_repo_hygiene.py --verbose  # 打印全部被检查文件

退出码：0 = 通过；1 = 发现违规。
"""

from __future__ import annotations

import fnmatch
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# ── 1. 白名单：docs/ 与 scripts/ 下允许入库的文件 ────────────────────────────
# 新增交付文件时必须在这里显式加一行 —— 这是有意的摩擦：
# 「要进仓库」必须是一个主动决定，而不是默认结果。
DOCS_ALLOW = {
    "docs/BENCHMARK_v0.10.3.md",
    "docs/OFFLINE_MODEL_GUIDE.md",
    "docs/USER_GUIDE.md",
    "docs/USE_CASES.md",
}

SCRIPTS_ALLOW = {
    "scripts/check_algorithm_leak.py",
    "scripts/check_repo_hygiene.py",
    "scripts/enable_git_hooks.ps1",
    "scripts/install.ps1",
    "scripts/install.sh",
    "scripts/validate_frontend_contract.js",
}

# ── 2. 黑名单：任何位置都不允许入库的路径模式 ────────────────────────────────
DENY_PATTERNS = [
    # 备份 / 日志 / 临时
    "*.bak", "*.log", "*.tmp", "*.orig", "*.rej",
    # 证书与私钥
    "*.p12", "*.pem", "*.key", "*.crt", "*.mobileprovision",
    # 测试与审计产物
    "test-results*", "*-test-output.*", "*-test-results.*",
    "*-regression-report.*", "*-report.*",
    "playwright-artifacts/*", "evidence/*",
    # 本地开发目录
    "temp/*", "tmp/*", "target/*", "target-dev/*", "target_v*/*",
    "node_modules/*", "*/node_modules/*",
    "marketing/*", "daoti/*", "competition/*",
    "external_tests/*", "desktop-test/*",
    # 根目录临时脚本（_ 或 - 开头）
    "_*", "-*",
    # 调试文档
    "debug-*.md",
]

# ── 3. 内容扫描：禁止出现在任何被跟踪文本文件里的字符串 ──────────────────────
# 这些是「一旦入库就等于泄露」的内容，不是风格问题。
DENY_CONTENT = [
    ("139.224.231.170", "生产服务器 IP"),
    ("aml_deploy_rebuild", "生产服务器 SSH 私钥文件名"),
    ("C:\\wwwroot", "生产服务器绝对路径"),
    ("C:\\Users\\Administrator\\.ssh", "本机 SSH 目录"),
    ("BEGIN RSA PRIVATE KEY", "RSA 私钥块"),
    ("BEGIN OPENSSH PRIVATE KEY", "OpenSSH 私钥块"),
    ("ghp_", "GitHub 个人访问令牌前缀"),
    ("gho_", "GitHub OAuth 令牌前缀"),
]

# 内容扫描跳过的文件：本脚本自身就是规则定义，必然包含这些字面量
SKIP_CONTENT_FILES = {"scripts/check_repo_hygiene.py"}

# 内容扫描跳过的二进制/大文件后缀
SKIP_CONTENT_SUFFIX = {
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".svg",
    ".ttf", ".otf", ".woff", ".woff2", ".pdf", ".zip", ".gz",
    ".exe", ".dll", ".so", ".dylib", ".wasm", ".onnx", ".pt",
    ".safetensors", ".lock",
}


def tracked_files() -> list[str]:
    """返回 git 跟踪的全部文件（相对仓库根，正斜杠）。"""
    out = subprocess.run(
        ["git", "-C", str(REPO_ROOT), "ls-files", "-z"],
        capture_output=True, check=True,
    ).stdout
    return [p for p in out.decode("utf-8", "replace").split("\0") if p]


def check_paths(files: list[str]) -> list[str]:
    problems: list[str] = []
    for f in files:
        if f.startswith("docs/") and f not in DOCS_ALLOW:
            problems.append(f"[路径] {f} —— docs/ 白名单外，本地文档不得入库")
            continue
        if f.startswith("scripts/") and f not in SCRIPTS_ALLOW:
            problems.append(f"[路径] {f} —— scripts/ 白名单外，本地脚本不得入库")
            continue
        for pat in DENY_PATTERNS:
            if fnmatch.fnmatch(f, pat) or fnmatch.fnmatch(f, f"*/{pat}"):
                problems.append(f"[路径] {f} —— 命中禁止模式 {pat}")
                break
    return problems


def check_content(files: list[str]) -> list[str]:
    problems: list[str] = []
    for f in files:
        if f in SKIP_CONTENT_FILES:
            continue
        if Path(f).suffix.lower() in SKIP_CONTENT_SUFFIX:
            continue
        path = REPO_ROOT / f
        if not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        for needle, why in DENY_CONTENT:
            if needle in text:
                problems.append(f"[内容] {f} —— 含{why}：{needle}")
    return problems


def main() -> int:
    verbose = "--verbose" in sys.argv
    files = tracked_files()
    if verbose:
        print(f"被跟踪文件：{len(files)} 个")

    problems = check_paths(files) + check_content(files)

    if not problems:
        print(f"仓库卫生检查通过（{len(files)} 个被跟踪文件）")
        return 0

    print("仓库卫生检查未通过：")
    for p in problems:
        print("  " + p)
    print()
    print("处置建议：")
    print("  · 本地开发/测试文件：git rm --cached <file>（保留本地文件），并确认 .gitignore 已覆盖")
    print("  · 需要交付的 docs/scripts 文件：在 scripts/check_repo_hygiene.py 的白名单里显式加一行")
    print("  · 敏感信息：立即 git rm --cached，并轮换已泄露的凭据")
    return 1


if __name__ == "__main__":
    sys.exit(main())
