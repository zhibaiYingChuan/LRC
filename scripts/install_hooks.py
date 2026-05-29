#!/usr/bin/env python3
"""
Loong Recall (L-RC / 忆) - 预提交钩子安装脚本

将 scripts/pre-commit 安装到 .git/hooks/ 目录。
每次 git commit 前自动运行泄露检测。

用法:
  python scripts/install_hooks.py
"""

import os
import shutil
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent
HOOK_SOURCE = PROJECT_ROOT / "scripts" / "pre-commit"
HOOK_DEST = PROJECT_ROOT / ".git" / "hooks" / "pre-commit"


def main():
    print("=" * 60)
    print("  Loong Recall (L-RC / 忆) - 钩子安装")
    print("=" * 60)

    if not HOOK_SOURCE.exists():
        print(f"  错误: 找不到钩子源文件 {HOOK_SOURCE}")
        return 1

    hooks_dir = HOOK_DEST.parent
    if not hooks_dir.exists():
        print(f"  错误: .git/hooks 目录不存在，请先初始化 Git 仓库")
        return 1

    shutil.copy2(HOOK_SOURCE, HOOK_DEST)

    # 设置可执行权限 (Unix)
    if sys.platform != "win32":
        os.chmod(HOOK_DEST, 0o755)

    print(f"  已将预提交钩子安装到: {HOOK_DEST}")
    print()
    print("  检查内容:")
    print("    1. cargo check --features server  (编译检查)")
    print("    2. cargo test                      (单元测试)")
    print("    3. python scripts/check_algorithm_leak.py (泄露检测)")
    print()
    print("  安装完成！每次 git commit 前将自动执行上述检查。")
    return 0


if __name__ == "__main__":
    sys.exit(main())