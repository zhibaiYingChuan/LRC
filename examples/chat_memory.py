#!/usr/bin/env python3
"""
Loong Recall (L-RC / 忆) 使用示例

演示通过 HTTP 接口与 Loong Recall MCP 服务交互的基本流程。
运行前请先启动服务:
    ./target/release/code-memory-server --src-dir ./src --port 3099
"""

import json
import urllib.request
import time


def mcp_request(method, params=None):
    """发送 JSON-RPC 请求到 Loong Recall MCP 服务"""
    url = "http://127.0.0.1:3099/mcp"
    payload = {
        "jsonrpc": "2.0",
        "id": int(time.time() * 1000),
        "method": method,
        "params": params or {},
    }
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req) as resp:
        return json.loads(resp.read().decode("utf-8"))


def demo():
    print("=" * 60)
    print("  Loong Recall (L-RC / 忆) 使用示例")
    print("=" * 60)

    # 1. 初始化握手
    print("\n[1] 初始化 MCP 连接...")
    init_resp = mcp_request("initialize", {
        "protocolVersion": "2024-11-05",
        "capabilities": {}
    })
    print(f"    服务端: {init_resp['result']['serverInfo']['name']}")

    # 2. 查看可用工具
    print("\n[2] 获取可用工具列表...")
    tools_resp = mcp_request("tools/list")
    tool_names = [t["name"] for t in tools_resp["result"]["tools"]]
    print(f"    共 {len(tool_names)} 个工具: {', '.join(tool_names)}")

    # 3. 写入记忆
    print("\n[3] 写入测试记忆...")
    memories = [
        {
            "content": "项目使用 pnpm 作为包管理器",
            "memory_type": "preference",
            "tags": ["tooling", "pnpm"],
            "importance": 7,
        },
        {
            "content": "API 服务端口约定为 8080，数据库端口 5432",
            "memory_type": "fact",
            "tags": ["api", "config"],
            "importance": 8,
        },
        {
            "content": "决定使用 Axum 而非 Actix Web，因为 Axum 生态更活跃且与 Tower 中间件兼容更好",
            "memory_type": "decision",
            "tags": ["architecture", "web-framework"],
            "importance": 9,
        },
    ]

    for mem in memories:
        result = mcp_request("tools/call", {
            "name": "remember",
            "arguments": mem,
        })
        content = result["result"]["content"][0]["text"]
        print(f"    已写入: {content[:80]}...")

    # 4. 检索记忆
    print("\n[4] 语义检索记忆...")
    queries = [
        "包管理工具",
        "服务器端口配置",
        "为什么选择这个 Web 框架",
    ]

    for query in queries:
        result = mcp_request("tools/call", {
            "name": "recall",
            "arguments": {"query": query, "top_k": 3},
        })
        content = result["result"]["content"][0]["text"]
        # 提取第一行关键信息
        lines = content.strip().split("\n")
        summary = lines[0] if lines else content[:80]
        print(f"    查询 '{query}': {summary[:80]}...")

    # 5. 查看记忆统计
    print("\n[5] 记忆库统计...")
    result = mcp_request("tools/call", {
        "name": "memory_stats",
        "arguments": {},
    })
    content = result["result"]["content"][0]["text"]
    print(content)

    # 6. 列出所有记忆
    print("\n[6] 列出所有记忆...")
    result = mcp_request("tools/call", {
        "name": "list_memories",
        "arguments": {"sort_by": "importance", "order": "desc", "limit": 10},
    })
    content = result["result"]["content"][0]["text"]
    print(content)

    print("\n" + "=" * 60)
    print("  示例完成！")
    print("=" * 60)


if __name__ == "__main__":
    demo()