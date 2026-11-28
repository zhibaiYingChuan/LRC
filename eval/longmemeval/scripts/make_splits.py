"""从 LongMemEval-S 生成可审计的分片清单。

用法：
  python eval/longmemeval/scripts/make_splits.py \
    --data eval/longmemeval/data/longmemeval_s_cleaned.json \
    --max 250 --n-shards 3 \
    --out eval/longmemeval/splits/fast_232.json

输出包含：数据集 SHA256、前 N 条实例、abstention 标记、分片编号、题型分布。
评测脚本本身按 `index % n_shards` 分片，本清单用于第三方核对“到底跑了哪 232 条”。
"""

import argparse
import hashlib
import json
import os
from collections import Counter


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", required=True)
    parser.add_argument("--max", type=int, default=250)
    parser.add_argument("--n-shards", type=int, default=3)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    with open(args.data, "r", encoding="utf-8") as f:
        dataset = json.load(f)

    picked = dataset[: args.max]
    instances = []
    for i, inst in enumerate(picked):
        qid = inst["question_id"]
        instances.append(
            {
                "index": i,
                "shard": i % args.n_shards,
                "question_id": qid,
                "question_type": inst.get("question_type", "unknown"),
                "abstention": "_abs" in qid,
            }
        )

    evaluated = [x for x in instances if not x["abstention"]]
    types = Counter(x["question_type"] for x in evaluated)
    manifest = {
        "dataset": os.path.basename(args.data),
        "dataset_sha256": sha256_file(args.data),
        "max_instances": args.max,
        "n_shards": args.n_shards,
        "total_selected": len(instances),
        "abstentions": len(instances) - len(evaluated),
        "evaluated": len(evaluated),
        "question_types": dict(sorted(types.items())),
        "instances": instances,
    }

    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=2)

    print(
        f"写入 {args.out}: total={len(instances)} evaluated={len(evaluated)} "
        f"abstentions={manifest['abstentions']}"
    )
    print("题型分布:", dict(sorted(types.items())))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
