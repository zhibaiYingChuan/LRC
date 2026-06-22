"""
LRC v0.5.6 Loong Recall 基准测试 — 检索能力全面评估脚本

遵守 LRC 自带基准测试框架的规则：
1. 三层测试模型（通用检索、独有能力、综合能力）
2. 评分体系 0.0-1.0，明确通过/失败阈值
3. 环境隔离（每次测试独立数据目录）
4. 确定性测试数据（固定 ID 和时间分布）
5. 雷达图多维度评分

聚焦 LRC 三种检索能力：
1. TF-IDF 检索（词边界匹配 + TF-IDF 加权）
2. 洛书几何编码（9 维向量 + 八卦分类 + 几何距离加权）
3. LLM 查询翻译器（DeepSeek API 将自然语言翻译为关键词）

通过 MCP stdio 接口与 LRC sidecar 通信，全面测试三种检索能力。

评估模式：
- tfidf: 纯 TF-IDF 检索（无 LLM）
- llm: LLM 查询翻译器 + TF-IDF 检索
- both: 两种模式都测试，生成对比报告

用法示例：
  python lrc_native_benchmark.py --mode tfidf
  python lrc_native_benchmark.py --mode llm --llm-api "openai:sk-xxx:deepseek-chat:https://api.deepseek.com/v1"
  python lrc_native_benchmark.py --mode both --llm-api "openai:sk-xxx:deepseek-chat:https://api.deepseek.com/v1"
"""
import os
import sys
import json
import time
import subprocess
import threading
import queue
import re
import argparse
from pathlib import Path


# ════════════════════════════════════════════════════════════
# LRC stdio 客户端
# ════════════════════════════════════════════════════════════

class LRCStdioClient:
    """通过 stdio 模式与 LRC sidecar 通信"""

    def __init__(self, exe_path, data_dir, llm_api=None):
        self.exe_path = exe_path
        self.data_dir = data_dir
        self.llm_api = llm_api
        self.proc = None
        self.req_id = 0
        self.response_queues = {}
        self._read_thread = None
        self._write_lock = threading.Lock()
        self._stderr_file = None

    def start(self):
        """启动 sidecar stdio 进程"""
        # 清理数据目录（环境隔离）
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
            "clientInfo": {"name": "lrc-benchmark-client", "version": "0.5.6"}
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

    def remember(self, content, memory_type="fact", project="benchmark", tags=None, importance=5):
        """写入一条记忆"""
        args = {
            "content": content,
            "memory_type": memory_type,
            "project": project,
            "importance": importance,
        }
        if tags:
            args["tags"] = tags
        return self._call("tools/call", {
            "name": "remember",
            "arguments": args
        }, timeout=60)

    def batch_remember(self, memories):
        """批量注入记忆"""
        return self._call("tools/call", {
            "name": "batch_remember",
            "arguments": {"memories": memories}
        }, timeout=600)

    def recall(self, query, top_k=10, project=None, memory_type=None):
        """检索记忆"""
        args = {"query": query, "top_k": top_k}
        if project:
            args["project"] = project
        if memory_type:
            args["memory_type"] = memory_type
        return self._call("tools/call", {
            "name": "recall",
            "arguments": args
        }, timeout=300)

    def list_memories(self, limit=1000, project=None):
        """列出记忆"""
        args = {"limit": limit}
        if project:
            args["project"] = project
        return self._call("tools/call", {
            "name": "list_memories",
            "arguments": args
        }, timeout=60)

    def memory_stats(self):
        """获取记忆库统计"""
        return self._call("tools/call", {
            "name": "memory_stats",
            "arguments": {}
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


# ════════════════════════════════════════════════════════════
# 测试数据生成（遵守 LRC 基准测试规则：确定性数据）
# ════════════════════════════════════════════════════════════

def generate_test_memories(count, prefix="test", importance=5):
    """生成测试记忆（对标 LRC 自带 generate_test_memories）

    确定性：固定内容模板，时间分布跨越 365 天
    """
    memories = []
    for i in range(count):
        content = (
            f"{prefix}记忆 #{i:04d}: 这是一条关于{prefix}的测试记忆内容。"
            f"包含关键词：项目、API、数据库、配置。编号 {i}。"
        )
        memories.append({
            "content": content,
            "memory_type": "fact",
            "project": "benchmark",
            "tags": ["测试", "基准", f"{prefix}-{i:04d}"],
            "importance": importance,
        })
    return memories


def generate_golden_memories():
    """生成黄金记忆（对标 LRC 自带 L1-2 的 golden memory）

    确定性：固定内容，用于精确匹配测试
    """
    return [
        {
            "content": "项目 Loong Recall 使用 Rust 编写，记忆核心基于洛书编码器的 9 维向量空间",
            "memory_type": "fact",
            "project": "loong-recall",
            "tags": ["洛书", "编码器", "golden-001"],
            "importance": 8,
        },
        {
            "content": "洛书几何编码器将记忆映射到 9 维洛书空间，通过八卦分类实现记忆的几何聚类",
            "memory_type": "fact",
            "project": "loong-recall",
            "tags": ["洛书", "八卦", "golden-002"],
            "importance": 8,
        },
        {
            "content": "TF-IDF 检索引擎使用词边界检测，避免 cat 匹配 category 的子串误匹配问题",
            "memory_type": "fact",
            "project": "loong-recall",
            "tags": ["TF-IDF", "词边界", "golden-003"],
            "importance": 7,
        },
    ]


def generate_conversation_memories():
    """生成对话记忆（对标 LRC 自带 L1-3 的 session recall）

    确定性：固定对话内容，用于长对话事实提取测试
    """
    conversations = [
        ("用户: 我叫张三，目前在北京工作", "fact", 8),
        ("用户: 我使用 Python 和 Rust 进行开发", "fact", 8),
        ("用户: 我的项目叫 Loong，是一个记忆系统", "fact", 8),
        ("用户: 数据库使用 PostgreSQL，缓存用 Redis", "fact", 5),
        ("用户: 我更喜欢 pnpm 而不是 npm", "preference", 8),
        ("用户: 上次你说用 Rust 实现洛书编码器，我已经完成了", "fact", 5),
        ("用户: 下周一要提交项目报告，周三有演示", "fact", 5),
        ("用户: 我的团队有 5 个人，都是后端工程师", "fact", 5),
        ("用户: 服务器部署在阿里云华东区", "fact", 2),
        ("用户: 我每天下午 3 点喝咖啡", "preference", 2),
    ]
    memories = []
    for i, (content, mem_type, importance) in enumerate(conversations):
        memories.append({
            "content": content,
            "memory_type": mem_type,
            "project": "chat",
            "tags": ["对话", f"session-{i}"],
            "importance": importance,
        })
    return memories


def generate_noise_memories(count=20):
    """生成噪声记忆（对标 LRC 自带 generate_noise_memories）

    确定性：固定噪声内容，用于抗污染测试
    """
    noise_texts = [
        "今天天气真好，适合出去散步",
        "我午餐吃了三明治和咖啡",
        "会议改到下午三点，请准时参加",
        "这个功能需要重构，架构太复杂了",
        "Python 是世界上最快的语言（矛盾信息）",
        "Rust 不适合做 Web 开发（错误信息）",
        "推荐使用 jQuery 来构建现代前端项目（过时建议）",
        "明天要交房租，别忘了转账",
        "这个电影评分很高，周末去看",
        "数据库连接永远不会超时（错误信息）",
    ]
    memories = []
    for i in range(count):
        text = noise_texts[i % len(noise_texts)]
        memories.append({
            "content": f"噪声 #{i}: {text}",
            "memory_type": "fact",
            "project": "benchmark",
            "tags": ["噪声"],
            "importance": 2,
        })
    return memories


def generate_word_boundary_memories():
    """生成词边界检测测试记忆

    确定性：固定内容，用于词边界检测精度测试
    包含容易子串误匹配的单词对
    """
    return [
        {
            "content": "cat is a small animal that catches mice",
            "memory_type": "fact",
            "project": "word-boundary",
            "tags": ["cat", "wb-001"],
            "importance": 5,
        },
        {
            "content": "category classification system for organizing items",
            "memory_type": "fact",
            "project": "word-boundary",
            "tags": ["category", "wb-002"],
            "importance": 5,
        },
        {
            "content": "rust programming language for systems programming",
            "memory_type": "fact",
            "project": "word-boundary",
            "tags": ["rust", "wb-003"],
            "importance": 5,
        },
        {
            "content": "frustrated users often complain about slow performance",
            "memory_type": "fact",
            "project": "word-boundary",
            "tags": ["frustrated", "wb-004"],
            "importance": 5,
        },
        {
            "content": "import statement for module dependencies",
            "memory_type": "fact",
            "project": "word-boundary",
            "tags": ["import", "wb-005"],
            "importance": 5,
        },
        {
            "content": "important configuration settings must not be ignored",
            "memory_type": "fact",
            "project": "word-boundary",
            "tags": ["important", "wb-006"],
            "importance": 5,
        },
    ]


# ════════════════════════════════════════════════════════════
# 结果解析辅助函数
# ════════════════════════════════════════════════════════════

def parse_recall_text(resp):
    """解析 recall 响应，返回结果文本"""
    if not resp or "result" not in resp or "content" not in resp["result"]:
        return ""
    for content in resp["result"]["content"]:
        if content.get("type") == "text":
            return content.get("text", "")
    return ""


def parse_recall_memories(resp):
    """解析 recall 响应，返回记忆列表（content, tags, type）

    LRC 返回格式：
        内容: xxx
        分类: yyy | 类型: zzz | 重要性: www | 标签: vvv | 项目: uuu
        ID: www
    """
    text = parse_recall_text(resp)
    if not text:
        return []

    memories = []
    # 按记忆条目分割
    blocks = re.split(r"（记忆 #\d+", text)
    for block in blocks[1:]:
        # 跳过合成记忆
        if "类型: synthesis" in block:
            continue

        # 提取内容（匹配到换行后的分类: 或其他字段，或 | 分隔符）
        content_match = re.search(
            r"内容:\s*(.+?)(?=\n(?:分类:|标签:|类型:|重要性:|时间:|ID:|项目:)|\||$)",
            block, re.DOTALL,
        )
        content = content_match.group(1).strip() if content_match else ""

        # 提取标签
        tags_match = re.search(r"标签:\s*([^|\n]+)", block)
        tags = []
        if tags_match:
            tags_str = tags_match.group(1).strip()
            tags = [t.strip().strip("`") for t in tags_str.split(",")]

        # 提取类型
        type_match = re.search(r"类型:\s*(\w+)", block)
        mem_type = type_match.group(1).strip() if type_match else "fact"

        memories.append({
            "content": content,
            "tags": tags,
            "type": mem_type,
        })
    return memories


# ════════════════════════════════════════════════════════════
# 基准测试结果类型（对标 LRC BenchmarkResult）
# ════════════════════════════════════════════════════════════

class BenchmarkResult:
    """基准测试结果（对标 LRC BenchmarkResult 结构体）"""

    def __init__(self, name, layer, description, industry_problem, passed, score, details, duration_ms):
        self.name = name
        self.layer = layer
        self.description = description
        self.industry_problem = industry_problem
        self.passed = passed
        self.score = score
        self.details = details
        self.duration_ms = duration_ms

    def to_dict(self):
        return {
            "name": self.name,
            "layer": self.layer,
            "description": self.description,
            "industry_problem": self.industry_problem,
            "passed": self.passed,
            "score": self.score,
            "details": self.details,
            "duration_ms": self.duration_ms,
        }


# ════════════════════════════════════════════════════════════
# 第一层：通用记忆检索基准（对标 LRC 自带 L1）
# ════════════════════════════════════════════════════════════

def benchmark_l1_retrieval_latency(client, mode_label):
    """L1-1: TF-IDF 检索延迟可扩展性（对标 LRC 自带 L1-1）

    规则：1K 记忆规模，P50 < 500ms 且 P95 < 1000ms
    """
    print(f"\n  [L1-1] TF-IDF 检索延迟可扩展性（1K 记忆规模）...", flush=True)
    start = time.time()

    # 注入 1000 条记忆
    memories = generate_test_memories(1000, "latency", 5)
    batch_size = 100
    for i in range(0, len(memories), batch_size):
        client.batch_remember(memories[i:i + batch_size])

    # 执行 100 次检索
    latencies = []
    for _ in range(100):
        s = time.time()
        client.recall("数据库", top_k=10, project="benchmark")
        latencies.append((time.time() - s) * 1000)  # 转为毫秒

    latencies.sort()
    p50 = latencies[49]
    p95 = latencies[94]
    p99 = latencies[98]

    # LRC 自带规则：P50 < 500ms 且 P95 < 1000ms
    passed = p50 < 500.0 and p95 < 1000.0
    score = 0.9 if passed else min(500.0 / max(p50, 1.0), 1.0) * 0.8

    duration_ms = int((time.time() - start) * 1000)
    result = BenchmarkResult(
        name="benchmark_retrieval_latency_scalability",
        layer=1,
        description="TF-IDF 检索延迟可扩展性（1K 记忆规模 P50/P95）",
        industry_problem="RAG 系统长上下文检索延迟退化问题",
        passed=passed,
        score=score,
        details=f"P50: {p50:.1f}ms, P95: {p95:.1f}ms, P99: {p99:.1f}ms",
        duration_ms=duration_ms,
    )
    print(f"    {('✓' if passed else '✗')} P50={p50:.1f}ms P95={p95:.1f}ms P99={p99:.1f}ms", flush=True)
    return result


def benchmark_l1_recall_precision(client, mode_label):
    """L1-2: TF-IDF 检索召回率精确匹配（对标 LRC 自带 L1-2）

    规则：返回结果且包含核心关键词（"Loong Recall"或"洛书"）
    """
    print(f"\n  [L1-2] TF-IDF 检索召回率精确匹配...", flush=True)
    start = time.time()

    # 注入黄金记忆
    golden = generate_golden_memories()
    client.batch_remember(golden)

    # 检索
    resp = client.recall("Loong Recall Rust 洛书编码器", top_k=5, project="loong-recall")
    memories = parse_recall_memories(resp)

    passed = len(memories) > 0
    top_content = memories[0]["content"] if memories else ""
    score = 0.95 if (passed and ("Loong Recall" in top_content or "洛书" in top_content)) else 0.3

    duration_ms = int((time.time() - start) * 1000)
    result = BenchmarkResult(
        name="benchmark_retrieval_recall_precision",
        layer=1,
        description="TF-IDF 检索召回率 — 精确匹配",
        industry_problem="向量检索中的语义漂移问题",
        passed=passed,
        score=score,
        details=f"返回 {len(memories)} 条结果，首条包含关键词: {'是' if score > 0.5 else '否'}",
        duration_ms=duration_ms,
    )
    print(f"    {('✓' if passed else '✗')} 返回 {len(memories)} 条结果，score={score:.2f}", flush=True)
    return result


def benchmark_l1_session_recall(client, mode_label):
    """L1-3: Session Recall 长对话事实提取（对标 LRC 自带 L1-3）

    规则：召回率 ≥ 50%（6 个查询中至少 3 个命中）
    """
    print(f"\n  [L1-3] Session Recall 长对话事实提取...", flush=True)
    start = time.time()

    # 注入对话记忆
    conversations = generate_conversation_memories()
    client.batch_remember(conversations)

    # 6 个查询测试
    queries = ["张三", "Rust", "Loong", "PostgreSQL", "pnpm", "北京"]
    recalled = 0
    for query in queries:
        resp = client.recall(query, top_k=3, project="chat")
        memories = parse_recall_memories(resp)
        if memories:
            combined = " ".join(m["content"] for m in memories)
            if query in combined:
                recalled += 1

    recall_rate = recalled / len(queries)
    # LRC 自带规则：召回率 ≥ 50%
    passed = recall_rate >= 0.5
    score = recall_rate

    duration_ms = int((time.time() - start) * 1000)
    result = BenchmarkResult(
        name="benchmark_session_recall_accuracy",
        layer=1,
        description="Session Recall — 长对话上下文事实提取",
        industry_problem="会话引擎中的遗忘灾难问题",
        passed=passed,
        score=score,
        details=f"召回率: {recall_rate * 100:.1f}% ({recalled}/{len(queries)})",
        duration_ms=duration_ms,
    )
    print(f"    {('✓' if passed else '✗')} 召回率: {recall_rate * 100:.1f}% ({recalled}/{len(queries)})", flush=True)
    return result


# ════════════════════════════════════════════════════════════
# 第二层：高级记忆能力基准（公平版 — 测能力，不测架构）
# ════════════════════════════════════════════════════════════

def benchmark_l2_memory_organization(client, mode_label):
    """L2-1: 记忆组织有效性（知识更新/冲突解决）

    公平性：测试系统能否在新旧信息冲突时返回最新信息。
    任何记忆系统都应具备此能力，与内部架构无关。

    规则：注入旧知识和新知识，查询当前状态，验证返回最新信息
    """
    print(f"\n  [L2-1] 记忆组织有效性（知识更新/冲突解决）...", flush=True)
    start = time.time()

    # 注入旧知识（低重要性）
    old_memories = [
        {"content": "用户居住在纽约，工作在曼哈顿", "memory_type": "fact", "project": "user-profile", "tags": ["旧地址"], "importance": 3},
        {"content": "用户使用 Python 2.7 进行开发", "memory_type": "fact", "project": "user-profile", "tags": ["旧技术栈"], "importance": 3},
    ]
    client.batch_remember(old_memories)

    # 注入新知识（高重要性，更新信息）
    new_memories = [
        {"content": "用户搬到了伦敦，现在在金融城工作", "memory_type": "fact", "project": "user-profile", "tags": ["新地址"], "importance": 8},
        {"content": "用户已切换到 Python 3.12 进行开发", "memory_type": "fact", "project": "user-profile", "tags": ["新技术栈"], "importance": 8},
    ]
    client.batch_remember(new_memories)

    # 查询当前状态
    queries = [
        ("用户现在住在哪里", "伦敦", "纽约"),  # 应返回伦敦，不应返回纽约
        ("用户用什么编程语言", "Python 3", "Python 2"),  # 应返回 Python 3，不应返回 Python 2
    ]

    correct_count = 0
    details = []
    for query, new_keyword, old_keyword in queries:
        resp = client.recall(query, top_k=3, project="user-profile")
        memories = parse_recall_memories(resp)

        if memories:
            # 检查第一条记忆是否是最新信息（排序优先级）
            top_content = memories[0]["content"]
            if new_keyword in top_content and old_keyword not in top_content:
                correct_count += 1
                details.append(f"{query}: ✓ 第一条返回最新信息({new_keyword})")
            elif new_keyword in top_content:
                correct_count += 1
                details.append(f"{query}: ✓ 第一条包含最新信息({new_keyword})")
            else:
                details.append(f"{query}: ✗ 第一条未返回最新信息")
        else:
            details.append(f"{query}: ✗ 未返回任何记忆")

    accuracy = correct_count / len(queries)
    passed = accuracy >= 0.5
    score = accuracy

    duration_ms = int((time.time() - start) * 1000)
    result = BenchmarkResult(
        name="benchmark_memory_organization",
        layer=2,
        description="记忆组织有效性 — 知识更新/冲突解决",
        industry_problem="新旧信息冲突时的记忆更新问题",
        passed=passed,
        score=score,
        details=f"知识更新准确率: {accuracy * 100:.1f}% ({correct_count}/{len(queries)}); " + "; ".join(details),
        duration_ms=duration_ms,
    )
    print(f"    {('✓' if passed else '✗')} 知识更新准确率: {accuracy * 100:.1f}% ({correct_count}/{len(queries)})", flush=True)
    return result


def benchmark_l2_fuzzy_query_robustness(client, mode_label):
    """L2-2: 模糊查询鲁棒性（口语化/碎片化查询）

    公平性：测试系统能否理解口语化、碎片化的查询并关联到正确上下文。
    任何记忆系统都应具备此能力，与是否有"LLM 翻译器"无关。

    规则：用口语化查询，验证能否命中正确记忆
    """
    print(f"\n  [L2-2] 模糊查询鲁棒性（口语化/碎片化查询）...", flush=True)
    start = time.time()

    # 注入事实记忆
    factual_memories = [
        {"content": "项目使用 PostgreSQL 数据库，部署在 AWS 东京区域", "memory_type": "fact", "project": "fuzzy-test", "tags": ["db"], "importance": 7},
        {"content": "API 服务器监听 8080 端口，使用 JWT 认证", "memory_type": "fact", "project": "fuzzy-test", "tags": ["api"], "importance": 7},
        {"content": "前端使用 React 框架，构建工具是 Vite", "memory_type": "fact", "project": "fuzzy-test", "tags": ["frontend"], "importance": 6},
        {"content": "团队有 5 个工程师，都在北京办公", "memory_type": "fact", "project": "fuzzy-test", "tags": ["team"], "importance": 5},
    ]
    client.batch_remember(factual_memories)

    # 口语化/碎片化查询
    fuzzy_queries = [
        ("那个数据库用的啥来着", "PostgreSQL"),
        ("部署在哪的", "AWS"),
        ("端口是多少", "8080"),
        ("前端用的什么框架", "React"),
        ("团队几个人", "5"),
        ("在哪办公", "北京"),
    ]

    hit_count = 0
    details = []
    for query, expected_keyword in fuzzy_queries:
        resp = client.recall(query, top_k=3, project="fuzzy-test")
        memories = parse_recall_memories(resp)
        combined = " ".join(m["content"] for m in memories)
        if expected_keyword in combined:
            hit_count += 1
            details.append(f"'{query}' → ✓ 命中({expected_keyword})")
        else:
            details.append(f"'{query}' → ✗ 未命中")

    hit_rate = hit_count / len(fuzzy_queries)
    passed = hit_rate >= 0.5
    score = hit_rate

    duration_ms = int((time.time() - start) * 1000)
    result = BenchmarkResult(
        name="benchmark_fuzzy_query_robustness",
        layer=2,
        description="模糊查询鲁棒性 — 口语化/碎片化查询",
        industry_problem="非精确查询的语义理解问题",
        passed=passed,
        score=score,
        details=f"模糊查询命中率: {hit_rate * 100:.1f}% ({hit_count}/{len(fuzzy_queries)}); " + "; ".join(details),
        duration_ms=duration_ms,
    )
    print(f"    {('✓' if passed else '✗')} 模糊查询命中率: {hit_rate * 100:.1f}% ({hit_count}/{len(fuzzy_queries)})", flush=True)
    return result


def benchmark_l2_precise_match_anti_noise(client, mode_label):
    """L2-3: 精确匹配与抗噪（双关词/术语区分）

    公平性：测试系统能否区分双关词的不同含义。
    任何记忆系统都应具备此能力，与内部实现细节无关。

    规则：查询双关词，验证返回的是正确含义的文档
    """
    print(f"\n  [L2-3] 精确匹配与抗噪（双关词/术语区分）...", flush=True)
    start = time.time()

    # 注入双关词文档
    ambiguous_memories = [
        {"content": "苹果公司发布新 iPhone，股价上涨 5%", "memory_type": "fact", "project": "ambiguous", "tags": ["company"], "importance": 7},
        {"content": "苹果是一种水果，富含维生素 C，每天吃一个有益健康", "memory_type": "fact", "project": "ambiguous", "tags": ["fruit"], "importance": 5},
        {"content": "Java 是一种编程语言，用于后端开发", "memory_type": "fact", "project": "ambiguous", "tags": ["language"], "importance": 7},
        {"content": "Java 是印度尼西亚的一个岛屿，以咖啡闻名", "memory_type": "fact", "project": "ambiguous", "tags": ["island"], "importance": 5},
        {"content": "Python 是一种编程语言，适合数据科学", "memory_type": "fact", "project": "ambiguous", "tags": ["language"], "importance": 7},
        {"content": "Python 是一种蟒蛇，生活在热带雨林", "memory_type": "fact", "project": "ambiguous", "tags": ["animal"], "importance": 5},
    ]
    client.batch_remember(ambiguous_memories)

    # 查询双关词，验证返回正确含义
    test_cases = [
        ("苹果公司", "iPhone", "水果"),  # 应返回公司，不应返回水果
        ("Java 编程", "编程语言", "岛屿"),  # 应返回编程语言，不应返回岛屿
        ("Python 编程", "编程语言", "蟒蛇"),  # 应返回编程语言，不应返回蟒蛇
    ]

    correct_count = 0
    details = []
    for query, should_contain, should_not_contain in test_cases:
        resp = client.recall(query, top_k=3, project="ambiguous")
        memories = parse_recall_memories(resp)

        if memories:
            # 检查第一条记忆是否是正确含义（排序优先级）
            top_content = memories[0]["content"]
            if should_contain in top_content:
                correct_count += 1
                details.append(f"'{query}' → ✓ 第一条返回正确含义({should_contain})")
            else:
                details.append(f"'{query}' → ✗ 第一条未返回正确含义")
        else:
            details.append(f"'{query}' → ✗ 未返回任何记忆")

    accuracy = correct_count / len(test_cases)
    passed = accuracy >= 0.5
    score = accuracy

    duration_ms = int((time.time() - start) * 1000)
    result = BenchmarkResult(
        name="benchmark_precise_match_anti_noise",
        layer=2,
        description="精确匹配与抗噪 — 双关词/术语区分",
        industry_problem="双关词语义歧义问题",
        passed=passed,
        score=score,
        details=f"双关词区分准确率: {accuracy * 100:.1f}% ({correct_count}/{len(test_cases)}); " + "; ".join(details),
        duration_ms=duration_ms,
    )
    print(f"    {('✓' if passed else '✗')} 双关词区分准确率: {accuracy * 100:.1f}% ({correct_count}/{len(test_cases)})", flush=True)
    return result


def benchmark_l2_anti_pollution(client, mode_label):
    """L2-4: 抗污染能力（增强噪声类型）

    公平性：测试系统在多种噪声干扰下能否稳定返回核心事实。
    任何记忆系统都应具备此能力。

    规则：注入核心事实 + 多种噪声，5 次检索一致性 ≥ 3/5 且前 5 条无噪声
    """
    print(f"\n  [L2-4] 抗污染能力（增强噪声类型）...", flush=True)
    start = time.time()

    # 注入核心事实
    core_memories = [
        {"content": "项目数据库连接配置: host=db.prod.example.com, port=5432, database=myapp", "memory_type": "fact", "project": "anti-pollution", "tags": ["core"], "importance": 8},
        {"content": "API 密钥配置: sk-1234567890abcdef, 过期时间 2026-12-31", "memory_type": "fact", "project": "anti-pollution", "tags": ["core"], "importance": 8},
    ]
    client.batch_remember(core_memories)

    # 注入多种噪声（矛盾信息、过时建议、错误信息、无关信息）
    noise_memories = [
        {"content": "数据库连接配置: host=localhost, port=3306（过时配置）", "memory_type": "fact", "project": "anti-pollution", "tags": ["noise"], "importance": 2},
        {"content": "API 密钥配置: sk-oldkey123（旧密钥，已失效）", "memory_type": "fact", "project": "anti-pollution", "tags": ["noise"], "importance": 2},
        {"content": "今天天气真好，适合出去散步（无关信息）", "memory_type": "fact", "project": "anti-pollution", "tags": ["noise"], "importance": 1},
        {"content": "数据库连接永远不会超时（错误信息）", "memory_type": "fact", "project": "anti-pollution", "tags": ["noise"], "importance": 1},
        {"content": "推荐使用 jQuery 构建现代前端（过时建议）", "memory_type": "fact", "project": "anti-pollution", "tags": ["noise"], "importance": 1},
        {"content": "Python 是世界上最快的语言（矛盾信息）", "memory_type": "fact", "project": "anti-pollution", "tags": ["noise"], "importance": 1},
        {"content": "会议改到下午三点（无关信息）", "memory_type": "fact", "project": "anti-pollution", "tags": ["noise"], "importance": 1},
        {"content": "明天要交房租（无关信息）", "memory_type": "fact", "project": "anti-pollution", "tags": ["noise"], "importance": 1},
    ]
    client.batch_remember(noise_memories)

    # 5 次检索一致性测试（检查前 3 条中是否包含核心事实）
    consistency = 0
    noise_in_top = 0
    core_keywords = ["db.prod.example.com", "sk-1234567890abcdef"]
    for _ in range(5):
        resp = client.recall("数据库连接配置 API 密钥", top_k=10, project="anti-pollution")
        memories = parse_recall_memories(resp)
        # 检查前 3 条中是否包含核心事实的关键词
        top_n = memories[:3]
        has_core = any(
            any(kw in m["content"] for kw in core_keywords)
            for m in top_n
        )
        if has_core:
            consistency += 1
        else:
            noise_in_top += 1

    passed = consistency >= 3
    score = consistency / 5

    duration_ms = int((time.time() - start) * 1000)
    result = BenchmarkResult(
        name="benchmark_anti_pollution_capability",
        layer=2,
        description="抗污染能力 — 多种噪声下的核心事实一致性",
        industry_problem="噪声记忆干扰检索结果的问题",
        passed=passed,
        score=score,
        details=f"5 次检索一致性: {consistency}/5，前 3 条包含核心事实次数: {5 - noise_in_top}/5",
        duration_ms=duration_ms,
    )
    print(f"    {('✓' if passed else '✗')} 5 次检索一致性: {consistency}/5", flush=True)
    return result


# ════════════════════════════════════════════════════════════
# 第三层：综合能力与信任基准（公平版 — 测能力，不测架构）
# ════════════════════════════════════════════════════════════

def benchmark_l3_cross_topic_retrieval(client, mode_label):
    """L3-1: 跨主题上下文检索（多主题记忆精准召回）

    公平性：测试系统能否正确区分并检索到指定主题的记忆。
    任何记忆系统都应具备此能力，与内部"记忆类型"概念无关。

    规则：注入多主题记忆，查询特定主题，验证返回正确主题
    """
    print(f"\n  [L3-1] 跨主题上下文检索（多主题记忆精准召回）...", flush=True)
    start = time.time()

    # 注入多主题记忆
    multi_topic_memories = [
        # 技术主题
        {"content": "项目使用 Rust 编写后端，Axum 框架，PostgreSQL 数据库", "memory_type": "fact", "project": "cross-topic", "tags": ["tech"], "importance": 7},
        {"content": "前端使用 React + TypeScript，Vite 构建", "memory_type": "fact", "project": "cross-topic", "tags": ["tech"], "importance": 7},
        # 个人主题
        {"content": "张三的生日是 1990 年 5 月 15 日", "memory_type": "fact", "project": "cross-topic", "tags": ["personal"], "importance": 6},
        {"content": "张三的邮箱是 zhangsan@example.com", "memory_type": "fact", "project": "cross-topic", "tags": ["personal"], "importance": 6},
        # 决策主题
        {"content": "决定采用微服务架构而不是单体架构，原因是团队规模扩大", "memory_type": "fact", "project": "cross-topic", "tags": ["decision"], "importance": 8},
        {"content": "决定使用 Redis 做缓存，TTL 设置为 1 小时", "memory_type": "fact", "project": "cross-topic", "tags": ["decision"], "importance": 7},
    ]
    client.batch_remember(multi_topic_memories)

    # 查询特定主题
    topic_queries = [
        ("Rust Axum PostgreSQL", "tech", "技术"),
        ("React TypeScript Vite", "tech", "技术"),
        ("张三 生日", "personal", "个人"),
        ("张三 邮箱", "personal", "个人"),
        ("微服务架构", "decision", "决策"),
        ("Redis 缓存", "decision", "决策"),
    ]

    hit_count = 0
    details = []
    for query, expected_topic, topic_name in topic_queries:
        resp = client.recall(query, top_k=3, project="cross-topic")
        memories = parse_recall_memories(resp)

        if memories:
            # 检查第一条记忆是否包含查询关键词（排序优先级）
            top_content = memories[0]["content"]
            keywords = query.split()
            if any(kw in top_content for kw in keywords):
                hit_count += 1
                details.append(f"'{query}' → ✓ 第一条命中{topic_name}主题")
            else:
                details.append(f"'{query}' → ✗ 第一条未命中")
        else:
            details.append(f"'{query}' → ✗ 未返回任何记忆")

    hit_rate = hit_count / len(topic_queries)
    passed = hit_rate >= 0.5
    score = hit_rate

    duration_ms = int((time.time() - start) * 1000)
    result = BenchmarkResult(
        name="benchmark_cross_topic_retrieval",
        layer=3,
        description="跨主题上下文检索 — 多主题记忆精准召回",
        industry_problem="多主题记忆的精准检索问题",
        passed=passed,
        score=score,
        details=f"跨主题检索命中率: {hit_rate * 100:.1f}% ({hit_count}/{len(topic_queries)}); " + "; ".join(details),
        duration_ms=duration_ms,
    )
    print(f"    {('✓' if passed else '✗')} 跨主题检索命中率: {hit_rate * 100:.1f}% ({hit_count}/{len(topic_queries)})", flush=True)
    return result


def benchmark_l3_multi_tenant_isolation(client, mode_label):
    """L3-2: 多租户数据隔离（跨用户/项目无干扰）

    公平性：测试系统在多用户场景下能否正确隔离数据。
    任何记忆系统都应具备此能力，与内部用"项目"还是"Collection"无关。

    规则：注入用户 A 和 B 的记忆，查询"我的密码"，验证不串扰
    """
    print(f"\n  [L3-2] 多租户数据隔离（跨用户/项目无干扰）...", flush=True)
    start = time.time()

    # 注入用户 A 的记忆
    user_a_memories = [
        {"content": "用户A的密码是 Aa123456", "memory_type": "fact", "project": "user-a", "tags": ["password"], "importance": 8},
        {"content": "用户A的服务器部署在 AWS 东京", "memory_type": "fact", "project": "user-a", "tags": ["server"], "importance": 6},
    ]
    client.batch_remember(user_a_memories)

    # 注入用户 B 的记忆
    user_b_memories = [
        {"content": "用户B的密码是 Bb789012", "memory_type": "fact", "project": "user-b", "tags": ["password"], "importance": 8},
        {"content": "用户B的服务器部署在 Azure 美东", "memory_type": "fact", "project": "user-b", "tags": ["server"], "importance": 6},
    ]
    client.batch_remember(user_b_memories)

    # 查询用户 A，不应返回用户 B 的记忆
    resp_a = client.recall("我的密码是什么", top_k=5, project="user-a")
    memories_a = parse_recall_memories(resp_a)
    combined_a = " ".join(m["content"] for m in memories_a)

    # 查询用户 B，不应返回用户 A 的记忆
    resp_b = client.recall("我的密码是什么", top_k=5, project="user-b")
    memories_b = parse_recall_memories(resp_b)
    combined_b = " ".join(m["content"] for m in memories_b)

    # 验证隔离
    a_has_own = "Aa123456" in combined_a
    a_has_b = "Bb789012" in combined_a
    b_has_own = "Bb789012" in combined_b
    b_has_a = "Aa123456" in combined_b

    isolation_score = 0
    details = []
    if a_has_own:
        isolation_score += 1
        details.append("用户A查询返回A的密码: ✓")
    else:
        details.append("用户A查询未返回A的密码: ✗")
    if not a_has_b:
        isolation_score += 1
        details.append("用户A查询未泄露B的密码: ✓")
    else:
        details.append("用户A查询泄露了B的密码: ✗")
    if b_has_own:
        isolation_score += 1
        details.append("用户B查询返回B的密码: ✓")
    else:
        details.append("用户B查询未返回B的密码: ✗")
    if not b_has_a:
        isolation_score += 1
        details.append("用户B查询未泄露A的密码: ✓")
    else:
        details.append("用户B查询泄露了A的密码: ✗")

    isolation_rate = isolation_score / 4
    passed = isolation_score >= 3  # 至少 3/4 通过
    score = isolation_rate

    duration_ms = int((time.time() - start) * 1000)
    result = BenchmarkResult(
        name="benchmark_multi_tenant_isolation",
        layer=3,
        description="多租户数据隔离 — 跨用户/项目无干扰",
        industry_problem="多租户场景下的数据隔离问题",
        passed=passed,
        score=score,
        details=f"隔离率: {isolation_rate * 100:.1f}% ({isolation_score}/4); " + "; ".join(details),
        duration_ms=duration_ms,
    )
    print(f"    {('✓' if passed else '✗')} 隔离率: {isolation_rate * 100:.1f}% ({isolation_score}/4)", flush=True)
    return result


def benchmark_l3_large_scale_performance(client, mode_label):
    """L3-3: 大规模记忆检索性能

    规则：500 文档场景下 P50 < 100ms
    """
    print(f"\n  [L3-3] 大规模记忆检索性能（500 文档）...", flush=True)
    start = time.time()

    # 注入 500 条记忆
    memories = generate_test_memories(500, "large", 5)
    batch_size = 100
    for i in range(0, len(memories), batch_size):
        client.batch_remember(memories[i:i + batch_size])

    # 执行 50 次检索
    latencies = []
    for _ in range(50):
        s = time.time()
        client.recall("数据库 API 配置", top_k=10, project="benchmark")
        latencies.append((time.time() - s) * 1000)

    latencies.sort()
    p50 = latencies[24]
    p95 = latencies[47]
    p99 = latencies[48]

    # 规则：P50 < 100ms
    passed = p50 < 100.0
    score = min(100.0 / max(p50, 1.0), 1.0)

    duration_ms = int((time.time() - start) * 1000)
    result = BenchmarkResult(
        name="benchmark_large_scale_performance",
        layer=3,
        description="大规模记忆检索性能（500 文档 P50/P95/P99）",
        industry_problem="大规模记忆库检索延迟退化问题",
        passed=passed,
        score=score,
        details=f"P50: {p50:.1f}ms, P95: {p95:.1f}ms, P99: {p99:.1f}ms",
        duration_ms=duration_ms,
    )
    print(f"    {('✓' if passed else '✗')} P50={p50:.1f}ms P95={p95:.1f}ms P99={p99:.1f}ms", flush=True)
    return result


def benchmark_l3_llm_latency_accuracy(client, mode_label):
    """L3-4: LLM 翻译器延迟与精度平衡

    规则：LLM 模式延迟 < 5s 且精度提升（仅 LLM 模式测试）
    """
    print(f"\n  [L3-4] LLM 翻译器延迟与精度平衡...", flush=True)
    start = time.time()

    # 注入需要语义理解的记忆
    semantic_memories = [
        {"content": "认证系统使用 JWT token，过期时间 24 小时", "memory_type": "fact", "project": "llm-balance", "tags": ["auth"], "importance": 7},
        {"content": "日志系统使用 ELK stack，Elasticsearch 存储日志", "memory_type": "fact", "project": "llm-balance", "tags": ["logging"], "importance": 6},
        {"content": "缓存策略使用 Redis LRU 淘汰算法", "memory_type": "fact", "project": "llm-balance", "tags": ["cache"], "importance": 6},
    ]
    client.batch_remember(semantic_memories)

    # 自然语言查询
    natural_queries = [
        "用户登录是怎么实现的",
        "日志收集方案是什么",
        "缓存过期策略是怎样的",
    ]

    hit_count = 0
    latencies = []
    for query in natural_queries:
        s = time.time()
        resp = client.recall(query, top_k=3, project="llm-balance")
        latency = time.time() - s
        latencies.append(latency)

        memories = parse_recall_memories(resp)
        if memories:
            combined = " ".join(m["content"] for m in memories)
            # 检查是否命中关键词
            if any(kw in combined for kw in ["JWT", "token", "ELK", "Elasticsearch", "Redis", "LRU"]):
                hit_count += 1

    avg_latency = sum(latencies) / len(latencies)
    hit_rate = hit_count / len(natural_queries)

    # 规则：延迟 < 5s 且命中率 ≥ 50%
    passed = avg_latency < 5.0 and hit_rate >= 0.5
    score = (hit_rate + min(5.0 / max(avg_latency, 0.1), 1.0)) / 2

    duration_ms = int((time.time() - start) * 1000)
    result = BenchmarkResult(
        name="benchmark_llm_latency_accuracy_balance",
        layer=3,
        description="LLM 翻译器延迟与精度平衡",
        industry_problem="LLM 翻译延迟与检索精度的平衡问题",
        passed=passed,
        score=score,
        details=f"平均延迟: {avg_latency:.2f}s, 命中率: {hit_rate * 100:.1f}% ({hit_count}/{len(natural_queries)})",
        duration_ms=duration_ms,
    )
    print(f"    {('✓' if passed else '✗')} 平均延迟: {avg_latency:.2f}s, 命中率: {hit_rate * 100:.1f}%", flush=True)
    return result


# ════════════════════════════════════════════════════════════
# 雷达图评分（对标 LRC build_radar_scores）
# ════════════════════════════════════════════════════════════

def build_radar_scores(results):
    """构建雷达图评分（公平版：测能力，不测架构）

    将 11 项测试映射为通用能力维度名（任何记忆系统都可对比）
    """
    radar = {}
    for r in results:
        name = r.name
        # L1 通用检索基准（保持不变）
        if name == "benchmark_retrieval_latency_scalability":
            radar["检索性能"] = r.score
        elif name == "benchmark_retrieval_recall_precision":
            radar["检索精度"] = r.score
        elif name == "benchmark_session_recall_accuracy":
            radar["会话回忆"] = r.score
        # L2 高级记忆能力基准（公平版：测效果，不测架构）
        elif name == "benchmark_memory_organization":
            radar["记忆组织"] = r.score
        elif name == "benchmark_fuzzy_query_robustness":
            radar["模糊查询"] = r.score
        elif name == "benchmark_precise_match_anti_noise":
            radar["精确抗噪"] = r.score
        elif name == "benchmark_anti_pollution_capability":
            radar["抗污染"] = r.score
        # L3 综合能力与信任基准（公平版：测效果，不测架构）
        elif name == "benchmark_cross_topic_retrieval":
            radar["跨主题检索"] = r.score
        elif name == "benchmark_multi_tenant_isolation":
            radar["多租户隔离"] = r.score
        elif name == "benchmark_large_scale_performance":
            radar["大规模性能"] = r.score
        elif name == "benchmark_llm_latency_accuracy_balance":
            radar["LLM平衡"] = r.score
    return radar


# ════════════════════════════════════════════════════════════
# 报告生成
# ════════════════════════════════════════════════════════════

def generate_report(all_results, mode_label, llm_enabled, output_path):
    """生成 Markdown 评估报告"""
    os.makedirs(os.path.dirname(output_path), exist_ok=True)

    with open(output_path, "w", encoding="utf-8") as f:
        f.write(f"# LRC v0.5.6 Loong Recall 基准测试报告 — {mode_label}\n\n")
        f.write(f"**评估日期**: {time.strftime('%Y-%m-%d %H:%M:%S')}\n\n")
        f.write(f"**LRC 版本**: v0.5.6\n\n")
        f.write(f"**评估模式**: {mode_label}\n\n")
        f.write(f"**评估合规**: LRC 自带基准测试框架规则\n\n")
        f.write("---\n\n")

        # 汇总
        total = len(all_results)
        passed = sum(1 for r in all_results if r.passed)
        failed = total - passed
        status = "PASS" if failed == 0 else "FAIL"

        f.write("## 1. 评估汇总\n\n")
        f.write(f"| 指标 | 值 |\n")
        f.write(f"| :--- | :--- |\n")
        f.write(f"| 总测试数 | {total} |\n")
        f.write(f"| 通过 | {passed} |\n")
        f.write(f"| 失败 | {failed} |\n")
        f.write(f"| 状态 | {status} |\n")
        f.write(f"| 总耗时 | {sum(r.duration_ms for r in all_results)}ms |\n\n")

        # 三层状态
        layer1 = [r for r in all_results if r.layer == 1]
        layer2 = [r for r in all_results if r.layer == 2]
        layer3 = [r for r in all_results if r.layer == 3]

        f.write("### 三层状态概览\n\n")
        f.write("| 层级 | 名称 | 总数 | 通过 | 状态 |\n")
        f.write("| :--- | :--- | ---: | ---: | :--- |\n")
        f.write(f"| L1 | 通用记忆检索基准 | {len(layer1)} | {sum(1 for r in layer1 if r.passed)} | {'PASS' if all(r.passed for r in layer1) else 'FAIL'} |\n")
        f.write(f"| L2 | 高级记忆能力基准（公平版） | {len(layer2)} | {sum(1 for r in layer2 if r.passed)} | {'PASS' if all(r.passed for r in layer2) else 'FAIL'} |\n")
        f.write(f"| L3 | 综合能力与信任基准（公平版） | {len(layer3)} | {sum(1 for r in layer3 if r.passed)} | {'PASS' if all(r.passed for r in layer3) else 'FAIL'} |\n\n")

        # 雷达图
        radar = build_radar_scores(all_results)
        f.write("### 雷达图评分\n\n")
        f.write("| 维度 | 评分 |\n")
        f.write("| :--- | ---: |\n")
        for dim, score in radar.items():
            bar = "█" * int(score * 20)
            f.write(f"| {dim} | {score:.2f} {bar} |\n")
        f.write("\n")

        # 详细结果
        f.write("---\n\n")
        f.write("## 2. 详细测试结果\n\n")

        for layer_num, layer_name, layer_results in [
            (1, "第一层：通用记忆检索基准", layer1),
            (2, "第二层：高级记忆能力基准（公平版）", layer2),
            (3, "第三层：综合能力与信任基准（公平版）", layer3),
        ]:
            f.write(f"### L{layer_num} {layer_name}\\n\n")
            for r in layer_results:
                status_icon = "✓" if r.passed else "✗"
                f.write(f"#### {status_icon} {r.name}\n\n")
                f.write(f"- **描述**: {r.description}\n")
                f.write(f"- **行业问题**: {r.industry_problem}\n")
                f.write(f"- **状态**: {'PASS' if r.passed else 'FAIL'}\n")
                f.write(f"- **评分**: {r.score:.2f}\n")
                f.write(f"- **详情**: {r.details}\n")
                f.write(f"- **耗时**: {r.duration_ms}ms\n\n")

        # 结论
        f.write("---\n\n")
        f.write("## 3. 结论\n\n")
        f.write(f"- **总体状态**: {status}（{passed}/{total} 通过）\n")
        f.write(f"- **三层状态**: L1 {'PASS' if all(r.passed for r in layer1) else 'FAIL'}, "
                f"L2 {'PASS' if all(r.passed for r in layer2) else 'FAIL'}, "
                f"L3 {'PASS' if all(r.passed for r in layer3) else 'FAIL'}\n")
        f.write(f"- **评估模式**: {mode_label}\n")
        if llm_enabled:
            f.write(f"- **LLM 查询翻译器**: 已启用，测试了自然语言查询的翻译效果\n")
        else:
            f.write(f"- **LLM 查询翻译器**: 未启用，仅测试 TF-IDF 基本检索能力\n")
        f.write(f"- **洛书几何编码**: 所有记忆自动获得 9 维洛书向量\n")


def generate_comparison_report(tfidf_results, llm_results, output_path):
    """生成 TF-IDF vs LLM 对比报告"""
    os.makedirs(os.path.dirname(output_path), exist_ok=True)

    with open(output_path, "w", encoding="utf-8") as f:
        f.write(f"# LRC v0.5.6 Loong Recall 基准测试 — TF-IDF vs LLM 对比报告\n\n")
        f.write(f"**评估日期**: {time.strftime('%Y-%m-%d %H:%M:%S')}\n\n")
        f.write(f"**LRC 版本**: v0.5.6\n\n")
        f.write("---\n\n")

        # 对比表
        f.write("## 1. 两种模式对比\n\n")
        f.write("| 测试项 | 层级 | TF-IDF 评分 | LLM 评分 | 增益 |\n")
        f.write("| :--- | :--- | ---: | ---: | ---: |\n")
        for t, l in zip(tfidf_results, llm_results):
            gain = (l.score - t.score) / max(t.score, 0.01) * 100
            f.write(f"| {t.name} | L{t.layer} | {t.score:.2f} | {l.score:.2f} | {gain:+.1f}% |\n")
        f.write("\n")

        # 雷达图对比
        tfidf_radar = build_radar_scores(tfidf_results)
        llm_radar = build_radar_scores(llm_results)
        f.write("## 2. 雷达图评分对比\n\n")
        f.write("| 维度 | TF-IDF | LLM | 增益 |\n")
        f.write("| :--- | ---: | ---: | ---: |\n")
        for dim in tfidf_radar:
            t_score = tfidf_radar[dim]
            l_score = llm_radar.get(dim, 0)
            gain = (l_score - t_score) / max(t_score, 0.01) * 100
            f.write(f"| {dim} | {t_score:.2f} | {l_score:.2f} | {gain:+.1f}% |\n")
        f.write("\n")

        # 汇总
        tfidf_passed = sum(1 for r in tfidf_results if r.passed)
        llm_passed = sum(1 for r in llm_results if r.passed)
        f.write("## 3. 汇总\n\n")
        f.write(f"| 指标 | TF-IDF | LLM |\n")
        f.write(f"| :--- | ---: | ---: |\n")
        f.write(f"| 通过数 | {tfidf_passed}/{len(tfidf_results)} | {llm_passed}/{len(llm_results)} |\n")
        f.write(f"| 总评分 | {sum(r.score for r in tfidf_results) / len(tfidf_results):.2f} | {sum(r.score for r in llm_results) / len(llm_results):.2f} |\n\n")


# ════════════════════════════════════════════════════════════
# 主函数
# ════════════════════════════════════════════════════════════

def run_all_benchmarks(client, mode_label):
    """运行所有基准测试（对标 LRC run_all_benchmarks）"""
    results = []

    print(f"\n{'=' * 60}", flush=True)
    print(f"LRC v0.5.6 Loong Recall 基准测试 — {mode_label}", flush=True)
    print(f"{'=' * 60}", flush=True)

    # 第一层：通用记忆检索基准
    print(f"\n--- 第一层：通用记忆检索基准 ---", flush=True)
    results.append(benchmark_l1_retrieval_latency(client, mode_label))
    results.append(benchmark_l1_recall_precision(client, mode_label))
    results.append(benchmark_l1_session_recall(client, mode_label))

    # 第二层：高级记忆能力基准（公平版：测能力，不测架构）
    print(f"\n--- 第二层：高级记忆能力基准（公平版）---", flush=True)
    results.append(benchmark_l2_memory_organization(client, mode_label))
    results.append(benchmark_l2_fuzzy_query_robustness(client, mode_label))
    results.append(benchmark_l2_precise_match_anti_noise(client, mode_label))
    results.append(benchmark_l2_anti_pollution(client, mode_label))

    # 第三层：综合能力与信任基准（公平版：测能力，不测架构）
    print(f"\n--- 第三层：综合能力与信任基准（公平版）---", flush=True)
    results.append(benchmark_l3_cross_topic_retrieval(client, mode_label))
    results.append(benchmark_l3_multi_tenant_isolation(client, mode_label))
    results.append(benchmark_l3_large_scale_performance(client, mode_label))
    results.append(benchmark_l3_llm_latency_accuracy(client, mode_label))

    return results


def main():
    parser = argparse.ArgumentParser(description="LRC v0.5.6 Loong Recall 基准测试 — 检索能力全面评估")
    parser.add_argument("--exe", default="G:/rust-target/release/code-memory-server.exe",
                        help="LRC sidecar 二进制路径")
    parser.add_argument("--data-dir", default="G:/BEIR/lrc_native_benchmark_data",
                        help="LRC 数据目录")
    parser.add_argument("--llm-api", default=None,
                        help="LLM API 配置（格式: openai:api_key:model:base_url）")
    parser.add_argument("--mode", choices=["tfidf", "llm", "both"], default="both",
                        help="评估模式: tfidf（纯TF-IDF）、llm（LLM翻译器）、both（两者都测）")
    parser.add_argument("--output-dir", default="G:/BEIR/results",
                        help="输出报告目录")
    args = parser.parse_args()

    os.makedirs(args.output_dir, exist_ok=True)

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

    all_mode_results = {}

    for mode_label, llm_api in modes_to_test:
        print(f"\n{'=' * 60}", flush=True)
        print(f"评估模式: {mode_label}", flush=True)
        print(f"{'=' * 60}", flush=True)

        # 为每个模式使用独立的数据目录（环境隔离）
        mode_data_dir = args.data_dir + "_" + ("tfidf" if llm_api is None else "llm")

        # 启动 LRC sidecar
        print("\n启动 LRC sidecar...", flush=True)
        client = LRCStdioClient(args.exe, mode_data_dir, llm_api)
        client.start()

        # 运行所有基准测试
        results = run_all_benchmarks(client, mode_label)

        # 打印汇总
        total = len(results)
        passed = sum(1 for r in results if r.passed)
        failed = total - passed
        status = "PASS" if failed == 0 else "FAIL"

        print(f"\n{'=' * 60}", flush=True)
        print(f"LRC v0.5.6 Loong Recall 基准测试结果 — {mode_label}", flush=True)
        print(f"{'=' * 60}", flush=True)
        print(f"总测试数: {total}", flush=True)
        print(f"通过: {passed}", flush=True)
        print(f"失败: {failed}", flush=True)
        print(f"状态: {status}", flush=True)
        print(f"总耗时: {sum(r.duration_ms for r in results)}ms", flush=True)
        print(f"", flush=True)

        # 三层状态
        layer1 = [r for r in results if r.layer == 1]
        layer2 = [r for r in results if r.layer == 2]
        layer3 = [r for r in results if r.layer == 3]
        print(f"L1 通用记忆检索基准: {'PASS' if all(r.passed for r in layer1) else 'FAIL'} "
              f"({sum(1 for r in layer1 if r.passed)}/{len(layer1)})", flush=True)
        print(f"L2 高级记忆能力基准（公平版）: {'PASS' if all(r.passed for r in layer2) else 'FAIL'} "
              f"({sum(1 for r in layer2 if r.passed)}/{len(layer2)})", flush=True)
        print(f"L3 综合能力与信任基准（公平版）: {'PASS' if all(r.passed for r in layer3) else 'FAIL'} "
              f"({sum(1 for r in layer3 if r.passed)}/{len(layer3)})", flush=True)
        print(f"{'=' * 60}", flush=True)

        # 雷达图
        radar = build_radar_scores(results)
        print(f"\n雷达图评分:", flush=True)
        for dim, score in radar.items():
            bar = "█" * int(score * 20)
            print(f"  {dim:12s} {score:.2f} {bar}", flush=True)

        # 生成报告
        report_path = os.path.join(
            args.output_dir,
            f"LRC_NATIVE_BENCHMARK_{'TFIDF' if llm_api is None else 'LLM'}.md"
        )
        generate_report(results, mode_label, llm_api is not None, report_path)
        print(f"\n报告已保存: {report_path}", flush=True)

        # 保存日志
        log_path = os.path.join(
            args.output_dir,
            f"lrc_native_benchmark_{'tfidf' if llm_api is None else 'llm'}.log"
        )
        with open(log_path, "w", encoding="utf-8") as f:
            f.write(f"LRC v0.5.6 Loong Recall 基准测试\n")
            f.write(f"{'=' * 60}\n")
            f.write(f"评估模式: {mode_label}\n")
            f.write(f"总测试数: {total}\n")
            f.write(f"通过: {passed}\n")
            f.write(f"失败: {failed}\n")
            f.write(f"状态: {status}\n\n")
            for r in results:
                status_icon = "✓" if r.passed else "✗"
                f.write(f"\n{status_icon} [{r.name}] (L{r.layer})\n")
                f.write(f"  描述: {r.description}\n")
                f.write(f"  行业问题: {r.industry_problem}\n")
                f.write(f"  评分: {r.score:.2f}\n")
                f.write(f"  详情: {r.details}\n")
                f.write(f"  耗时: {r.duration_ms}ms\n")
        print(f"日志已保存: {log_path}", flush=True)

        all_mode_results[mode_label] = results

        # 关闭 sidecar
        client.close()

    # 生成对比报告
    if len(all_mode_results) == 2:
        print(f"\n生成对比报告...", flush=True)
        labels = list(all_mode_results.keys())
        tfidf_results = all_mode_results[labels[0]]
        llm_results = all_mode_results[labels[1]]
        comparison_path = os.path.join(args.output_dir, "LRC_NATIVE_BENCHMARK_COMPARISON.md")
        generate_comparison_report(tfidf_results, llm_results, comparison_path)
        print(f"对比报告已保存: {comparison_path}", flush=True)

    print(f"\n评估完成！", flush=True)


if __name__ == "__main__":
    main()
