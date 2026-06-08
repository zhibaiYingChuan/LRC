"""
LRC L2 层独立贡献量化测试 (Ablation Study)
===============================================
目标：量化 L2 层（Luoshu 几何检索 + TrapezoidFocus 梯形聚焦）的独立贡献。

测试三种模式：
  - L1-only:   recall()   → 纯 TF-IDF 关键词匹配
  - L2-only:   recall(lrc_mode="luoshu")  → 纯洛书几何检索
  - L1+L2+RRF: recall_enhanced()  → 双路检索 + RRF 融合

量化指标：
  - 召回精度 (Recall@K): 正确答案是否出现在召回结果中
  - L2 独立增益: L1+L2+RRF 优于 max(L1, L2) 的实例比例
  - L2 互补发现: L2 发现但 L1 未发现的答案比例
  - RRF 融合效果: 融合后是否优于单独的最好通路

使用方法：
  1. 启动 LRC 服务端（LLM 增强模式）
  2. python lrc_l2_ablation.py --data data/longmemeval_s_cleaned.json --output results/l2_ablation.json --max 20
  3. 查看生成的对比报告
"""

import json
import os
import sys
import time
import argparse
import urllib.request
from collections import Counter, defaultdict
from typing import Optional

# ============================================================
# LRC 服务端 HTTP 客户端（精简版，复用 lrc_full_longmemeval.py 的核心逻辑）
# ============================================================

class LRCServerClient:
    """通过 HTTP MCP API 调用真实 LRC 服务端"""

    def __init__(self, base_url: str = "http://localhost:3099"):
        self.base_url = base_url
        self._req_id = 0

    def _call_tool(self, tool_name: str, arguments: dict) -> dict:
        self._req_id += 1
        payload = {
            "jsonrpc": "2.0",
            "id": self._req_id,
            "method": "tools/call",
            "params": {"name": tool_name, "arguments": arguments},
        }
        data = json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(
            f"{self.base_url}/mcp", data=data,
            headers={"Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(req, timeout=120) as resp:
                return json.loads(resp.read().decode("utf-8"))
        except Exception as e:
            print(f"  LRC API 调用失败 ({tool_name}): {e}")
            return {"error": str(e)}

    def remember_batch(self, memories: list[dict]) -> dict:
        return self._call_tool("batch_remember", {"memories": memories})

    def recall_l1(self, query: str, top_k: int = 10, project: Optional[str] = None) -> list[dict]:
        """L1 层召回：纯 TF-IDF 关键词匹配"""
        return self._recall(query, top_k, "conversation", project)

    def recall_l2(self, query: str, top_k: int = 10, project: Optional[str] = None) -> list[dict]:
        """L2 层召回：纯洛书几何检索（TrapezoidFocus）"""
        return self._recall(query, top_k, "conversation", project, lrc_mode="luoshu")

    def recall_enhanced(self, query: str, top_k: int = 10, project: Optional[str] = None) -> list[dict]:
        """L1 + L2 + RRF 双路检索融合"""
        args = {"query": query, "top_k": top_k, "memory_type": "conversation"}
        if project:
            args["project"] = project
        result = self._call_tool("recall_enhanced", args)
        if "error" in result:
            return []
        content_list = result.get("result", {}).get("content", [])
        if not content_list:
            return []
        return self._parse_recall_text(content_list[0].get("text", ""))

    def _recall(self, query: str, top_k: int, memory_type: str, project: Optional[str] = None, lrc_mode: Optional[str] = None) -> list[dict]:
        """基础召回（支持切換 L1/L2 模式）"""
        args = {"query": query, "top_k": top_k, "memory_type": memory_type}
        if project:
            args["project"] = project
        if lrc_mode:
            args["lrc_mode"] = lrc_mode
        result = self._call_tool("recall", args)
        if "error" in result:
            return []
        content_list = result.get("result", {}).get("content", [])
        if not content_list:
            return []
        return self._parse_recall_text(content_list[0].get("text", ""))

    def _parse_recall_text(self, text: str) -> list[dict]:
        import re
        memories = []
        parts = re.split(r"（记忆 #\d+", text)
        for part in parts[1:]:
            content_match = re.search(r"内容:\s*(.+?)(?:\n(?:八卦|类型|ID|RRF 融合度|相似度|得分|标签)|$)", part)
            if content_match:
                content = content_match.group(1).strip()
                score_match = re.search(r"RRF 融合度\s*([\d.]+)", part)
                if not score_match:
                    score_match = re.search(r"相似度\s*([\d.]+)", part)
                if not score_match:
                    score_match = re.search(r"得分\s*([\d.]+)", part)
                score = float(score_match.group(1)) if score_match else 0.0
                memories.append({"content": content, "score": score})
        return memories

    def health_check(self) -> bool:
        try:
            req = urllib.request.Request(f"{self.base_url}/health")
            with urllib.request.urlopen(req, timeout=5) as resp:
                return resp.status == 200
        except Exception:
            return False


# ============================================================
# 会话注入器
# ============================================================

class SessionInjector:
    def __init__(self, client: LRCServerClient, project: str):
        self.client = client
        self.project = project

    def inject(self, haystack_sessions: list, haystack_dates: list, haystack_session_ids: list) -> int:
        batch_memories = []
        for session, date_str, sid in zip(haystack_sessions, haystack_dates, haystack_session_ids):
            parts = [f"[{t.get('role', 'unknown')}]: {t.get('content', '')}" for t in session]
            text = "\n".join(parts)
            if len(text) > 6000:
                text = text[:6000] + "\n...[截断]"
            batch_memories.append({
                "content": text, "memory_type": "conversation",
                "project": self.project, "tags": [sid, f"date:{date_str}"], "importance": 5,
            })

        for i in range(0, len(batch_memories), 20):
            chunk = batch_memories[i:i + 20]
            result = self.client.remember_batch(chunk)
            if "error" in result:
                print(f"  批量注入失败 (批次 {i//20 + 1}): {result['error']}")
        return len(batch_memories)


# ============================================================
# 召回结果评估
# ============================================================

def check_answer_in_results(answer: str, results: list[dict], top_k: int = 10) -> dict:
    """检查正确答案是否出现在召回结果中"""
    if not results:
        return {"hit": False, "position": -1, "top_score": 0.0, "result_count": 0}

    answer_lower = answer.lower().strip()
    for i, item in enumerate(results[:top_k]):
        content = item.get("content", "").lower()
        # 模糊匹配：答案的关键词是否在召回内容中
        answer_words = set(answer_lower.split())
        content_words = set(content.split())
        overlap = len(answer_words & content_words)
        if overlap >= max(1, len(answer_words) * 0.5):
            return {"hit": True, "position": i + 1, "top_score": item.get("score", 0.0), "result_count": len(results)}

    return {"hit": False, "position": -1, "top_score": results[0].get("score", 0.0) if results else 0.0, "result_count": len(results)}


# ============================================================
# LLM 查询翻译器（精简版）
# ============================================================

class QueryTranslator:
    def __init__(self, api_key: str, api_base: str, model: str = "deepseek-chat"):
        self.api_key = api_key
        self.api_base = api_base
        self.model = model

    def translate(self, query: str) -> str:
        if not self.api_key:
            return query
        data = json.dumps({
            "model": self.model,
            "messages": [{"role": "user", "content": (
                "你是一个记忆检索助手。将用户的自然语言问题翻译成可能在对话历史中出现的关键词。"
                "提取核心实体（人名、地名、事物名）、关键动作。只返回关键词，用空格分隔。"
                f"\n\n用户问题：{query}"
            )}],
            "max_tokens": 80,
            "temperature": 0.0,
        }).encode("utf-8")

        req = urllib.request.Request(
            f"{self.api_base}/chat/completions", data=data,
            headers={"Content-Type": "application/json", "Authorization": f"Bearer {self.api_key}"},
        )
        try:
            with urllib.request.urlopen(req, timeout=10) as resp:
                result = json.loads(resp.read().decode("utf-8"))
                return result["choices"][0]["message"]["content"].strip()
        except Exception:
            return query


# ============================================================
# 主运行流程：Ablation Study
# ============================================================

def run_ablation(
    data_path: str,
    output_path: str,
    max_instances: Optional[int] = None,
    api_key: Optional[str] = None,
    api_base: Optional[str] = None,
    model: str = "deepseek-chat",
    top_k: int = 10,
    lrc_url: str = "http://localhost:3099",
):
    client = LRCServerClient(lrc_url)
    if not client.health_check():
        print(f"错误: LRC 服务端不可用 ({lrc_url})")
        sys.exit(1)
    print(f"LRC 服务端连接成功: {lrc_url}")

    with open(data_path, "r", encoding="utf-8") as f:
        dataset = json.load(f)
    print(f"加载数据集: {len(dataset)} 条实例")

    if max_instances:
        dataset = dataset[:max_instances]

    translator = QueryTranslator(api_key, api_base, model)

    # 统计收集
    stats = {
        "total": len(dataset),
        "top_k": top_k,
        "l1": {"hits": 0, "positions": [], "scores": [], "recall_times": [], "total_results": 0},
        "l2": {"hits": 0, "positions": [], "scores": [], "recall_times": [], "total_results": 0},
        "l1l2_rrf": {"hits": 0, "positions": [], "scores": [], "recall_times": [], "total_results": 0},
        "instances": [],  # 逐实例详情
    }

    # 分组统计
    per_type = defaultdict(lambda: {
        "l1_hits": 0, "l2_hits": 0, "rrf_hits": 0, "count": 0,
        "l2_only_hits": 0,  # L2 命中但 L1 未命中
        "rrf_only_hits": 0,  # RRF 命中但 L1/L2 都未命中
    })

    print(f"\n{'='*60}")
    print(f"LRC L2 层独立贡献 Ablation 测试")
    print(f"{'='*60}")
    print(f"  实例数: {len(dataset)}")
    print(f"  召回数: top_{top_k}")
    print(f"  测试模式: L1-only | L2-only | L1+L2+RRF")
    print()

    for idx, instance in enumerate(dataset):
        question_id = instance["question_id"]
        question_type = instance["question_type"]
        question = instance["question"]
        answer = str(instance.get("answer", ""))
        haystack_sessions = instance.get("haystack_sessions", [])
        haystack_dates = instance.get("haystack_dates", [])
        haystack_session_ids = instance.get("haystack_session_ids", [])

        print(f"\n[{idx+1}/{len(dataset)}] {question_id} ({question_type})")
        print(f"  问题: {question[:80]}...")
        print(f"  答案: {answer[:80]}...")
        print(f"  会话数: {len(haystack_sessions)}")

        # 1. 注入记忆
        project = f"ablation_{question_id}"
        injector = SessionInjector(client, project)
        t0 = time.time()
        injector.inject(haystack_sessions, haystack_dates, haystack_session_ids)
        inject_time = time.time() - t0
        print(f"  注入耗时: {inject_time:.1f}s")

        # 2. 翻译查询
        translated = translator.translate(question)
        query = translated if translated else question

        # 3. 三种模式召回
        instance_result = {
            "question_id": question_id,
            "question_type": question_type,
            "question": question,
            "answer": answer,
            "num_sessions": len(haystack_sessions),
            "inject_time": round(inject_time, 2),
            "modes": {},
        }

        # L1-only
        t0 = time.time()
        l1_results = client.recall_l1(query, top_k, project)
        l1_time = time.time() - t0
        l1_eval = check_answer_in_results(answer, l1_results, top_k)
        instance_result["modes"]["l1"] = {
            "time": round(l1_time, 2),
            "hit": l1_eval["hit"],
            "position": l1_eval["position"],
            "score": round(l1_eval["top_score"], 4),
            "num_results": l1_eval["result_count"],
        }
        stats["l1"]["hits"] += l1_eval["hit"]
        stats["l1"]["positions"].append(l1_eval["position"])
        stats["l1"]["scores"].append(l1_eval["top_score"])
        stats["l1"]["recall_times"].append(l1_time)
        stats["l1"]["total_results"] += l1_eval["result_count"]

        # L2-only
        t0 = time.time()
        l2_results = client.recall_l2(query, top_k, project)
        l2_time = time.time() - t0
        l2_eval = check_answer_in_results(answer, l2_results, top_k)
        instance_result["modes"]["l2"] = {
            "time": round(l2_time, 2),
            "hit": l2_eval["hit"],
            "position": l2_eval["position"],
            "score": round(l2_eval["top_score"], 4),
            "num_results": l2_eval["result_count"],
        }
        stats["l2"]["hits"] += l2_eval["hit"]
        stats["l2"]["positions"].append(l2_eval["position"])
        stats["l2"]["scores"].append(l2_eval["top_score"])
        stats["l2"]["recall_times"].append(l2_time)
        stats["l2"]["total_results"] += l2_eval["result_count"]

        # L1+L2+RRF
        t0 = time.time()
        rrf_results = client.recall_enhanced(query, top_k, project)
        rrf_time = time.time() - t0
        rrf_eval = check_answer_in_results(answer, rrf_results, top_k)
        instance_result["modes"]["l1l2_rrf"] = {
            "time": round(rrf_time, 2),
            "hit": rrf_eval["hit"],
            "position": rrf_eval["position"],
            "score": round(rrf_eval["top_score"], 4),
            "num_results": rrf_eval["result_count"],
        }
        stats["l1l2_rrf"]["hits"] += rrf_eval["hit"]
        stats["l1l2_rrf"]["positions"].append(rrf_eval["position"])
        stats["l1l2_rrf"]["scores"].append(rrf_eval["top_score"])
        stats["l1l2_rrf"]["recall_times"].append(rrf_time)
        stats["l1l2_rrf"]["total_results"] += rrf_eval["result_count"]

        # 4. 记录交叉分析
        instance_result["cross_analysis"] = {
            "l1_only_hit": l1_eval["hit"] and not l2_eval["hit"],
            "l2_only_hit": l2_eval["hit"] and not l1_eval["hit"],
            "both_hit": l1_eval["hit"] and l2_eval["hit"],
            "neither_hit": not l1_eval["hit"] and not l2_eval["hit"],
            "rrf_better_than_both": rrf_eval["hit"] and not l1_eval["hit"] and not l2_eval["hit"],
            "rrf_worse_than_best": (l1_eval["hit"] or l2_eval["hit"]) and not rrf_eval["hit"],
        }

        stats["instances"].append(instance_result)

        # 打印实例结果
        l1_status = "HIT" if l1_eval["hit"] else "MISS"
        l2_status = "HIT" if l2_eval["hit"] else "MISS"
        rrf_status = "HIT" if rrf_eval["hit"] else "MISS"
        print(f"  L1(TF-IDF): {l1_status} | L2(Luoshu): {l2_status} | RRF: {rrf_status}")

        # 分组统计
        pt = per_type[question_type]
        pt["count"] += 1
        pt["l1_hits"] += l1_eval["hit"]
        pt["l2_hits"] += l2_eval["hit"]
        pt["rrf_hits"] += rrf_eval["hit"]
        if l2_eval["hit"] and not l1_eval["hit"]:
            pt["l2_only_hits"] += 1
        if rrf_eval["hit"] and not l1_eval["hit"] and not l2_eval["hit"]:
            pt["rrf_only_hits"] += 1

        # 增量保存
        os.makedirs(os.path.dirname(output_path) or ".", exist_ok=True)
        with open(output_path, "w", encoding="utf-8") as f:
            json.dump(stats, f, ensure_ascii=False, indent=2)

    # ============================================================
    # 生成报告
    # ============================================================
    total = stats["total"]
    n = max(total, 1)

    l1_hit_rate = stats["l1"]["hits"] / n * 100
    l2_hit_rate = stats["l2"]["hits"] / n * 100
    rrf_hit_rate = stats["l1l2_rrf"]["hits"] / n * 100

    avg_l1_pos = sum(p for p in stats["l1"]["positions"] if p > 0) / max(stats["l1"]["hits"], 1)
    avg_l2_pos = sum(p for p in stats["l2"]["positions"] if p > 0) / max(stats["l2"]["hits"], 1)
    avg_rrf_pos = sum(p for p in stats["l1l2_rrf"]["positions"] if p > 0) / max(stats["l1l2_rrf"]["hits"], 1)

    avg_l1_time = sum(stats["l1"]["recall_times"]) / n
    avg_l2_time = sum(stats["l2"]["recall_times"]) / n
    avg_rrf_time = sum(stats["l1l2_rrf"]["recall_times"]) / n

    # L2 独立贡献
    l2_only_count = sum(1 for i in stats["instances"] if i["cross_analysis"]["l2_only_hit"])
    rrf_only_count = sum(1 for i in stats["instances"] if i["cross_analysis"]["rrf_better_than_both"])
    both_hit_count = sum(1 for i in stats["instances"] if i["cross_analysis"]["both_hit"])

    report = f"""
{'='*60}
LRC L2 层独立贡献 Ablation 测试报告
{'='*60}

## 总体召回命中率 (Recall@{top_k})

| 模式 | 命中数 | 命中率 | 平均位置 | 平均耗时 |
|------|--------|--------|----------|----------|
| L1-only (TF-IDF) | {stats['l1']['hits']}/{total} | {l1_hit_rate:.1f}% | {avg_l1_pos:.1f} | {avg_l1_time:.2f}s |
| L2-only (Luoshu) | {stats['l2']['hits']}/{total} | {l2_hit_rate:.1f}% | {avg_l2_pos:.1f} | {avg_l2_time:.2f}s |
| L1+L2+RRF       | {stats['l1l2_rrf']['hits']}/{total} | {rrf_hit_rate:.1f}% | {avg_rrf_pos:.1f} | {avg_rrf_time:.2f}s |

## L2 层独立贡献分析

| 指标 | 数量 | 占比 |
|------|------|------|
| L2 唯一命中（L1未命中但L2命中） | {l2_only_count} | {l2_only_count/n*100:.1f}% |
| RRF 唯一命中（L1/L2均未命中但RRF命中） | {rrf_only_count} | {rrf_only_count/n*100:.1f}% |
| 双路同时命中 | {both_hit_count} | {both_hit_count/n*100:.1f}% |

**L2 独立贡献 = {l2_only_count/n*100:.1f}%**（L2 发现而 L1 未发现的实例比例）

**RRF 融合增益 = {rrf_only_count/n*100:.1f}%**（融合后超越单独最好通路的实例比例）

## 按问题类型分组

| 类型 | 实例数 | L1 命中率 | L2 命中率 | RRF 命中率 | L2 唯一命中 |
|------|--------|-----------|-----------|------------|-------------|
"""

    for qtype, pt in sorted(per_type.items()):
        c = max(pt["count"], 1)
        report += f"| {qtype} | {pt['count']} | {pt['l1_hits']/c*100:.1f}% | {pt['l2_hits']/c*100:.1f}% | {pt['rrf_hits']/c*100:.1f}% | {pt['l2_only_hits']/c*100:.1f}% |\n"

    report += f"""
## 结论

1. **L1 (TF-IDF) 基础召回率**: {l1_hit_rate:.1f}%
2. **L2 (洛书几何检索) 独立召回率**: {l2_hit_rate:.1f}%
3. **L1+L2+RRF 融合召回率**: {rrf_hit_rate:.1f}%
4. **L2 独立贡献**: {l2_only_count/n*100:.1f}%（{l2_only_count} 个实例中 L2 发现了 L1 遗漏的答案）
5. **RRF 融合增益**: {rrf_only_count/n*100:.1f}%（{rrf_only_count} 个实例中融合结果超越了最好单独通路）

{'='*60}
"""

    print(report)

    # 保存报告
    report_path = output_path.replace(".json", ".md")
    with open(report_path, "w", encoding="utf-8") as f:
        f.write(report)
    print(f"报告已保存: {report_path}")

    return stats


# ============================================================
# CLI
# ============================================================

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="LRC L2 层独立贡献 Ablation 测试")
    parser.add_argument("--data", default="data/longmemeval_s_cleaned.json", help="数据集路径")
    parser.add_argument("--output", default="results/l2_ablation.json", help="输出路径")
    parser.add_argument("--max", type=int, default=20, help="最大实例数")
    parser.add_argument("--top-k", type=int, default=10, help="召回数")
    parser.add_argument("--model", default="deepseek-chat")
    parser.add_argument("--api-key", default=None)
    parser.add_argument("--api-base", default=None)
    parser.add_argument("--lrc-url", default="http://localhost:3099")

    args = parser.parse_args()

    run_ablation(
        data_path=args.data,
        output_path=args.output,
        max_instances=args.max,
        api_key=args.api_key,
        api_base=args.api_base,
        model=args.model,
        top_k=args.top_k,
        lrc_url=args.lrc_url,
    )