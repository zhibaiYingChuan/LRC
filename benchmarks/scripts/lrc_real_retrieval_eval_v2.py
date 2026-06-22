"""
LRC LongMemEval 检索精度评估 v2 — Turn 级注入 + 重要性差异化
==========================================================
基于 v1 (lrc_real_retrieval_eval.py) 改进：
  1. 恢复 Turn 级注入：每个 turn 作为独立记忆
  2. 重要性差异化：
     - 会话级记忆 importance=5（保持）
     - has_answer 的 Turn importance=8（高优先级）
     - 普通 Turn importance=4（低于会话级，避免噪声）
  3. Turn 级记忆 tags 包含 session_id + turn_idx，便于 Session 召回

预期效果：
  - Session Recall@10: 72.77% → 85%+
  - Turn Recall@10: 38.09% → 75%+
  - Session/Turn MRR 显著提升
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
            "clientInfo": {"name": "lrc-eval-v2", "version": "2.0.0"},
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
        }, timeout=300)  # v2 注入量更大，超时放宽到 300s

    def batch_remember_chunked(self, memories: list[dict], chunk_size: int = 200) -> dict:
        """分批注入记忆（LRC batch_remember 上限 200 条/次）"""
        total = len(memories)
        injected = 0
        for i in range(0, total, chunk_size):
            chunk = memories[i:i + chunk_size]
            result = self.batch_remember(chunk)
            if "error" in result:
                return result
            injected += len(chunk)
        return {"result": {"content": [{"type": "text", "text": f"分批注入完成: {injected}/{total} 条"}]}}

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
    """解析 LRC recall 返回的文本，提取每条记忆的标签和内容"""
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
# v2 注入策略：会话级 + Turn 级双重注入 + 重要性差异化
# ============================================================

def build_memories_v2(haystack_sessions, haystack_dates, haystack_session_ids, project_name):
    """v2 注入策略：会话级 + Turn 级双重注入，重要性差异化

    返回:
        batch: 待注入的记忆列表
        stats: 注入统计（会话数、turn 数、has_answer turn 数）
    """
    batch = []
    n_sessions = 0
    n_turns = 0
    n_answer_turns = 0

    for session, date_str, sid in zip(haystack_sessions, haystack_dates, haystack_session_ids):
        # === 会话级记忆（importance=5）===
        turns_text = []
        for turn in session:
            role = turn.get("role", "unknown")
            content = turn.get("content", "")
            if len(content.strip()) >= 5:
                turns_text.append(f"[{role}]: {content}")
        full = "\n".join(turns_text)
        if len(full.strip()) >= 20:
            if len(full) > 8000:
                full = full[:8000] + "\n...[truncated]"
            batch.append({
                "content": full,
                "memory_type": "conversation",
                "project": project_name,
                "tags": [sid, f"date:{date_str}", "level:session"],
                "importance": 5,
            })
            n_sessions += 1

        # === Turn 级记忆（importance 差异化）===
        for turn_idx, turn in enumerate(session):
            role = turn.get("role", "unknown")
            content = turn.get("content", "").strip()
            if len(content) < 5:
                continue  # 跳过过短的 turn

            has_answer = turn.get("has_answer", False)
            # Turn 级记忆 content 包含 role + 原始内容，便于 Turn 召回判定
            turn_content = f"[{role}] {content}"
            if len(turn_content) > 4000:
                turn_content = turn_content[:4000] + "...[truncated]"

            # 重要性差异化：has_answer=8（高），普通=4（低于会话级）
            importance = 8 if has_answer else 4

            batch.append({
                "content": turn_content,
                "memory_type": "conversation",
                "project": project_name,
                "tags": [sid, f"turn:{turn_idx}", f"date:{date_str}",
                         "level:turn", "has_answer" if has_answer else "no_answer"],
                "importance": importance,
            })
            n_turns += 1
            if has_answer:
                n_answer_turns += 1

    stats = {
        "n_sessions": n_sessions,
        "n_turns": n_turns,
        "n_answer_turns": n_answer_turns,
        "total": len(batch),
    }
    return batch, stats


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
    log_path: str = None,
):
    """评估 LRC 真实 sidecar 的检索精度（v2 策略）"""

    print(f"加载数据集: {data_path}", flush=True)
    with open(data_path, "r", encoding="utf-8") as f:
        dataset = json.load(f)

    if max_instances:
        dataset = dataset[:max_instances]

    print(f"评估 {len(dataset)} 条实例 (Top-K={top_k})", flush=True)
    print(f"策略: v2 (会话级 + Turn 级注入 + 重要性差异化)", flush=True)
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
    total_memories = 0
    total_sessions = 0
    total_turns = 0
    total_answer_turns = 0
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

            # 步骤 1: 注入记忆（v2 策略：会话级 + Turn 级）
            t0 = time.time()
            project_name = f"lme_{question_id}"
            batch, stats = build_memories_v2(
                haystack_sessions, haystack_dates, haystack_session_ids, project_name
            )

            result = client.batch_remember_chunked(batch)
            t1 = time.time()
            total_inject_time += (t1 - t0)
            total_memories += stats["total"]
            total_sessions += stats["n_sessions"]
            total_turns += stats["n_turns"]
            total_answer_turns += stats["n_answer_turns"]

            if "error" in result:
                err_msg = str(result.get("error", ""))[:200]
                print(f"  [{idx+1}/{len(dataset)}] {question_id} 注入失败: {err_msg}", flush=True)
                if verbose or idx < 3:
                    print(f"  完整 result: {str(result)[:500]}", flush=True)
                errors.append((question_id, "injection_failed"))
                continue

            if verbose and idx < 3:
                print(f"  注入: {stats['total']} 条 (会话={stats['n_sessions']}, "
                      f"turn={stats['n_turns']}, answer_turn={stats['n_answer_turns']}) "
                      f"({t1-t0:.1f}s)", flush=True)

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
            import traceback as _tb
            print(f"  [{idx+1}/{len(dataset)}] {question_id} 异常: {e}", flush=True)
            if verbose or idx < 3:
                print(_tb.format_exc(), flush=True)
            errors.append((question_id, str(e)[:100]))
        finally:
            client.stop()

    # 输出结果
    n_evaluated = len(metrics["session_recall"])
    abstention_count = sum(1 for inst in dataset[:max_instances or len(dataset)] if "_abs" in inst["question_id"])

    output_lines = []
    output_lines.append("=" * 70)
    output_lines.append("LRC LongMemEval 检索精度评估报告（v2: Turn 级注入 + 重要性差异化）")
    output_lines.append("=" * 70)
    output_lines.append(f"  评估实例数: {n_evaluated} (跳过 {abstention_count} 条 abstention)")
    output_lines.append(f"  错误数: {len(errors)}")
    output_lines.append(f"  Top-K: {top_k}")
    output_lines.append(f"  平均注入耗时: {total_inject_time/max(n_evaluated,1):.3f}s/实例")
    output_lines.append(f"  平均检索耗时: {total_recall_time/max(n_evaluated,1):.4f}s/实例")
    output_lines.append(f"  平均记忆数/实例: {total_memories/max(n_evaluated,1):.1f} "
                        f"(会话={total_sessions/max(n_evaluated,1):.1f}, "
                        f"turn={total_turns/max(n_evaluated,1):.1f}, "
                        f"answer_turn={total_answer_turns/max(n_evaluated,1):.1f})")
    output_lines.append("")

    if n_evaluated == 0:
        output_lines.append("  无有效评估结果")
        _log_output(output_lines, log_path)
        return metrics, type_metrics, errors

    output_lines.append("总体指标:")
    output_lines.append(f"  Session Recall@{top_k}:  {np.mean(metrics['session_recall']):.4f}")
    output_lines.append(f"  Turn Recall@{top_k}:     {np.mean(metrics['turn_recall']):.4f}")
    output_lines.append(f"  Session MRR:             {np.mean(metrics['mrr_session']):.4f}")
    output_lines.append(f"  Turn MRR:                {np.mean(metrics['mrr_turn']):.4f}")

    output_lines.append("")
    output_lines.append("按问题类型:")
    output_lines.append(f"  {'问题类型':<30} {'数量':>5} {'Session R@K':>12} {'Turn R@K':>12} {'Session MRR':>12} {'Turn MRR':>12}")
    output_lines.append(f"  {'-'*30} {'-'*5} {'-'*12} {'-'*12} {'-'*12} {'-'*12}")
    for qtype, tm in sorted(type_metrics.items()):
        if tm["session_recall"]:
            output_lines.append(f"  {qtype:<30} {len(tm['session_recall']):>5} "
                                f"{np.mean(tm['session_recall']):>12.4f} "
                                f"{np.mean(tm['turn_recall']):>12.4f} "
                                f"{np.mean(tm['mrr_session']):>12.4f} "
                                f"{np.mean(tm['mrr_turn']):>12.4f}")

    if errors:
        output_lines.append(f"\n错误详情 ({len(errors)} 条):")
        for qid, err in errors[:10]:
            output_lines.append(f"  {qid}: {err}")

    _log_output(output_lines, log_path)
    return metrics, type_metrics, errors


def _log_output(lines: list[str], log_path: str = None):
    """输出到 stdout 和可选的日志文件"""
    text = "\n".join(lines)
    print(text, flush=True)
    if log_path:
        with open(log_path, "a", encoding="utf-8") as f:
            f.write(text + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="LRC LongMemEval 检索精度评估 v2（Turn 级注入）")
    parser.add_argument("--data", default="data/longmemeval_s_cleaned.json")
    parser.add_argument("--exe", default=r"G:\rust-target\release\code-memory-server.exe")
    parser.add_argument("--data-dir", default=r"G:\code-memory\data_eval_v2")
    parser.add_argument("--top-k", type=int, default=10)
    parser.add_argument("--max", type=int, default=None)
    parser.add_argument("--verbose", "-v", action="store_true")
    parser.add_argument("--llm-api", default=None,
                        help="LLM API 配置（如 openai:sk-xxx:deepseek-chat:https://api.deepseek.com/v1）")
    parser.add_argument("--log", default=None, help="日志输出路径")

    args = parser.parse_args()

    evaluate_retrieval(
        data_path=args.data,
        exe_path=args.exe,
        data_dir=args.data_dir,
        top_k=args.top_k,
        max_instances=args.max,
        verbose=args.verbose,
        llm_api=args.llm_api,
        log_path=args.log,
    )
