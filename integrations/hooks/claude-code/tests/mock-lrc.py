#!/usr/bin/env python3
"""Hook 端到端测试用的最小 LRC mock（仅测试用，不随产品交付）。

用法：
    MOCK_MODE=distill  python3 mock-lrc.py 3199   # /v1/context/distill 返回 items
    MOCK_MODE=fallback python3 mock-lrc.py 3199   # /v1/context/distill 返回 404

收到的请求会追加写入 $MOCK_LOG（默认 /tmp/lrc_mock_requests.jsonl），
测试脚本据此断言"到底调了哪个接口、带了什么参数"。
"""
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

MODE = os.environ.get("MOCK_MODE", "distill")
LOG = os.environ.get("MOCK_LOG", "/tmp/lrc_mock_requests.jsonl")


def record(entry):
    with open(LOG, "a", encoding="utf-8") as f:
        f.write(json.dumps(entry, ensure_ascii=False) + "\n")


class Handler(BaseHTTPRequestHandler):
    def _send(self, code, payload):
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        record({"method": "GET", "path": self.path})
        if self.path.startswith("/v1/memories/recent"):
            self._send(200, {
                "memories": [
                    {"id": "m1", "content_preview": "存储锁 Mutex → RwLock",
                     "memory_type": "decision", "project": "demo", "importance": 8},
                ],
                "total": 1, "returned": 1,
            })
        elif self.path.startswith("/v1/context/index"):
            self._send(200, {"index_line": "LRC 归档区 1 条（项目：demo）",
                             "count": 1, "total": 1, "items": []})
        else:
            self._send(404, {"error": "not_found"})

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length).decode("utf-8") if length else ""
        try:
            payload = json.loads(raw) if raw else {}
        except json.JSONDecodeError:
            payload = {"_raw": raw}
        record({"method": "POST", "path": self.path, "body": payload})

        if self.path == "/v1/context/distill":
            if MODE == "distill":
                self._send(200, {
                    "items": [{
                        "content": "[决策] 用 pnpm 替代 npm",
                        "memory_type": "decision",
                        "tags": ["decision", "distilled"],
                        "importance": 7,
                        "source_turns": [0, 0],
                    }],
                    "count": 1,
                    "index_line": "前序对话已提炼 1 条至 LRC（项目：demo）",
                })
            else:
                self._send(404, {"error": "not_implemented"})
        elif self.path == "/v1/context/migrate":
            self._send(200, {"written": 1, "merged": 0, "failed": 0,
                             "remaining": 0, "truncated": False})
        elif self.path == "/v1/memories/remember":
            self._send(200, {"success": True, "memory_id": "mock-id"})
        else:
            self._send(404, {"error": "not_found"})

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 3199
    HTTPServer(("127.0.0.1", port), Handler).serve_forever()
