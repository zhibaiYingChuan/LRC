"""
LRC 全功能 LongMemEval 基准测试适配器
==========================================
接入真实 LRC 服务端（HTTP MCP API），启用全部功能层：
  - L1: Fast Match（关键词匹配）
  - L2: Luoshu Recall（洛书几何检索 + TrapezoidFocus 梯形聚焦）
  - L2: Recall Enhanced（双路检索 + RRF 融合）
  - LLM: 查询翻译器（自然语言 → 关键词）
  - LLM: 问答生成（DeepSeek V3）

使用方法：
  1. 启动 LRC 服务端:
     code-memory-server --port 3099 --global --mode fast --llm-api openai:sk-xxx:deepseek-chat:https://api.deepseek.com/v1
  2. 设置环境变量 DEEPSEEK_API_KEY
  3. python lrc_full_longmemeval.py --data data/longmemeval_s_cleaned.json --output results/lrc_hypotheses_full_l2.jsonl
  4. 使用 evaluate_qa.py 评估结果
"""

import json
import os
import sys
import time
import argparse
import urllib.request
from collections import Counter
from typing import Optional

# ============================================================
# LRC 服务端 HTTP 客户端
# ============================================================

class LRCServerClient:
    """通过 HTTP MCP API 调用真实 LRC 服务端"""

    def __init__(self, base_url: str = "http://localhost:3099"):
        self.base_url = base_url
        self._req_id = 0

    def _call_tool(self, tool_name: str, arguments: dict) -> dict:
        """调用 MCP 工具"""
        self._req_id += 1
        payload = {
            "jsonrpc": "2.0",
            "id": self._req_id,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments,
            },
        }
        data = json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(
            f"{self.base_url}/mcp",
            data=data,
            headers={"Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(req, timeout=120) as resp:
                result = json.loads(resp.read().decode("utf-8"))
                return result
        except Exception as e:
            print(f"  LRC API 调用失败 ({tool_name}): {e}")
            return {"error": str(e)}

    def remember(
        self,
        content: str,
        memory_type: str = "conversation",
        project: Optional[str] = None,
        tags: Optional[list[str]] = None,
        importance: int = 5,
        ttl_days: Optional[int] = None,
        session_id: Optional[str] = None,
    ) -> dict:
        """存入记忆 — 调用 LRC remember MCP 工具"""
        args = {
            "content": content,
            "memory_type": memory_type,
            "importance": importance,
        }
        if project:
            args["project"] = project
        if tags:
            args["tags"] = tags
        if ttl_days:
            args["ttl_days"] = ttl_days
        if session_id:
            args["session_id"] = session_id

        return self._call_tool("remember", args)

    def remember_batch(self, memories: list[dict]) -> dict:
        """批量记忆注入 — 调用 LRC batch_remember MCP 工具"""
        return self._call_tool("batch_remember", {"memories": memories})

    def recall(
        self,
        query: str,
        top_k: int = 10,
        memory_type: Optional[str] = None,
        project: Optional[str] = None,
        tags: Optional[list[str]] = None,
        lrc_mode: Optional[str] = None,
    ) -> list[dict]:
        """基础召回 — 调用 LRC recall MCP 工具

        lrc_mode 参数：
          - None/空: L1 Fast Match（TF-IDF 关键词匹配）
          - "luoshu": L2 Luoshu 几何检索（TrapezoidFocus 梯形聚焦）
        """
        args = {"query": query, "top_k": top_k}
        if memory_type:
            args["memory_type"] = memory_type
        if project:
            args["project"] = project
        if tags:
            args["tags"] = tags
        if lrc_mode:
            args["lrc_mode"] = lrc_mode

        result = self._call_tool("recall", args)
        if "error" in result:
            return []

        content_list = result.get("result", {}).get("content", [])
        if not content_list:
            return []

        text = content_list[0].get("text", "")
        return self._parse_recall_text(text)

    def recall_l1(self, query: str, top_k: int = 10, project: Optional[str] = None) -> list[dict]:
        """L1 层召回（Fast Match TF-IDF）"""
        return self.recall(query=query, top_k=top_k, memory_type="conversation", project=project)

    def recall_l2(self, query: str, top_k: int = 10, project: Optional[str] = None) -> list[dict]:
        """L2 层召回（Luoshu 几何检索）"""
        return self.recall(query=query, top_k=top_k, memory_type="conversation", project=project, lrc_mode="luoshu")

    def recall_enhanced(
        self,
        query: str,
        top_k: int = 10,
        memory_type: Optional[str] = None,
        project: Optional[str] = None,
        tags: Optional[list[str]] = None,
    ) -> list[dict]:
        """双路检索增强 — 调用 LRC recall_enhanced MCP 工具

        快速通路（关键词匹配）+ 深度通路（洛书几何检索），通过 RRF 融合。
        """
        args = {"query": query, "top_k": top_k}
        if memory_type:
            args["memory_type"] = memory_type
        if project:
            args["project"] = project
        if tags:
            args["tags"] = tags

        result = self._call_tool("recall_enhanced", args)
        if "error" in result:
            return []

        # 解析返回的文本内容，提取记忆列表
        content_list = result.get("result", {}).get("content", [])
        if not content_list:
            return []

        text = content_list[0].get("text", "")
        return self._parse_recall_text(text)

    def _parse_recall_text(self, text: str) -> list[dict]:
        """解析 recall/recall_enhanced 返回的文本，提取记忆列表"""
        memories = []
        import re
        # 按 "（记忆 #N" 分割（中文括号格式）
        parts = re.split(r"（记忆 #\d+", text)
        for part in parts[1:]:  # 跳过第一个空部分
            # 提取记忆内容（在 "内容:" 之后）
            content_match = re.search(r"内容:\s*(.+?)(?:\n(?:八卦|类型|ID|RRF 融合度|相似度|得分)|$)", part)
            if content_match:
                content = content_match.group(1).strip()
                # 提取 RRF 融合度分数（recall_enhanced）或相似度分数（recall）
                score_match = re.search(r"RRF 融合度\s*([\d.]+)", part)
                if not score_match:
                    score_match = re.search(r"相似度\s*([\d.]+)", part)
                if not score_match:
                    score_match = re.search(r"得分\s*([\d.]+)", part)
                score = float(score_match.group(1)) if score_match else 0.0
                memories.append({"content": content, "score": score})
        return memories

    def health_check(self) -> bool:
        """检查服务端是否可用"""
        try:
            req = urllib.request.Request(f"{self.base_url}/health")
            with urllib.request.urlopen(req, timeout=5) as resp:
                return resp.status == 200
        except Exception:
            return False


# ============================================================
# 会话记忆注入器（LRC 服务端版）
# ============================================================

class SessionMemoryInjectorLRC:
    """将会话历史注入 LRC 服务端记忆库

    每个会话独立存储为一条记忆，支持并发注入提升速度。
    使用 project 字段隔离不同测试实例。
    """

    def __init__(self, client: LRCServerClient, project: str):
        self.client = client
        self.project = project

    def inject_all_sessions(self, haystack_sessions: list, haystack_dates: list, haystack_session_ids: list):
        """批量注入所有会话到 LRC 服务端（使用 batch_remember 提升速度）"""
        total = len(haystack_sessions)

        # 构建批量记忆列表
        batch_memories = []
        for session, date_str, sid in zip(haystack_sessions, haystack_dates, haystack_session_ids):
            # 将会话 turn 拼接为文本
            session_text_parts = []
            for turn in session:
                role = turn.get("role", "unknown")
                content = turn.get("content", "")
                session_text_parts.append(f"[{role}]: {content}")

            session_text = "\n".join(session_text_parts)

            # 限制内容长度，避免单条记忆过大
            max_len = 6000
            if len(session_text) > max_len:
                session_text = session_text[:max_len] + "\n...[内容截断]"

            batch_memories.append({
                "content": session_text,
                "memory_type": "conversation",
                "project": self.project,
                "tags": [sid, f"date:{date_str}"],
                "importance": 5,
            })

        # 分批发送，每批最多 20 条（避免单次请求过大导致超时）
        batch_size = 20
        for i in range(0, len(batch_memories), batch_size):
            chunk = batch_memories[i:i + batch_size]
            result = self.client.remember_batch(chunk)
            if "error" in result:
                print(f"  批量注入失败 (批次 {i//batch_size + 1}): {result['error']}")

        return total


# ============================================================
# LLM 问答器
# ============================================================

class LLMAnswerer:
    """使用 LLM 回答问题的组件"""

    def __init__(self, api_key: Optional[str] = None, api_base: Optional[str] = None, model: str = "deepseek-chat"):
        self.api_key = api_key or os.getenv("DEEPSEEK_API_KEY") or os.getenv("OPENAI_API_KEY")
        self.api_base = api_base or os.getenv("DEEPSEEK_API_BASE") or "https://api.deepseek.com/v1"
        self.model = model

    def _call_api(self, messages: list[dict], max_tokens: int = 200) -> str:
        """调用 LLM API"""
        if not self.api_key:
            raise ValueError("未设置 API Key")

        data = json.dumps({
            "model": self.model,
            "messages": messages,
            "max_tokens": max_tokens,
            "temperature": 0.1,
        }).encode("utf-8")

        req = urllib.request.Request(
            f"{self.api_base}/chat/completions",
            data=data,
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {self.api_key}",
            },
        )

        try:
            with urllib.request.urlopen(req, timeout=60) as resp:
                result = json.loads(resp.read().decode("utf-8"))
                return result["choices"][0]["message"]["content"].strip()
        except Exception as e:
            print(f"  API 调用失败: {e}")
            return "[API 调用失败]"

    def answer(self, question: str, context: list[dict], question_type: str = "single-session-user") -> str:
        """基于检索到的上下文回答问题"""
        if not context:
            return "I don't have enough information to answer this question."

        # 构建上下文
        context_text_parts = []
        for item in context:
            score = item.get("score", 0.0)
            content = item.get("content", "")
            context_text_parts.append(f"[相关度: {score:.2f}]\n{content}")

        context_text = "\n\n---\n\n".join(context_text_parts)

        system_prompt = """You are a helpful AI assistant with a long-term memory system.
You will be given retrieved memories from past conversations, and a question about the user.
Answer the question based ONLY on the information in the retrieved memories.
If the information is not in the memories, respond with: "I don't have enough information to answer this question."
Be concise and direct. Do not make up information."""

        user_prompt = f"""Here are the relevant memories retrieved from past conversations:

{context_text}

---
Question: {question}

Answer the question based on the memories above:"""

        messages = [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_prompt},
        ]

        return self._call_api(messages)


# ============================================================
# LLM 查询翻译器
# ============================================================

class LLMTranslator:
    """LRC LLM 查询翻译器 — 将自然语言问题翻译为检索关键词"""

    TRANSLATION_PROMPT = (
        "你是一个记忆检索助手。将用户的自然语言问题翻译成可能在对话历史中出现的关键词、实体和概念。"
        "提取问题中的核心实体（人名、地名、事物名）、关键动作、时间线索和修饰词。"
        "只返回逗号分隔的关键词列表，不要解释，不要多余文字。"
    )

    def __init__(self, api_key: Optional[str] = None, api_base: Optional[str] = None, model: str = "deepseek-chat"):
        self.api_key = api_key or os.getenv("DEEPSEEK_API_KEY") or os.getenv("OPENAI_API_KEY")
        self.api_base = api_base or os.getenv("DEEPSEEK_API_BASE") or "https://api.deepseek.com/v1"
        self.model = model

    def translate(self, query: str) -> list[str]:
        """将问题翻译为关键词列表"""
        if not self.api_key:
            return [query]

        data = json.dumps({
            "model": self.model,
            "messages": [
                {"role": "user", "content": f"{self.TRANSLATION_PROMPT}\n\n用户问题：{query}"}
            ],
            "max_tokens": 100,
            "temperature": 0.0,
        }).encode("utf-8")

        req = urllib.request.Request(
            f"{self.api_base}/chat/completions",
            data=data,
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {self.api_key}",
            },
        )

        try:
            with urllib.request.urlopen(req, timeout=10) as resp:
                result = json.loads(resp.read().decode("utf-8"))
                content = result["choices"][0]["message"]["content"].strip()
                keywords = [k.strip() for k in content.split(",") if k.strip()]
                return keywords if keywords else [query]
        except Exception as e:
            print(f"  LLM 翻译失败 (回退到原始查询): {e}")
            return [query]


# ============================================================
# 主运行流程
# ============================================================

def run_benchmark(
    data_path: str,
    output_path: str,
    max_instances: Optional[int] = None,
    start_from: int = 0,
    api_key: Optional[str] = None,
    api_base: Optional[str] = None,
    model: str = "deepseek-chat",
    top_k: int = 10,
    lrc_url: str = "http://localhost:3099",
):
    """运行完整的 LongMemEval 基准测试（LRC 全功能模式）

    Args:
        data_path: 数据集 JSON 文件路径
        output_path: 输出 hypothesis JSONL 文件路径
        max_instances: 最大测试实例数
        start_from: 从第几条开始
        api_key: DeepSeek API Key
        api_base: API 地址
        model: 模型名称
        top_k: 召回记忆条数
        lrc_url: LRC 服务端地址
    """
    # 检查 LRC 服务端
    client = LRCServerClient(lrc_url)
    if not client.health_check():
        print(f"错误: LRC 服务端不可用 ({lrc_url})")
        print("请先启动 LRC 服务端:")
        print("  code-memory-server --port 3099 --global --mode fast --llm-api openai:sk-xxx:deepseek-chat:https://api.deepseek.com/v1")
        sys.exit(1)
    print(f"LRC 服务端连接成功: {lrc_url}")

    print(f"加载数据集: {data_path}")
    with open(data_path, "r", encoding="utf-8") as f:
        dataset = json.load(f)
    print(f"共 {len(dataset)} 条测试实例")

    if max_instances:
        dataset = dataset[:max_instances]

    if start_from > 0:
        dataset = dataset[start_from:]
        print(f"从第 {start_from} 条继续，剩余 {len(dataset)} 条")

    # 初始化 LLM 组件
    llm = LLMAnswerer(api_key=api_key, api_base=api_base, model=model)
    translator = LLMTranslator(api_key=api_key, api_base=api_base, model=model)
    print(f"LLM 问答器: {model}")
    print(f"LLM 翻译器: {model}")

    # 断点续传
    results = []
    if start_from > 0 and os.path.exists(output_path):
        try:
            with open(output_path, "r", encoding="utf-8") as f:
                for line in f:
                    line = line.strip()
                    if line:
                        results.append(json.loads(line))
            print(f"已加载 {len(results)} 条已有结果")
        except Exception as e:
            print(f"加载已有结果失败: {e}")
            results = []

    type_counts = Counter()
    os.makedirs(os.path.dirname(output_path) or ".", exist_ok=True)

    for idx, instance in enumerate(dataset):
        question_id = instance["question_id"]
        question_type = instance["question_type"]
        question = instance["question"]
        answer = instance.get("answer", "")
        haystack_sessions = instance.get("haystack_sessions", [])
        haystack_dates = instance.get("haystack_dates", [])
        haystack_session_ids = instance.get("haystack_session_ids", [])

        type_counts[question_type] += 1

        print(f"\n[{idx+1}/{len(dataset)}] {question_id} ({question_type})")
        print(f"  问题: {question[:100]}...")
        print(f"  会话数: {len(haystack_sessions)}")

        # 每个实例使用独立的 project 隔离
        project = f"longmemeval_{question_id}"
        injector = SessionMemoryInjectorLRC(client, project)

        # 注入所有会话到 LRC 服务端
        t0 = time.time()
        total_memories = injector.inject_all_sessions(
            haystack_sessions, haystack_dates, haystack_session_ids
        )
        inject_time = time.time() - t0
        print(f"  记忆注入: {len(haystack_sessions)} 条会话记忆 (耗时 {inject_time:.2f}s)")

        # 检索相关记忆 — 使用 LRC 全功能双路检索
        t0 = time.time()

        # LLM 翻译查询
        keywords = translator.translate(question)
        combined_query = " ".join(keywords[:5])
        print(f"  LLM 翻译关键词: {keywords[:5]}...")

        # 使用 recall_enhanced（L1 快速 + L2 洛书深度 + RRF 融合）
        recalled = client.recall_enhanced(
            query=combined_query,
            top_k=top_k,
            memory_type="conversation",
            project=project,
        )
        recall_time = time.time() - t0
        print(f"  双路检索召回: {len(recalled)} 条 (耗时 {recall_time:.2f}s)")

        if recalled:
            top_score = recalled[0].get("score", 0.0)
            print(f"  最高 RRF 融合分: {top_score:.4f}")
            for i, item in enumerate(recalled[:3]):
                content = item.get("content", "")
                score = item.get("score", 0.0)
                snippet = content[:80].replace("\n", " ")
                print(f"    #{i+1}: [{score:.4f}] {snippet}...")

        # 生成答案
        hypothesis = llm.answer(question, recalled, question_type)
        print(f"  假设答案: {hypothesis[:150]}...")
        answer_str = str(answer) if not isinstance(answer, str) else answer
        print(f"  正确答案: {answer_str[:150]}...")

        results.append({
            "question_id": question_id,
            "hypothesis": hypothesis,
        })

        # 增量保存
        with open(output_path, "w", encoding="utf-8") as f:
            for r in results:
                f.write(json.dumps(r, ensure_ascii=False) + "\n")

    # 最终统计
    print("\n" + "=" * 60)
    print("LRC 全功能 LongMemEval 基准测试完成")
    print(f"  总实例数: {len(results)}")
    print(f"  输出文件: {output_path}")
    print(f"\n问题类型分布:")
    for t, c in type_counts.most_common():
        print(f"  {t}: {c}")

    print(f"\n下一步：使用 LongMemEval 评估脚本评分")
    print(f"  python evaluate_qa.py deepseek-chat {output_path} data/longmemeval_s_cleaned.json")


# ============================================================
# CLI
# ============================================================

if __name__ == "__main__":
    parser = argparse.ArgumentParser(
        description="LRC 全功能 LongMemEval 基准测试适配器（接入真实 LRC 服务端）",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
示例:
  # 完整运行（需要先启动 LRC 服务端）
  python lrc_full_longmemeval.py --data data/longmemeval_s_cleaned.json --output results/lrc_hypotheses_full_l2.jsonl

  # 限制条数测试
  python lrc_full_longmemeval.py --max 10

  # 指定 LRC 服务端地址
  python lrc_full_longmemeval.py --lrc-url http://localhost:3099
        """,
    )
    parser.add_argument("--data", default="data/longmemeval_s_cleaned.json", help="数据集 JSON 文件路径")
    parser.add_argument("--output", default="results/lrc_hypotheses_full_l2.jsonl", help="输出 hypothesis JSONL 文件路径")
    parser.add_argument("--max", type=int, default=None, help="最大测试实例数")
    parser.add_argument("--start", type=int, default=0, help="从第几条开始（断点续传）")
    parser.add_argument("--top-k", type=int, default=10, help="召回记忆条数")
    parser.add_argument("--model", default="deepseek-chat", help="LLM 模型名称")
    parser.add_argument("--api-key", default=None, help="API Key（默认使用环境变量）")
    parser.add_argument("--api-base", default=None, help="API 地址")
    parser.add_argument("--lrc-url", default="http://localhost:3099", help="LRC 服务端地址")

    args = parser.parse_args()

    print("=" * 60)
    print("LRC 全功能 LongMemEval 基准测试")
    print("=" * 60)
    print(f"  数据集: {args.data}")
    print(f"  输出: {args.output}")
    print(f"  模型: {args.model}")
    print(f"  召回数: {args.top_k}")
    print(f"  LRC 服务端: {args.lrc_url}")
    print(f"  功能层: L1(Fast Match) + L2(Luoshu 几何检索) + LLM(查询翻译+问答)")
    if args.max:
        print(f"  限制: {args.max} 条")
    if args.start:
        print(f"  起始: 第 {args.start} 条")
    print()

    run_benchmark(
        data_path=args.data,
        output_path=args.output,
        max_instances=args.max,
        start_from=args.start,
        api_key=args.api_key,
        api_base=args.api_base,
        model=args.model,
        top_k=args.top_k,
        lrc_url=args.lrc_url,
    )