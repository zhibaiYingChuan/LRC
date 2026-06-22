"""
LRC LongMemEval 检索精度评估 — 真实 sidecar stdio 模式
=====================================================
通过 MCP stdio 协议直接与 LRC sidecar 通信。
每个 LongMemEval 实例：
  1. 启动 sidecar stdio 进程
  2. initialize → batch_remember → recall
  3. 检查 Top-K 结果中是否包含证据会话
  4. 关闭进程

评估指标：Session Recall@10, Turn Recall@10, MRR
"""

import json
import os
import re
import sys
import time
import argparse
import subprocess
import threading
import queue
import numpy as np
from collections import Counter
from typing import Optional


# ============================================================
# LRC Stdio 客户端 — 通过 MCP 协议与 sidecar 通信
# ============================================================

class LRCStdioClient:
    """通过 stdio 模式与 LRC sidecar 通信"""

    def __init__(self, exe_path: str, data_dir: str, src_dir: str = "G:\\lrc_empty_workspace",
                 llm_api: str = None):
        self.exe_path = exe_path
        self.data_dir = data_dir
        self.src_dir = src_dir
        self.llm_api = llm_api
        self.proc: Optional[subprocess.Popen] = None
        self._req_id = 0
        self._q: queue.Queue = None
        self._threads: list = []

    def start(self):
        """启动 sidecar stdio 进程"""
        # 清理数据目录
        if os.path.exists(self.data_dir):
            for f in os.listdir(self.data_dir):
                p = os.path.join(self.data_dir, f)
                if os.path.isfile(p):
                    os.remove(p)
        else:
            os.makedirs(self.data_dir)

        env = os.environ.copy()
        env["RUST_LOG"] = "warn"

        cmd = [
            self.exe_path,
            "--stdio",
            "--src-dir", self.src_dir,
            "--data-dir", self.data_dir,
        ]
        # 指定 LLM API（DeepSeek）用于查询翻译
        if self.llm_api:
            cmd.extend(["--llm-api", self.llm_api])

        self.proc = subprocess.Popen(
            cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
            encoding="utf-8",
            env=env,
        )

        self._q = queue.Queue()
        t1 = threading.Thread(target=self._read_stdout, daemon=True)
        t2 = threading.Thread(target=self._read_stderr, daemon=True)
        t1.start()
        t2.start()
        self._threads = [t1, t2]

        # 发送 initialize
        r = self._call("initialize", {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "lrc-eval", "version": "1.0.0"},
        }, timeout=30)
        if "error" in r:
            raise RuntimeError(f"initialize 失败: {r['error']}")

    def _read_stdout(self):
        while True:
            line = self.proc.stdout.readline()
            if not line:
                break
            self._q.put(("stdout", line))

    def _read_stderr(self):
        while True:
            line = self.proc.stderr.readline()
            if not line:
                break
            self._q.put(("stderr", line))

    def stop(self):
        """关闭 sidecar 进程"""
        if self.proc:
            try:
                self.proc.stdin.close()
                self.proc.terminate()
                self.proc.wait(timeout=5)
            except Exception:
                self.proc.kill()
            self.proc = None

    def _call(self, method: str, params: dict, timeout: int = 120) -> dict:
        """调用 MCP 方法"""
        self._req_id += 1
        req_id = self._req_id
        if method == "tools/call":
            payload = {
                "jsonrpc": "2.0",
                "id": req_id,
                "method": "tools/call",
                "params": params,
            }
        else:
            payload = {
                "jsonrpc": "2.0",
                "id": req_id,
                "method": method,
                "params": params,
            }

        line = json.dumps(payload) + "\n"
        try:
            self.proc.stdin.write(line)
            self.proc.stdin.flush()
        except (BrokenPipeError, OSError):
            return {"error": "进程已关闭"}

        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                source, resp_line = self._q.get(timeout=1)
                resp_line = resp_line.strip()
                if not resp_line:
                    continue
                if source == "stdout" and resp_line.startswith("{"):
                    try:
                        resp = json.loads(resp_line)
                        if resp.get("id") == req_id:
                            return resp
                    except json.JSONDecodeError:
                        continue
            except queue.Empty:
                if self.proc.poll() is not None:
                    return {"error": f"进程已退出 (code={self.proc.returncode})"}
                continue
        return {"error": "超时"}

    def batch_remember(self, memories: list[dict]) -> dict:
        """批量注入记忆"""
        return self._call("tools/call", {
            "name": "batch_remember",
            "arguments": {"memories": memories},
        }, timeout=180)

    def recall(self, query: str, top_k: int = 10, project: Optional[str] = None) -> str:
        """检索记忆，返回原始文本"""
        args = {"query": query, "top_k": top_k}
        if project:
            args["project"] = project
        result = self._call("tools/call", {
            "name": "recall",
            "arguments": args,
        }, timeout=60)
        if "error" in result:
            return ""
        content = result.get("result", {}).get("content", [])
        if not content:
            return ""
        return content[0].get("text", "")


# ============================================================
# 解析 recall 返回的文本
# ============================================================

def parse_recall_results(text: str) -> list[dict]:
    """解析 LRC recall 返回的文本，提取每条记忆的标签"""
    results = []
    # 按记忆条目分割（每条以"（记忆 #"开头）
    blocks = re.split(r"（记忆 #\d+", text)
    for block in blocks[1:]:  # 跳过第一块（标题）
        # 提取标签
        tags_match = re.search(r"标签:\s*([^\n]+)", block)
        tags = tags_match.group(1).strip() if tags_match else ""
        # 提取内容
        content_match = re.search(r"内容:\s*([^\n]+)", block)
        content = content_match.group(1).strip() if content_match else ""
        # 提取 ID
        id_match = re.search(r"ID:\s*`([^`]+)`", block)
        mem_id = id_match.group(1).strip() if id_match else ""

        results.append({
            "tags": tags,
            "content": content,
            "id": mem_id,
        })
    return results


# ============================================================
# 评估逻辑
# ============================================================

def evaluate_retrieval(
    data_path: str,
    exe_path: str,
    data_dir: str,
    top_k: int = 10,
    max_instances: Optional[int] = None,
    verbose: bool = False,
    llm_api: str = None,
):
    """评估 LRC 真实 sidecar 的检索精度"""

    print(f"加载数据集: {data_path}", flush=True)
    with open(data_path, "r", encoding="utf-8") as f:
        dataset = json.load(f)

    if max_instances:
        dataset = dataset[:max_instances]

    print(f"评估 {len(dataset)} 条实例 (Top-K={top_k})", flush=True)
    print(f"Sidecar: {exe_path}", flush=True)
    print(f"数据目录: {data_dir}", flush=True)
    print(f"LLM API: {llm_api if llm_api else '未指定（纯关键词匹配）'}", flush=True)
    print(flush=True)

    # 指标累计
    metrics = {
        "session_recall": [],
        "turn_recall": [],
        "mrr_session": [],
        "mrr_turn": [],
    }

    type_metrics: dict[str, dict] = {}
    type_instances = Counter()

    total_recall_time = 0.0
    total_inject_time = 0.0
    errors = []

    for idx, instance in enumerate(dataset):
        question_id = instance["question_id"]
        question_type = instance["question_type"]
        question = instance["question"]
        haystack_sessions = instance.get("haystack_sessions", [])
        haystack_dates = instance.get("haystack_dates", [])
        haystack_session_ids = instance.get("haystack_session_ids", [])
        answer_session_ids = set(instance.get("answer_session_ids", []))

        type_instances[question_type] += 1
        if type_metrics.get(question_type) is None:
            type_metrics[question_type] = {
                "session_recall": [],
                "turn_recall": [],
                "mrr_session": [],
                "mrr_turn": [],
            }

        is_abstention = "_abs" in question_id

        # 每个实例启动独立的 sidecar 进程
        client = LRCStdioClient(exe_path, data_dir, llm_api=llm_api)
        try:
            t0 = time.time()
            client.start()
            t1 = time.time()
            if verbose:
                print(f"  sidecar 启动: {t1-t0:.1f}s", flush=True)

            # 步骤 1: 注入会话记忆
            t0 = time.time()
            project_name = f"lme_{question_id}"
            batch = []
            for session, date_str, sid in zip(haystack_sessions, haystack_dates, haystack_session_ids):
                turns = []
                for turn in session:
                    role = turn.get("role", "unknown")
                    content = turn.get("content", "")
                    if len(content.strip()) >= 5:
                        turns.append(f"[{role}]: {content}")
                full = "\n".join(turns)
                if len(full.strip()) >= 20:
                    if len(full) > 8000:
                        full = full[:8000] + "\n...[truncated]"
                    batch.append({
                        "content": full,
                        "memory_type": "conversation",
                        "project": project_name,
                        "tags": [sid, f"date:{date_str}"],
                        "importance": 5,
                    })

            result = client.batch_remember(batch)
            t1 = time.time()
            total_inject_time += (t1 - t0)

            if "error" in result:
                print(f"  [{idx+1}/{len(dataset)}] {question_id} 注入失败: {result['error'][:100]}", flush=True)
                errors.append((question_id, "injection_failed"))
                continue

            if verbose and idx < 3:
                print(f"  注入: {len(batch)} 条 ({t1-t0:.1f}s)", flush=True)

            time.sleep(0.5)

            # 步骤 2: recall 检索
            t0 = time.time()
            raw_text = client.recall(query=question, top_k=top_k, project=f"lme_{question_id}")
            t1 = time.time()
            total_recall_time += (t1 - t0)

            # 解析检索结果
            recalled = parse_recall_results(raw_text)

            if verbose and idx < 5:
                print(f"  recall: {len(recalled)} 条 ({t1-t0:.2f}s)", flush=True)
                print(f"  原始返回前 500 字符: {raw_text[:500]}", flush=True)
                if recalled:
                    print(f"  第一条标签: {recalled[0]['tags'][:100]}", flush=True)

            # 步骤 3: 评估检索精度
            if is_abstention:
                continue

            # Session 级别召回：检查检索结果中是否包含 answer_session_ids
            session_hit = False
            session_rr = 0.0
            for rank, item in enumerate(recalled):
                item_tags = item.get("tags", "")
                item_content = item.get("content", "")
                for sid in answer_session_ids:
                    if sid in item_tags or sid in item_content:
                        if not session_hit:
                            session_hit = True
                            session_rr = 1.0 / (rank + 1)
                        break

            # Turn 级别召回：检查检索结果中是否包含 has_answer 的 turn 内容
            turn_hit = False
            turn_rr = 0.0
            answer_contents = []
            for session in haystack_sessions:
                for t in session:
                    if t.get("has_answer", False):
                        ans_content = t.get("content", "").strip()
                        if ans_content:
                            answer_contents.append(ans_content[:80].lower())

            for rank, item in enumerate(recalled):
                item_content = item.get("content", "").lower()
                for ans in answer_contents:
                    if ans and ans in item_content:
                        if not turn_hit:
                            turn_hit = True
                            turn_rr = 1.0 / (rank + 1)
                        break

            metrics["session_recall"].append(1 if session_hit else 0)
            metrics["turn_recall"].append(1 if turn_hit else 0)
            metrics["mrr_session"].append(session_rr)
            metrics["mrr_turn"].append(turn_rr)

            type_metrics[question_type]["session_recall"].append(1 if session_hit else 0)
            type_metrics[question_type]["turn_recall"].append(1 if turn_hit else 0)
            type_metrics[question_type]["mrr_session"].append(session_rr)
            type_metrics[question_type]["mrr_turn"].append(turn_rr)

            status = "✓" if session_hit else "✗"
            print(f"  [{idx+1}/{len(dataset)}] {question_id} ({question_type}) "
                  f"| Session: {status} | Turn: {'✓' if turn_hit else '✗'} | "
                  f"RR: {session_rr:.3f} | {t1-t0:.2f}s", flush=True)

        except Exception as e:
            print(f"  [{idx+1}/{len(dataset)}] {question_id} 异常: {e}", flush=True)
            errors.append((question_id, str(e)[:100]))
        finally:
            client.stop()

    # 输出结果
    n_evaluated = len(metrics["session_recall"])
    abstention_count = sum(1 for inst in dataset[:max_instances or len(dataset)] if "_abs" in inst["question_id"])

    print("\n" + "=" * 70, flush=True)
    print("LRC LongMemEval 检索精度评估报告（真实 sidecar stdio 模式）", flush=True)
    print("=" * 70, flush=True)
    print(f"  评估实例数: {n_evaluated} (跳过 {abstention_count} 条 abstention)", flush=True)
    print(f"  错误数: {len(errors)}", flush=True)
    print(f"  Top-K: {top_k}", flush=True)
    print(f"  平均注入耗时: {total_inject_time/max(n_evaluated,1):.3f}s/实例", flush=True)
    print(f"  平均检索耗时: {total_recall_time/max(n_evaluated,1):.4f}s/实例", flush=True)
    print(flush=True)

    if n_evaluated == 0:
        print("  无有效评估结果", flush=True)
        return metrics, type_metrics, errors

    print("总体指标:", flush=True)
    print(f"  Session Recall@{top_k}:  {np.mean(metrics['session_recall']):.4f}", flush=True)
    print(f"  Turn Recall@{top_k}:     {np.mean(metrics['turn_recall']):.4f}", flush=True)
    print(f"  Session MRR:             {np.mean(metrics['mrr_session']):.4f}", flush=True)
    print(f"  Turn MRR:                {np.mean(metrics['mrr_turn']):.4f}", flush=True)

    print(flush=True)
    print("按问题类型:", flush=True)
    print(f"  {'问题类型':<30} {'数量':>5} {'Session R@K':>12} {'Turn R@K':>12} {'Session MRR':>12} {'Turn MRR':>12}", flush=True)
    print(f"  {'-'*30} {'-'*5} {'-'*12} {'-'*12} {'-'*12} {'-'*12}", flush=True)
    for qtype, tm in sorted(type_metrics.items()):
        if tm["session_recall"]:
            print(f"  {qtype:<30} {len(tm['session_recall']):>5} "
                  f"{np.mean(tm['session_recall']):>12.4f} "
                  f"{np.mean(tm['turn_recall']):>12.4f} "
                  f"{np.mean(tm['mrr_session']):>12.4f} "
                  f"{np.mean(tm['mrr_turn']):>12.4f}", flush=True)

    if errors:
        print(f"\n错误详情 ({len(errors)} 条):", flush=True)
        for qid, err in errors[:10]:
            print(f"  {qid}: {err}", flush=True)

    return metrics, type_metrics, errors


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="LRC LongMemEval 检索精度评估（真实 sidecar）")
    parser.add_argument("--data", default="data/longmemeval_s_cleaned.json")
    parser.add_argument("--exe", default=r"G:\rust-target\release\code-memory-server.exe")
    parser.add_argument("--data-dir", default=r"G:\code-memory\data_eval")
    parser.add_argument("--top-k", type=int, default=10)
    parser.add_argument("--max", type=int, default=None)
    parser.add_argument("--verbose", "-v", action="store_true")
    parser.add_argument("--llm-api", default=None,
                        help="LLM API 配置（如 openai:sk-xxx:deepseek-chat:https://api.deepseek.com/v1）")

    args = parser.parse_args()

    evaluate_retrieval(
        data_path=args.data,
        exe_path=args.exe,
        data_dir=args.data_dir,
        top_k=args.top_k,
        max_instances=args.max,
        verbose=args.verbose,
        llm_api=args.llm_api,
    )
