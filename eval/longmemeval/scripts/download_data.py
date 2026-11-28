"""下载 LongMemEval 数据集（HF-Mirror，国内直连）。

数据来源：HuggingFace `xiaowu0162/longmemeval-cleaned`（镜像 hf-mirror.com）。
- `longmemeval_s_cleaned.json`：完整 S 集（约 277MB），评测默认使用；
- `longmemeval_oracle.json`：oracle 子集（约 15MB），仓库内已有一份副本。

输出目录：`eval/longmemeval/data/`（脚本相对路径，不依赖本机绝对路径）。
下载后自动校验 SHA256；不匹配直接失败，避免用错数据集复现出不同数字。
"""

import hashlib
import json
import os
import sys
import urllib.request

MIRROR = "https://hf-mirror.com"
REPO = "datasets/xiaowu0162/longmemeval-cleaned/resolve/main"
OUT_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "data"))

FILES = {
    "longmemeval_s_cleaned.json": "d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442",
    "longmemeval_oracle.json": "821a2034d219ab45846873dd14c14f12cfe7776e73527a483f9dac095d38620c",
}


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> int:
    os.makedirs(OUT_DIR, exist_ok=True)
    for fname, expected in FILES.items():
        out_path = os.path.join(OUT_DIR, fname)
        if os.path.isfile(out_path):
            actual = sha256_file(out_path)
            if actual == expected:
                print(f"[skip] {fname} 已存在且 SHA256 一致")
                continue
            print(f"[redo] {fname} 已存在但 SHA256 不一致，重新下载")

        url = f"{MIRROR}/{REPO}/{fname}"
        print(f"下载 {fname} ...")
        try:
            urllib.request.urlretrieve(url, out_path)
        except Exception as exc:  # noqa: BLE001 - CLI 需要明确失败原因
            print(f"  -> 失败: {exc}")
            return 1

        actual = sha256_file(out_path)
        size_mb = os.path.getsize(out_path) / (1024 * 1024)
        if actual != expected:
            print(f"  -> SHA256 不匹配\n     期望: {expected}\n     实际: {actual}")
            return 1
        print(f"  -> 完成 ({size_mb:.1f} MB), SHA256={actual}")

    s_path = os.path.join(OUT_DIR, "longmemeval_s_cleaned.json")
    with open(s_path, "r", encoding="utf-8") as f:
        data = json.load(f)
    print(f"\n成功加载 {len(data)} 条数据")
    return 0


if __name__ == "__main__":
    sys.exit(main())
