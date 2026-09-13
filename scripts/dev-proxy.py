"""
LRC 开发代理服务器
- 提供静态文件服务（来自 ../../static 目录）
- 代理 API 请求到 sidecar（端口 3099）
- 支持 CDP WebView2 交互测试

用法: python dev-proxy.py [端口号]
"""
import http.server
import http.client
import urllib.parse
import os
import sys
import json

SIDECAR_PORT = 3111 if (
    os.environ.get('LRC_DEV_MODE') == '1' or '--dev' in sys.argv
) else 3099
STATIC_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'static')
STATIC_DIR = os.path.normpath(STATIC_DIR)

class ProxyHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self._handle_request('GET')

    def do_POST(self):
        self._handle_request('POST')

    def do_PUT(self):
        self._handle_request('PUT')

    def do_DELETE(self):
        self._handle_request('DELETE')

    def _is_api_request(self):
        """判断是否为 API 请求"""
        path = urllib.parse.urlparse(self.path).path
        return path.startswith('/v1/') or path.startswith('/health') or path.startswith('/api/')

    def _handle_request(self, method):
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path
        query = parsed.query

        if self._is_api_request():
            self._proxy_to_sidecar(method, path, query)
        else:
            self._serve_static(path)

    def _proxy_to_sidecar(self, method, path, query):
        """代理请求到 sidecar"""
        try:
            body = None
            if method in ('POST', 'PUT'):
                content_length = int(self.headers.get('Content-Length', 0))
                if content_length > 0:
                    body = self.rfile.read(content_length)

            # 构建目标 URL
            target_path = path
            if query:
                target_path += '?' + query

            # 联想探索等重型接口在有大量记忆时可超过 10s（多跳 + 道体回归校验），
            # 代理超时需同步放宽，否则 1420 链路会误报 502 timed out
            conn = http.client.HTTPConnection('127.0.0.1', SIDECAR_PORT, timeout=30)
            conn.request(method, target_path, body=body, headers={
                'Content-Type': self.headers.get('Content-Type', 'application/json'),
                'Accept': self.headers.get('Accept', 'application/json'),
            })
            response = conn.getresponse()
            resp_body = response.read()

            self.send_response(response.status)
            for key, value in response.getheaders():
                if key.lower() not in ('transfer-encoding', 'content-encoding', 'content-length'):
                    self.send_header(key, value)
            self.send_header('Content-Length', str(len(resp_body)))
            self.end_headers()
            self.wfile.write(resp_body)
            conn.close()
        except Exception as e:
            self.send_response(502)
            self.send_header('Content-Type', 'application/json')
            self.end_headers()
            self.wfile.write(json.dumps({
                'ok': False,
                'message': f'代理请求失败: {str(e)}'
            }).encode())

    def _serve_static(self, path):
        """提供静态文件服务"""
        if not path or path == '/':
            path = '/index.html'

        # 移除查询参数
        file_path = os.path.normpath(os.path.join(STATIC_DIR, path.lstrip('/')))

        # 安全检查：确保文件在 STATIC_DIR 内
        if not file_path.startswith(STATIC_DIR):
            self.send_response(403)
            self.end_headers()
            self.wfile.write(b'Forbidden')
            return

        if os.path.isdir(file_path):
            file_path = os.path.join(file_path, 'index.html')

        if os.path.exists(file_path) and os.path.isfile(file_path):
            content = open(file_path, 'rb').read()
            if os.path.basename(file_path).lower() == 'index.html' and SIDECAR_PORT != 3099:
                content = content.replace(
                    b'<meta name="lrc-sidecar-port" content="3099">',
                    f'<meta name="lrc-sidecar-port" content="{SIDECAR_PORT}">'.encode('utf-8'),
                )
            self.send_response(200)
            # 根据扩展名设置 Content-Type
            ext = os.path.splitext(file_path)[1].lower()
            content_types = {
                '.html': 'text/html; charset=utf-8',
                '.js': 'application/javascript; charset=utf-8',
                '.css': 'text/css; charset=utf-8',
                '.json': 'application/json',
                '.png': 'image/png',
                '.jpg': 'image/jpeg',
                '.jpeg': 'image/jpeg',
                '.gif': 'image/gif',
                '.svg': 'image/svg+xml',
                '.ico': 'image/x-icon',
                '.woff': 'font/woff',
                '.woff2': 'font/woff2',
            }
            self.send_header('Content-Type', content_types.get(ext, 'application/octet-stream'))
            # 开发模式禁用 HTTP 缓存：否则 WebView2 启发式缓存旧 app.js，
            # 导致"改了静态文件但桌面看到的还是旧代码"（v0.9.7 调试踩坑根因）
            self.send_header('Cache-Control', 'no-store, no-cache, must-revalidate')
            self.send_header('Pragma', 'no-cache')
            self.send_header('Expires', '0')
            self.end_headers()
            self.wfile.write(content)
        else:
            self.send_response(404)
            self.send_header('Content-Type', 'text/plain; charset=utf-8')
            self.end_headers()
            self.wfile.write(f'File not found: {path}'.encode('utf-8'))

    def log_message(self, format, *args):
        """简化日志输出"""
        sys.stderr.write(f"[LRC-Dev] {args[0]} {args[1]} {args[2]}\n")

def main():
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 1420
    server = http.server.HTTPServer(('0.0.0.0', port), ProxyHandler)
    print(f"[LRC-Dev] 代理服务器启动: http://localhost:{port}")
    print(f"[LRC-Dev] 静态文件目录: {STATIC_DIR}")
    print(f"[LRC-Dev] API 代理目标: http://127.0.0.1:{SIDECAR_PORT}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\n[LRC-Dev] 服务器已停止")
        server.server_close()

if __name__ == '__main__':
    main()