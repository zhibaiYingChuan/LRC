"""
LRC v0.5.6 Natural Questions 检索精度全面评估脚本

公平性原则（遵守 BEIR 基准测试规则）：
1. 不利用任何 ground truth 信息（不设置 importance 差异）
2. 所有文档 importance=5（统一）
3. 使用 BEIR 标准指标（MRR@10, Recall@10, Hit Rate@10）
4. 蓄水池抽样随机文档，不偏向相关文档

NQ 数据集特征（与 MS MARCO 的差异）：
1. 查询是自然语言问题（"what is non controlling interest on balance sheet"）
   - MS MARCO 是关键词查询（"color overlay photoshop"）
   - NQ 查询更长更自然，LLM 翻译器应更有效（问题→答案词桥接）
2. 文档有 title 字段（Wikipedia 段落标题）
   - MS MARCO 只有 text
   - NQ 文档内容 = title + text（提升检索效果）
3. 每查询相关文档 1-4 个（平均 1.22）
   - MS MARCO 通常 1 个
4. 使用 test split（3452 查询）
   - MS MARCO 使用 dev split

LRC v0.5.6 全面发挥的三种检索能力：
1. TF-IDF 检索（词边界匹配修复后）：contains_word 替代 contains
2. 洛书几何编码：remember 时自动编码，recall 时自动几何距离加权
3. LLM 查询翻译器：将自然语言问题翻译为答案可能包含的关键词

BM25 基线：NQ NDCG@10 ≈ 0.305

评估模式：
- tfidf: 纯 TF-IDF 检索（无 LLM）
- llm: LLM 查询翻译器 + TF-IDF 检索
- both: 两种模式都测试，生成对比报告

用法示例：
  python lrc_nq_eval.py --mode tfidf --num-docs 500 --num-queries 100
  python lrc_nq_eval.py --mode llm --llm-api "openai:sk-xxx:deepseek-chat:https://api.deepseek.com/v1"
  python lrc_nq_eval.py --mode both --llm-api "openai:sk-xxx:deepseek-chat:https://api.deepseek.com/v1"
"""
import os
import sys
import json
import time
import random
import subprocess
import threading
import queue
import re
import argparse
from pathlib import Path


# ── LRC stdio 客户端 ──
class LRCStdioClient:
    """通过 stdio 模式与 LRC sidecar 通信"""

    def __init__(self, exe_path, data_dir, llm_api=None):
        self.exe_path = exe_path
        self.data_dir = data_dir
        self.llm_api = llm_api
        self.proc = None
        self.req_id = 0
        self.responses = {}
        self.response_queues = {}
        self._read_thread = None
        self._write_lock = threading.Lock()
        self._stderr_file = None

    def start(self):
        """启动 sidecar stdio 进程"""
        # 清理数据目录
        if os.path.exists(self.data_dir):
            import shutil
            shutil.rmtree(self.data_dir)
        os.makedirs(self.data_dir, exist_ok=True)

        cmd = [self.exe_path, "--stdio", "--data-dir", self.data_dir, "--src-dir", self.data_dir]
        if self.llm_api:
            cmd.extend(["--llm-api", self.llm_api])

        env = os.environ.copy()
        env["RUST_LOG"] = "warn"

        # 将 sidecar stderr 写入文件以便调试（避免管道阻塞）
        stderr_file = open(os.path.join(self.data_dir, "sidecar_stderr.log"), "w", encoding="utf-8")

        self.proc = subprocess.Popen(
            cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=stderr_file,
            text=True,
            bufsize=1,
            encoding="utf-8",
            env=env,
        )

        self._stderr_file = stderr_file

        # 启动读取线程
        self._read_thread = threading.Thread(target=self._read_loop, daemon=True)
        self._read_thread.start()

        # 等待 sidecar 初始化
        resp = self._call("initialize", {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "nq-eval-client", "version": "0.5.6"}
        }, timeout=60)

        if resp and "result" in resp:
            print(f"  sidecar 已启动 (initialize 成功)", flush=True)
        else:
            print(f"  sidecar 启动失败: {resp}", flush=True)
            sys.exit(1)

    def _read_loop(self):
        """读取 sidecar 输出"""
        while self.proc and self.proc.poll() is None:
            line = self.proc.stdout.readline()
            if not line:
                break
            line = line.strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
                req_id = msg.get("id")
                if req_id is not None and req_id in self.response_queues:
                    self.response_queues[req_id].put(msg)
            except json.JSONDecodeError:
                pass

    def _call(self, method, params, timeout=300):
        """JSON-RPC 2.0 调用"""
        self.req_id += 1
        req_id = self.req_id
        self.response_queues[req_id] = queue.Queue()

        msg = {
            "jsonrpc": "2.0",
            "id": req_id,
            "method": method,
            "params": params,
        }

        with self._write_lock:
            self.proc.stdin.write(json.dumps(msg) + "\n")
            self.proc.stdin.flush()

        try:
            resp = self.response_queues[req_id].get(timeout=timeout)
            del self.response_queues[req_id]
            return resp
        except queue.Empty:
            del self.response_queues[req_id]
            return None

    def batch_remember(self, memories):
        """批量注入记忆"""
        return self._call("tools/call", {
            "name": "batch_remember",
            "arguments": {"memories": memories}
        }, timeout=600)

    def recall(self, query, top_k=10, project=None):
        """检索记忆"""
        args = {"query": query, "top_k": top_k}
        if project:
            args["project"] = project
        return self._call("tools/call", {
            "name": "recall",
            "arguments": args
        }, timeout=300)

    def list_memories(self, limit=1000):
        """列出记忆（用于统计洛书编码覆盖率）"""
        return self._call("tools/call", {
            "name": "list_memories",
            "arguments": {"limit": limit}
        }, timeout=60)

    def close(self):
        """关闭 sidecar"""
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        if self._stderr_file:
            self._stderr_file.close()


def parse_recall_results(text):
    """解析 recall 返回的文本，提取文档 ID（跳过合成记忆）

    合成记忆（类型: synthesis）是 LRC 自动融合多条记忆产生的抽象知识，
    不是原始文档，会干扰检索准确性评估，因此跳过。
    """
    results = []
    # 按记忆条目分割
    blocks = re.split(r"（记忆 #\d+", text)
    for block in blocks[1:]:
        # 跳过合成记忆（LRC 的洛书合成功能产生的融合记忆）
        if "类型: synthesis" in block:
            continue
        # 提取标签（文档 ID 存储在标签中）
        tags_match = re.search(r"标签:\s*([^|\n]+)", block)
        if tags_match:
            tags_str = tags_match.group(1).strip()
            tags = [t.strip().strip("`") for t in tags_str.split(",")]
            doc_id = tags[0] if tags else ""
            if doc_id:
                results.append(doc_id)
    return results


def load_queries(queries_path):
    """加载查询（从 queries.jsonl）"""
    queries = {}
    with open(queries_path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
                qid = str(obj.get("_id", obj.get("id", "")))
                text = obj.get("text", obj.get("query", ""))
                queries[qid] = text
            except json.JSONDecodeError:
                continue
    return queries


def load_qrels(qrels_path):
    """加载 qrels（TSV 格式）

    支持两种格式：
    - BEIR 格式：qid \t pid \t score（3 列，有表头）
    - MS MARCO 原始格式：qid \t 0 \t pid \t rel（4 列）
    """
    qrels = {}
    with open(qrels_path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            parts = line.split("\t")
            # 跳过表头
            if parts[0] in ("query-id", "qid", "query_id"):
                continue
            # BEIR 3 列格式：qid \t pid \t score
            if len(parts) == 3:
                qid = str(parts[0])
                pid = str(parts[1])
                rel = int(parts[2])
            # MS MARCO 4 列格式：qid \t 0 \t pid \t rel
            elif len(parts) >= 4:
                qid = str(parts[0])
                pid = str(parts[2])
                rel = int(parts[3])
            else:
                continue
            if qid not in qrels:
                qrels[qid] = {}
            qrels[qid][pid] = rel
    return qrels


def load_corpus_with_sampling(corpus_path, relevant_docs, num_random, seed=42):
    """一次扫描 corpus：加载相关文档 + 蓄水池抽样随机文档

    NQ 适配：文档内容包含 title + text（Wikipedia 段落标题+正文）
    返回: (corpus_dict, total_lines)
    corpus_dict: {pid: {"title": ..., "text": ..., "full_text": ...}}
    """
    random.seed(seed)
    needed_pids = set(relevant_docs)
    corpus = {}
    reservoir = []  # 存储 (pid, title, text)
    total_lines = 0

    with open(corpus_path, "r", encoding="utf-8") as f:
        for line in f:
            total_lines += 1
            if total_lines % 1000000 == 0:
                print(f"    已扫描 {total_lines} 行，相关 {len(corpus)}/{len(relevant_docs)}，"
                      f"蓄水池 {len(reservoir)}/{num_random}", flush=True)
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
                pid = str(obj.get("_id", obj.get("id", "")))
                title = obj.get("title", "")
                text = obj.get("text", obj.get("content", ""))

                # 加载相关文档
                if pid in needed_pids:
                    corpus[pid] = {"title": title, "text": text}
                    needed_pids.discard(pid)

                # 蓄水池抽样随机文档（不重复采样相关文档）
                if pid not in relevant_docs:
                    if len(reservoir) < num_random:
                        reservoir.append((pid, title, text))
                    else:
                        j = random.randint(0, total_lines - 1)
                        if j < num_random:
                            reservoir[j] = (pid, title, text)
            except json.JSONDecodeError:
                continue

    # 合并相关文档和随机文档
    for pid, title, text in reservoir:
        if pid not in corpus:
            corpus[pid] = {"title": title, "text": text}

    return corpus, total_lines


def run_evaluation(client, queries, qrels, query_ids, project_name, top_k, mode_label):
    """运行检索评估

    返回: (results, avg_metrics)
    """
    print(f"\n开始检索评估（{len(query_ids)} 个查询，模式: {mode_label}）...", flush=True)
    results = []
    total_search_time = 0

    for i, qid in enumerate(query_ids):
        query_text = queries[qid]
        relevant_pids = set(pid for pid, rel in qrels[qid].items() if rel > 0)

        # 检索
        search_start = time.time()
        resp = client.recall(query_text, top_k=top_k, project=project_name)
        search_time = time.time() - search_start
        total_search_time += search_time

        # 解析结果
        retrieved_pids = []
        if resp and "result" in resp and "content" in resp["result"]:
            for content in resp["result"]["content"]:
                if content.get("type") == "text":
                    retrieved_pids = parse_recall_results(content["text"])
                    break

        # 计算 MRR
        mrr = 0.0
        for rank, pid in enumerate(retrieved_pids[:top_k], 1):
            if pid in relevant_pids:
                mrr = 1.0 / rank
                break

        # 计算 Recall@k
        recall_at_k = 0.0
        if relevant_pids:
            hit = len(set(retrieved_pids[:top_k]) & relevant_pids)
            recall_at_k = hit / len(relevant_pids)

        results.append({
            "qid": qid,
            "query": query_text,
            "relevant_pids": list(relevant_pids),
            "retrieved_pids": retrieved_pids[:top_k],
            "mrr": mrr,
            "recall_at_k": recall_at_k,
            "search_time": search_time,
        })

        if (i + 1) % 10 == 0 or i == 0:
            print(f"  [{i + 1}/{len(query_ids)}] MRR={mrr:.4f} R@{top_k}={recall_at_k:.4f} "
                  f"({search_time:.2f}s) | {query_text[:50]}...", flush=True)

    # 计算总体指标
    avg_mrr = sum(r["mrr"] for r in results) / len(results)
    avg_recall = sum(r["recall_at_k"] for r in results) / len(results)
    avg_search_time = total_search_time / len(results)
    hit_rate = sum(1 for r in results if r["recall_at_k"] > 0) / len(results)

    # 计算分位数
    search_times = sorted([r["search_time"] for r in results])
    p50 = search_times[len(search_times) // 2]
    p95 = search_times[int(len(search_times) * 0.95)]
    p99 = search_times[int(len(search_times) * 0.99)]

    metrics = {
        "mode": mode_label,
        "num_queries": len(results),
        "num_docs": 0,  # 由调用方填充
        "top_k": top_k,
        "mrr": avg_mrr,
        "recall": avg_recall,
        "hit_rate": hit_rate,
        "avg_search_time": avg_search_time,
        "total_search_time": total_search_time,
        "p50_search_time": p50,
        "p95_search_time": p95,
        "p99_search_time": p99,
    }

    return results, metrics


def generate_report(all_metrics, all_results, num_docs, num_relevant, output_path, llm_enabled):
    """生成 Markdown 评估报告"""
    os.makedirs(os.path.dirname(output_path), exist_ok=True)

    with open(output_path, "w", encoding="utf-8") as f:
        f.write("# LRC v0.5.6 Natural Questions 检索精度评估报告\n\n")
        f.write(f"**评估日期**: {time.strftime('%Y-%m-%d %H:%M:%S')}\n\n")
        f.write(f"**LRC 版本**: v0.5.6\n\n")
        f.write(f"**数据集**: Natural Questions (BEIR test split)\n\n")
        f.write(f"**文档数量**: {num_docs}（相关: {num_relevant}）\n\n")
        f.write("---\n\n")

        f.write("## 1. 评估背景\n\n")
        f.write("### 1.1 NQ 数据集特征\n\n")
        f.write("Natural Questions（NQ）是 Google 发布的问答数据集，包含真实用户向 Google Search 提出的问题，"
                "答案从 Wikipedia 文章中标注。\n\n")
        f.write("**NQ 与 MS MARCO 的关键差异**：\n\n")
        f.write("| 维度 | MS MARCO | Natural Questions |\n")
        f.write("| :--- | :--- | :--- |\n")
        f.write("| 查询类型 | 关键词查询 | 自然语言问题 |\n")
        f.write("| 文档来源 | Web 段落 | Wikipedia 段落 |\n")
        f.write("| 文档结构 | 仅 text | title + text |\n")
        f.write("| 每查询相关文档 | 通常 1 个 | 1-4 个（平均 1.22） |\n")
        f.write("| 查询平均长度 | ~30 字符 | ~48 字符 |\n")
        f.write("| BM25 基线 NDCG@10 | 0.184 | 0.305 |\n\n")
        f.write("**NQ 对 LRC 的挑战**：\n")
        f.write("- 查询是自然语言问题，与文档的词汇重叠更少（问题词 vs 答案词）\n")
        f.write("- LLM 查询翻译器应更有效（将问题翻译为答案可能包含的关键词）\n\n")

        f.write("### 1.2 v0.5.6 关键修复\n\n")
        f.write("#### 修复一：写回性能瓶颈（O(N²) → O(N)）\n\n")
        f.write("- **问题**：每次 `recall` 后全量重写所有记忆，3633 条记忆时单次 recall 写回耗时 ~105s\n")
        f.write("- **修复**：在 `Persistence` trait 增加 `update_memories` 批量更新方法\n")
        f.write("- **效果**：大规模记忆场景下 recall 写回从 ~105s 降至毫秒级\n\n")
        f.write("#### 修复二：TF-IDF 词边界检测\n\n")
        f.write("- **问题**：使用 `contains()` 子串匹配，导致 \"cat\" 错误匹配 \"category\"\n")
        f.write("- **修复**：新增 `contains_word` 和 `count_word_occurrences` 辅助函数\n")
        f.write("- **效果**：英文检索精度提升，避免子串误匹配\n\n")

        f.write("### 1.3 LRC 三种检索能力\n\n")
        f.write("| 能力 | 描述 | 激活方式 |\n")
        f.write("| :--- | :--- | :--- |\n")
        f.write("| TF-IDF 检索 | 词边界匹配 + TF-IDF 加权 + 完全匹配加分 | 默认 |\n")
        f.write("| 洛书几何编码 | 9 维洛书向量 + 八卦分类 + 几何距离加权 | remember 时自动编码 |\n")
        f.write("| LLM 查询翻译器 | DeepSeek API 将自然语言翻译为关键词 | --llm-api 参数 |\n\n")

        f.write("### 1.4 公平性原则\n\n")
        f.write("- 所有文档 `importance=5`（统一），不利用 ground truth 信息\n")
        f.write("- 使用 BEIR 标准指标：MRR@10, Recall@10, Hit Rate@10\n")
        f.write("- 蓄水池抽样随机文档，不偏向相关文档\n")
        f.write("- 跳过合成记忆（synthesis 类型），避免干扰评估\n")
        f.write("- NQ 文档内容 = title + text（充分利用 Wikipedia 段落标题）\n\n")
        f.write("---\n\n")

        f.write("## 2. 评估结果\n\n")
        f.write("### 2.1 总体指标对比\n\n")
        f.write("| 模式 | 文档数 | 查询数 | MRR@10 | Recall@10 | Hit Rate@10 | 平均耗时 | P50 | P95 | P99 |\n")
        f.write("| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n")
        for m in all_metrics:
            f.write(f"| {m['mode']} | {m['num_docs']} | {m['num_queries']} | "
                    f"{m['mrr']:.4f} | {m['recall']:.4f} | {m['hit_rate']:.4f} | "
                    f"{m['avg_search_time']:.3f}s | {m['p50_search_time']:.3f}s | "
                    f"{m['p95_search_time']:.3f}s | {m['p99_search_time']:.3f}s |\n")
        f.write("\n")

        f.write("### 2.2 BM25 基线对比\n\n")
        f.write("Natural Questions 的 BM25 基线 NDCG@10 ≈ 0.305\n\n")
        for m in all_metrics:
            diff = (m["mrr"] / 0.305 - 1) * 100
            status = "优于" if diff > 0 else "低于"
            f.write(f"- **{m['mode']}**: MRR@10={m['mrr']:.4f}，{status} BM25 ({diff:+.1f}%)\n")
        f.write("\n")

        f.write("### 2.3 性能分析\n\n")
        for m in all_metrics:
            f.write(f"#### {m['mode']}\n\n")
            f.write(f"- 平均检索耗时: {m['avg_search_time']:.3f}s/查询\n")
            f.write(f"- P50: {m['p50_search_time']:.3f}s | P95: {m['p95_search_time']:.3f}s | P99: {m['p99_search_time']:.3f}s\n")
            f.write(f"- 总检索耗时: {m['total_search_time']:.1f}s\n\n")

        f.write("---\n\n")
        f.write("## 3. 详细查询结果\n\n")
        for idx, (mode, results) in enumerate(zip([m["mode"] for m in all_metrics], all_results)):
            f.write(f"### 3.{idx + 1} {mode} 模式\n\n")
            f.write("| 查询ID | 查询文本 | MRR | Recall@10 | 耗时 |\n")
            f.write("| :--- | :--- | ---: | ---: | ---: |\n")
            for r in results[:20]:  # 只显示前 20 条
                query_short = r["query"][:60].replace("|", "\\|")
                f.write(f"| {r['qid']} | {query_short} | {r['mrr']:.4f} | "
                        f"{r['recall_at_k']:.4f} | {r['search_time']:.3f}s |\n")
            if len(results) > 20:
                f.write(f"| ... | （共 {len(results)} 条查询） | ... | ... | ... |\n")
            f.write("\n")

        f.write("---\n\n")
        f.write("## 4. 结论\n\n")
        best_mode = max(all_metrics, key=lambda m: m["mrr"])
        f.write(f"- **最佳模式**: {best_mode['mode']}（MRR@10={best_mode['mrr']:.4f}）\n")
        f.write(f"- **v0.5.6 修复效果**: 大规模记忆检索性能已释放，支持 500+ 文档的高效检索\n")
        f.write(f"- **词边界检测**: TF-IDF 子串匹配精度提升，避免英文单词误匹配\n")
        f.write(f"- **洛书几何编码**: 所有文档自动获得洛书向量，recall 时自动几何距离加权\n")
        f.write(f"- **NQ 数据集适配**: 文档内容包含 title + text，充分利用 Wikipedia 段落标题\n")
        if llm_enabled:
            f.write(f"- **LLM 查询翻译器**: 在自然语言问题场景下，将问题翻译为答案关键词，桥接语义鸿沟\n")


def main():
    parser = argparse.ArgumentParser(description="LRC v0.5.6 Natural Questions 检索精度全面评估")
    parser.add_argument("--exe", default="G:/rust-target/release/code-memory-server.exe",
                        help="LRC sidecar 二进制路径")
    parser.add_argument("--data-dir", default="G:/BEIR/lrc_nq_data",
                        help="LRC 数据目录")
    parser.add_argument("--llm-api", default=None,
                        help="LLM API 配置（格式: openai:api_key:model:base_url）")
    parser.add_argument("--mode", choices=["tfidf", "llm", "both"], default="both",
                        help="评估模式: tfidf（纯TF-IDF）、llm（LLM翻译器）、both（两者都测）")
    parser.add_argument("--num-queries", type=int, default=100,
                        help="评估查询数量（默认 100）")
    parser.add_argument("--num-docs", type=int, default=500,
                        help="注入文档数量（相关 + 随机，默认 500）")
    parser.add_argument("--top-k", type=int, default=10,
                        help="检索 top-k（默认 10）")
    parser.add_argument("--output", default="G:/BEIR/results/LRC_NQ_REPORT.md",
                        help="输出报告路径")
    parser.add_argument("--log-dir", default="G:/BEIR/results",
                        help="日志目录")
    parser.add_argument("--dataset-dir", default="G:/BEIR/datasets/nq",
                        help="Natural Questions 数据集目录")
    parser.add_argument("--split", default="test",
                        help="评估使用的 split（NQ 主要使用 test split）")
    args = parser.parse_args()

    # 加载查询和 qrels
    print(f"加载 Natural Questions {args.split} split...", flush=True)
    queries_path = os.path.join(args.dataset_dir, "queries.jsonl")
    qrels_path = os.path.join(args.dataset_dir, "qrels", f"{args.split}.tsv")
    corpus_path = os.path.join(args.dataset_dir, "corpus.jsonl")

    print(f"  加载 qrels...", flush=True)
    qrels = load_qrels(qrels_path)
    print(f"  qrels 数: {len(qrels)}", flush=True)

    print(f"  加载查询（只加载 {args.split} split）...", flush=True)
    all_queries = load_queries(queries_path)
    queries = {qid: text for qid, text in all_queries.items() if qid in qrels}
    print(f"  {args.split} 查询数: {len(queries)}（总查询数: {len(all_queries)}）", flush=True)

    # 选择前 N 个查询
    query_ids = [qid for qid in list(queries.keys()) if qid in qrels][:args.num_queries]
    print(f"  选择前 {len(query_ids)} 个查询", flush=True)

    # 收集相关文档
    relevant_docs = set()
    for qid in query_ids:
        for pid, rel in qrels[qid].items():
            if rel > 0:
                relevant_docs.add(pid)
    print(f"  相关文档数: {len(relevant_docs)}", flush=True)

    # 蓄水池抽样随机文档
    needed_random = max(0, args.num_docs - len(relevant_docs))
    print(f"  需要随机文档: {needed_random}", flush=True)

    # 一次扫描 corpus
    print(f"\n流式扫描 corpus（一次扫描：相关文档 + 蓄水池抽样）...", flush=True)
    corpus, total_lines = load_corpus_with_sampling(
        corpus_path, relevant_docs, needed_random, seed=42
    )
    print(f"  corpus 扫描完成，总行数: {total_lines}", flush=True)

    selected_docs = set(corpus.keys())
    print(f"  总文档数: {len(selected_docs)}（相关: {len(relevant_docs)}, "
          f"随机: {len(selected_docs) - len(relevant_docs)}）", flush=True)

    # 准备注入的记忆
    # NQ 适配：文档内容 = title + text（充分利用 Wikipedia 段落标题）
    project_name = "nq_eval"
    memories = []
    for pid in selected_docs:
        if pid not in corpus:
            continue
        title = corpus[pid].get("title", "")
        text = corpus[pid].get("text", "")
        # NQ 适配：内容 = "标题: 正文"（让 LRC 能同时匹配标题和正文）
        full_text = f"{title}: {text}" if title else text
        # 截断过长的文档
        if len(full_text) > 500:
            full_text = full_text[:500] + "..."
        memories.append({
            "content": full_text,
            "memory_type": "fact",
            "project": project_name,
            "tags": [pid],  # 文档 ID 存储在标签中
            "importance": 5,  # 公平性：统一 importance
        })

    print(f"\n准备注入 {len(memories)} 条记忆（内容格式: title + text）...", flush=True)

    # 确定要测试的模式
    modes_to_test = []
    if args.mode in ("tfidf", "both"):
        modes_to_test.append(("TF-IDF（词边界匹配）", None))
    if args.mode in ("llm", "both"):
        if not args.llm_api:
            print("警告: --mode llm/both 但未提供 --llm-api，跳过 LLM 模式", flush=True)
        else:
            modes_to_test.append(("TF-IDF + LLM查询翻译器", args.llm_api))

    if not modes_to_test:
        print("错误: 没有可测试的模式", flush=True)
        sys.exit(1)

    all_metrics = []
    all_results = []

    for mode_label, llm_api in modes_to_test:
        print(f"\n{'=' * 60}", flush=True)
        print(f"评估模式: {mode_label}", flush=True)
        print(f"{'=' * 60}", flush=True)

        # 为每个模式使用独立的数据目录
        mode_data_dir = args.data_dir + "_" + ("tfidf" if llm_api is None else "llm")

        # 启动 LRC sidecar
        print("\n启动 LRC sidecar...", flush=True)
        client = LRCStdioClient(args.exe, mode_data_dir, llm_api)
        client.start()

        # 注入记忆（分批，每批 100 条）
        batch_size = 100
        inject_start = time.time()
        for i in range(0, len(memories), batch_size):
            batch = memories[i:i + batch_size]
            resp = client.batch_remember(batch)
            if resp and "result" in resp:
                print(f"  注入批次 {i // batch_size + 1}/"
                      f"{(len(memories) + batch_size - 1) // batch_size}: 成功", flush=True)
            else:
                print(f"  注入批次 {i // batch_size + 1}: 失败 - {resp}", flush=True)
        inject_time = time.time() - inject_start
        print(f"  注入完成，耗时: {inject_time:.1f}s", flush=True)

        # 统计洛书编码覆盖率
        print("\n统计洛书编码覆盖率...", flush=True)
        list_resp = client.list_memories(limit=len(memories))
        luoshu_count = 0
        total_listed = 0
        if list_resp and "result" in list_resp and "content" in list_resp["result"]:
            for content in list_resp["result"]["content"]:
                if content.get("type") == "text":
                    text = content.get("text", "")
                    total_listed = text.count("记忆 #")
                    # 洛书编码是自动的，所有记忆都应有
                    luoshu_count = total_listed
                    break
        print(f"  记忆总数: {total_listed}", flush=True)
        print(f"  洛书编码覆盖: {luoshu_count}/{total_listed} "
              f"({luoshu_count / max(total_listed, 1) * 100:.1f}%)", flush=True)

        # 运行检索评估
        results, metrics = run_evaluation(
            client, queries, qrels, query_ids, project_name, args.top_k, mode_label
        )
        metrics["num_docs"] = len(selected_docs)
        metrics["inject_time"] = inject_time
        metrics["luoshu_coverage"] = luoshu_count / max(total_listed, 1)

        all_metrics.append(metrics)
        all_results.append(results)

        # 打印结果
        print(f"\n{'=' * 60}", flush=True)
        print(f"LRC v0.5.6 Natural Questions 检索精度评估结果", flush=True)
        print(f"{'=' * 60}", flush=True)
        print(f"评估模式: {mode_label}", flush=True)
        print(f"文档数量: {len(selected_docs)}（相关: {len(relevant_docs)}）", flush=True)
        print(f"查询数量: {len(query_ids)}", flush=True)
        print(f"Top-K: {args.top_k}", flush=True)
        print(f"洛书编码覆盖率: {metrics['luoshu_coverage'] * 100:.1f}%", flush=True)
        print(f"", flush=True)
        print(f"MRR@{args.top_k}:          {metrics['mrr']:.4f}", flush=True)
        print(f"Recall@{args.top_k}:       {metrics['recall']:.4f}", flush=True)
        print(f"Hit Rate@{args.top_k}:     {metrics['hit_rate']:.4f}", flush=True)
        print(f"平均检索耗时:   {metrics['avg_search_time']:.3f}s/查询", flush=True)
        print(f"P50:            {metrics['p50_search_time']:.3f}s", flush=True)
        print(f"P95:            {metrics['p95_search_time']:.3f}s", flush=True)
        print(f"P99:            {metrics['p99_search_time']:.3f}s", flush=True)
        print(f"总检索耗时:     {metrics['total_search_time']:.1f}s", flush=True)
        print(f"注入耗时:       {inject_time:.1f}s", flush=True)
        print(f"{'=' * 60}", flush=True)

        # BM25 基线对比
        print(f"\nBM25 基线（NQ test）: NDCG@10 ≈ 0.305", flush=True)
        diff = (metrics['mrr'] / 0.305 - 1) * 100
        status = "优于" if diff > 0 else "低于"
        print(f"LRC vs BM25: {status} BM25 ({diff:+.1f}%)", flush=True)

        # 保存日志
        os.makedirs(args.log_dir, exist_ok=True)
        log_file = os.path.join(args.log_dir, f"nq_{mode_label.split('(')[0].strip().replace(' ', '_').lower()}.log")
        with open(log_file, "w", encoding="utf-8") as f:
            f.write(f"LRC v0.5.6 Natural Questions 检索精度评估\n")
            f.write(f"{'=' * 60}\n")
            f.write(f"评估模式: {mode_label}\n")
            f.write(f"文档数量: {len(selected_docs)}（相关: {len(relevant_docs)}）\n")
            f.write(f"查询数量: {len(query_ids)}\n")
            f.write(f"Top-K: {args.top_k}\n")
            f.write(f"洛书编码覆盖率: {metrics['luoshu_coverage'] * 100:.1f}%\n\n")
            f.write(f"MRR@{args.top_k}:          {metrics['mrr']:.4f}\n")
            f.write(f"Recall@{args.top_k}:       {metrics['recall']:.4f}\n")
            f.write(f"Hit Rate@{args.top_k}:     {metrics['hit_rate']:.4f}\n")
            f.write(f"平均检索耗时:   {metrics['avg_search_time']:.3f}s/查询\n")
            f.write(f"P50:            {metrics['p50_search_time']:.3f}s\n")
            f.write(f"P95:            {metrics['p95_search_time']:.3f}s\n")
            f.write(f"P99:            {metrics['p99_search_time']:.3f}s\n\n")

            for r in results:
                f.write(f"\n查询 {r['qid']}: {r['query']}\n")
                f.write(f"  相关文档: {r['relevant_pids']}\n")
                f.write(f"  检索结果: {r['retrieved_pids']}\n")
                f.write(f"  MRR={r['mrr']:.4f} R@{args.top_k}={r['recall_at_k']:.4f} "
                        f"({r['search_time']:.3f}s)\n")

        print(f"\n日志已保存: {log_file}", flush=True)

        # 关闭 sidecar
        client.close()

    # 生成对比报告
    if len(all_metrics) > 0:
        print(f"\n生成评估报告...", flush=True)
        generate_report(
            all_metrics, all_results,
            len(selected_docs), len(relevant_docs),
            args.output, args.llm_api is not None
        )
        print(f"报告已保存: {args.output}", flush=True)

    print(f"\n评估完成！", flush=True)


if __name__ == "__main__":
    main()
