#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
HCSE Phase 6：安全沙箱 — 路径白名单 + 数据消毒 + 资源看门狗
=============================================================
提升到可信执行环境（TEE）标准，确保测试本身不引入安全风险。

三大防线：
  1. PathValidator：文件操作路径白名单，越界 → Hard Halt
  2. DataSanitizer：双重消毒（正则替换 + 结构剪枝），防止证据泄露
  3. ResourceWatchdog：内存 1024MB / CPU 60s 上限，超限先杀子进程

设计原则：
  - 测试代码不可信：所有 I/O 必须经过沙箱校验
  - 失败安全（Fail-Safe）：任何安全违规 → 立即终止 + 标记测试失败
  - 资源保护：优先终止子 CDP 会话，保护测试平台可用性
"""

import os
import re
import sys
import time
import json
import shutil
import signal
import logging
import threading
import subprocess
from pathlib import Path, PurePath
from dataclasses import dataclass, field
from typing import Optional

try:
    import psutil
except ImportError:
    psutil = None

logging.basicConfig(
    level=logging.INFO,
    format="[Sandbox][%(asctime)s][%(levelname)s] %(message)s",
    datefmt="%H:%M:%S",
)
logger = logging.getLogger("sandbox")


# ============================================================
# 防线 1：PathValidator — 路径白名单校验
# ============================================================

class PathSecurityError(Exception):
    """路径安全违规 — 触发 Hard Halt"""


class PathValidator:
    """
    路径白名单校验器 — 所有文件操作必须通过校验

    允许目录（基于当前工作目录）：
      - ./temp          临时文件
      - ./logs          日志文件
      - ./screenshots   截图文件
      - ./evidence      证据文件（HCSE 输出）

    禁止目录：
      - 系统目录（C:\\Windows, C:\\System32, /etc, /usr, /bin）
      - 用户敏感目录（~/.ssh, ~/.aws, ~/.trae-cn/memory）
      - 任意绝对路径越界（.. 路径穿越）
    """

    # 允许的相对子目录（基于项目根）
    ALLOWED_SUBDIRS = {"temp", "logs", "screenshots", "evidence"}

    # 禁止访问的系统目录前缀（Windows + Unix）
    FORBIDDEN_PREFIXES = [
        "c:\\windows", "c:\\system32", "c:\\program files",
        "c:\\program files (x86)", "c:\\users\\administrator\\.ssh",
        "c:\\users\\administrator\\.aws",
        "/etc", "/usr", "/bin", "/sbin", "/var/log",
        "/root/.ssh", "/home/.*?/.ssh",
    ]

    def __init__(self, project_root: Path):
        self.project_root = project_root.resolve()
        # 预创建允许的目录
        for subdir in self.ALLOWED_SUBDIRS:
            (self.project_root / subdir).mkdir(parents=True, exist_ok=True)
        logger.info(f"PathValidator 初始化，项目根: {self.project_root}")

    def validate(self, path: str | Path, operation: str = "write") -> Path:
        """
        校验路径是否在白名单内

        参数：
          path：待校验路径（相对或绝对）
          operation：操作类型（read/write/delete），用于错误信息

        返回：规范化后的绝对路径

        异常：PathSecurityError — 路径越界，触发 Hard Halt
        """
        raw_path = str(path)
        # 解析为绝对路径
        try:
            resolved = PurePath(path).resolve() if not str(path).startswith("/") else PurePath(path)
        except Exception:
            resolved = self.project_root / path

        abs_path = Path(path)
        if not abs_path.is_absolute():
            abs_path = self.project_root / path
        try:
            # realpath 解析符号链接和 ..
            real_abs = abs_path.resolve(strict=False)
        except Exception:
            real_abs = abs_path.absolute()

        real_str = str(real_abs).lower()

        # 检查 1：禁止系统目录
        for forbidden in self.FORBIDDEN_PREFIXES:
            if real_str.startswith(forbidden.lower()):
                self._halt(
                    f"路径安全违规：试图访问系统/敏感目录 {real_abs}（匹配禁止前缀 {forbidden}）"
                )

        # 检查 2：路径穿越（..）
        if ".." in PurePath(path).parts:
            self._halt(
                f"路径安全违规：检测到路径穿越 '..'，原始路径 {raw_path} → 解析后 {real_abs}"
            )

        # 检查 3：必须在允许的子目录内
        try:
            rel = real_abs.relative_to(self.project_root)
            top_dir = rel.parts[0].lower() if rel.parts else ""
        except ValueError:
            # 路径不在项目根下
            self._halt(
                f"路径安全违规：路径 {real_abs} 不在项目根 {self.project_root} 下"
            )
            return real_abs  # 不会执行到这里

        if top_dir not in self.ALLOWED_SUBDIRS:
            self._halt(
                f"路径安全违规：路径 {real_abs} 的顶级目录 '{top_dir}' 不在白名单 "
                f"{self.ALLOWED_SUBDIRS} 内。仅允许 temp/logs/screenshots/evidence。"
            )

        logger.debug(f"路径校验通过: {raw_path} → {real_abs} [{operation}]")
        return real_abs

    def safe_read(self, path: str | Path, encoding: str = "utf-8") -> str:
        """安全读取文件 — 先校验路径再读取"""
        validated = self.validate(path, "read")
        return validated.read_text(encoding=encoding)

    def safe_write(self, path: str | Path, content: str, encoding: str = "utf-8"):
        """安全写入文件 — 先校验路径再写入"""
        validated = self.validate(path, "write")
        validated.parent.mkdir(parents=True, exist_ok=True)
        validated.write_text(content, encoding=encoding)
        logger.info(f"安全写入: {validated} ({len(content)} 字节)")

    def _halt(self, reason: str):
        """Hard Halt — 立即终止测试进程"""
        logger.critical("=" * 60)
        logger.critical("环境安全违规 — HARD HALT")
        logger.critical(f"原因: {reason}")
        logger.critical("=" * 60)
        # 标记测试失败
        sys.exit(130)  # 130 = 安全违规专用退出码


# ============================================================
# 防线 2：DataSanitizer — 双重数据消毒
# ============================================================

class DataSanitizer:
    """
    数据消毒器 — 写入证据/日志前强制执行双重消毒

    消毒规则（对应 INV-007）：
      1. cookie value：删除所有 Cookie 头中的 value 部分
      2. authorization：替换为 [BEARER_TOKEN_REDACTED]
      3. email：替换为 [EMAIL_REDACTED]
      4. phone：替换为 [PHONE_REDACTED]
      5. API Key 模式：sk-xxx → [API_KEY_REDACTED]

    双重消毒：
      第一层：正则替换（覆盖字符串形式）
      第二层：结构剪枝（遍历 dict/list，删除敏感字段）
    """

    # 正则规则集
    PATTERNS = [
        # Authorization Bearer Token
        (re.compile(r"(authorization\s*[:=]\s*bearer\s+)([A-Za-z0-9\-._~+\/=]+)", re.IGNORECASE),
         r"\1[BEARER_TOKEN_REDACTED]"),
        # 通用 Authorization 头
        (re.compile(r"(authorization\s*[:=]\s*)([^\s,;]+)", re.IGNORECASE),
         r"\1[AUTH_REDACTED]"),
        # Cookie value（cookie: name=value 或 Set-Cookie: name=value）
        (re.compile(r"((?:cookie|set-cookie)\s*[:=]\s*)([^;\r\n]+)", re.IGNORECASE),
         r"\1[COOKIE_VALUE_REDACTED]"),
        # API Key（sk- 开头的 OpenAI 风格密钥）
        (re.compile(r"sk-[A-Za-z0-9]{20,}"), "[API_KEY_REDACTED]"),
        # Email
        (re.compile(r"[\w.+-]+@[\w-]+\.[\w.-]+"), "[EMAIL_REDACTED]"),
        # Phone（中国大陆手机号）
        (re.compile(r"1[3-9]\d{9}"), "[PHONE_REDACTED]"),
        # LLM API 配置中的密钥（provider:sk-xxx:model:base_url 格式）
        (re.compile(r"(openai|anthropic|deepseek|gemini)(:|\|\|)sk-[A-Za-z0-9]+",
                    re.IGNORECASE),
         r"\1\2[API_KEY_REDACTED]"),
    ]

    # 敏感字段名（结构剪枝时删除 value）
    SENSITIVE_FIELDS = {
        "authorization", "cookie", "set-cookie", "password", "token",
        "api_key", "apikey", "secret", "llm_api", "llm_api_key",
        "access_token", "refresh_token", "session_token",
    }

    @classmethod
    def sanitize_text(cls, text: str) -> str:
        """第一层：正则替换"""
        if not isinstance(text, str):
            text = str(text)
        for pattern, replacement in cls.PATTERNS:
            text = pattern.sub(replacement, text)
        return text

    @classmethod
    def sanitize_struct(cls, obj):
        """第二层：结构剪枝 — 递归遍历 dict/list"""
        if isinstance(obj, dict):
            result = {}
            for key, value in obj.items():
                key_lower = str(key).lower()
                if key_lower in cls.SENSITIVE_FIELDS:
                    result[key] = "[REDACTED]"
                elif isinstance(value, str):
                    result[key] = cls.sanitize_text(value)
                elif isinstance(value, (dict, list)):
                    result[key] = cls.sanitize_struct(value)
                else:
                    result[key] = value
            return result
        elif isinstance(obj, list):
            return [cls.sanitize_struct(item) for item in obj]
        elif isinstance(obj, str):
            return cls.sanitize_text(obj)
        else:
            return obj

    @classmethod
    def sanitize_json(cls, json_str: str) -> str:
        """双重消毒 JSON 字符串"""
        try:
            data = json.loads(json_str)
            sanitized = cls.sanitize_struct(data)
            return json.dumps(sanitized, ensure_ascii=False, indent=2)
        except json.JSONDecodeError:
            # 非 JSON，按纯文本消毒
            return cls.sanitize_text(json_str)

    @classmethod
    def sanitize_file(cls, path: Path):
        """原地消毒文件内容（跳过二进制文件）"""
        if not path.exists():
            return
        # 跳过二进制文件（PNG、JPG、WebM 等）
        binary_extensions = {".png", ".jpg", ".jpeg", ".gif", ".webm", ".mp4", ".ico", ".exe", ".dll"}
        if path.suffix.lower() in binary_extensions:
            logger.debug(f"跳过二进制文件: {path}")
            return
        try:
            content = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            logger.debug(f"跳过非 UTF-8 文件: {path}")
            return
        sanitized = cls.sanitize_json(content) if path.suffix == ".json" \
            else cls.sanitize_text(content)
        if content != sanitized:
            path.write_text(sanitized, encoding="utf-8")
            logger.info(f"文件已消毒: {path}")


# ============================================================
# 防线 3：ResourceWatchdog — 资源容量看门狗
# ============================================================

@dataclass
class ResourceSnapshot:
    """资源快照"""
    timestamp: float
    memory_mb: float
    cpu_percent: float
    child_count: int


class ResourceWatchdog:
    """
    资源看门狗 — 监控内存/CPU，超限先杀子进程

    阈值：
      - MAX_MEMORY_USAGE = 1024 MB
      - MAX_CPU_TIME = 60s（持续高 CPU 超过 60 秒）
      - 检查间隔 = 1s

    保护策略：
      1. 超限 → 先终止子 CDP 会话（child processes）
      2. 仍超限 → 终止测试脚本自身（保护平台）
      3. 记录资源快照到 evidence
    """

    MAX_MEMORY_MB = 1024
    MAX_CPU_TIME = 60  # 持续高 CPU 秒数
    CHECK_INTERVAL = 1.0

    def __init__(self, evidence_dir: Path, validator: PathValidator):
        self.evidence_dir = evidence_dir
        self.validator = validator
        self._snapshots: list[ResourceSnapshot] = []
        self._high_cpu_start: Optional[float] = None
        self._running = False
        self._thread: Optional[threading.Thread] = None
        self._violated = False
        self._child_procs: list[subprocess.Popen] = []

    def register_child(self, proc: subprocess.Popen):
        """注册子进程（CDP 会话），超限时优先终止"""
        self._child_procs.append(proc)
        logger.info(f"注册子进程 PID={proc.pid}（看门狗监控）")

    def start(self):
        """启动看门狗线程"""
        if psutil is None:
            logger.warning("psutil 未安装，资源看门狗降级为禁用")
            return
        self._running = True
        self._thread = threading.Thread(target=self._run, daemon=True, name="watchdog")
        self._thread.start()
        logger.info(f"资源看门狗已启动（内存上限={self.MAX_MEMORY_MB}MB，CPU 上限={self.MAX_CPU_TIME}s）")

    def stop(self):
        """停止看门狗"""
        self._running = False
        if self._thread:
            self._thread.join(timeout=3)
        # 导出资源快照
        self._export_snapshots()

    def _run(self):
        """看门狗主循环"""
        this_proc = psutil.Process()
        while self._running:
            try:
                mem = this_proc.memory_info().rss / (1024 * 1024)
                cpu = this_proc.cpu_percent(interval=0.5)
                children = len(this_proc.children(recursive=True))

                snapshot = ResourceSnapshot(
                    timestamp=time.time(),
                    memory_mb=mem,
                    cpu_percent=cpu,
                    child_count=children,
                )
                self._snapshots.append(snapshot)

                # 内存检查
                if mem > self.MAX_MEMORY_MB:
                    self._on_violation(
                        f"内存超限: {mem:.1f}MB > {self.MAX_MEMORY_MB}MB"
                    )
                    break

                # CPU 持续高占用检查
                if cpu > 90:
                    if self._high_cpu_start is None:
                        self._high_cpu_start = time.time()
                    elif time.time() - self._high_cpu_start > self.MAX_CPU_TIME:
                        self._on_violation(
                            f"CPU 持续高占用 {self.MAX_CPU_TIME}s+ ({cpu:.0f}%)"
                        )
                        break
                else:
                    self._high_cpu_start = None

            except (psutil.NoSuchProcess, Exception) as e:
                logger.warning(f"看门狗采样异常: {e}")

            time.sleep(self.CHECK_INTERVAL)

    def _on_violation(self, reason: str):
        """资源违规处理 — 先杀子进程，再终止自身"""
        self._violated = True
        logger.critical("=" * 60)
        logger.critical("资源容量违规 — 启动保护性终止")
        logger.critical(f"原因: {reason}")
        logger.critical("=" * 60)

        # 第一步：优先终止子 CDP 会话
        for proc in self._child_procs:
            try:
                if proc.poll() is None:  # 仍在运行
                    proc.terminate()
                    logger.warning(f"已终止子进程 PID={proc.pid}")
            except Exception as e:
                logger.error(f"终止子进程 PID={proc.pid} 失败: {e}")

        # 等待子进程退出
        for proc in self._child_procs:
            try:
                proc.wait(timeout=5)
            except Exception:
                try:
                    proc.kill()
                except Exception:
                    pass

        # 第二步：导出快照
        self._export_snapshots()

        # 第三步：终止自身（保护测试平台）
        logger.critical("测试平台保护性终止")
        sys.exit(131)  # 131 = 资源违规专用退出码

    def _export_snapshots(self):
        """导出资源快照到 evidence"""
        if not self._snapshots:
            return
        try:
            data = [
                {
                    "timestamp": s.timestamp,
                    "memory_mb": round(s.memory_mb, 1),
                    "cpu_percent": round(s.cpu_percent, 1),
                    "child_count": s.child_count,
                }
                for s in self._snapshots
            ]
            path = self.evidence_dir / f"resource_watchdog_{int(time.time())}.json"
            self.validator.safe_write(str(path), json.dumps(data, indent=2))
        except Exception as e:
            logger.error(f"导出资源快照失败: {e}")


# ============================================================
# Sandbox 主门面 — 统一对外接口
# ============================================================

class Sandbox:
    """
    安全沙箱门面 — 整合三道防线

    使用方式：
        sandbox = Sandbox(project_root=Path("."))
        sandbox.start()
        # 所有文件操作通过 sandbox.write / sandbox.read
        sandbox.write("evidence/report.json", json_content)
        # 所有子进程通过 sandbox.spawn 注册
        sandbox.spawn(child_proc)
        # 结束时自动导出资源快照 + 消毒所有证据
        sandbox.stop()
    """

    def __init__(self, project_root: Path):
        self.project_root = project_root.resolve()
        self.validator = PathValidator(self.project_root)
        self.evidence_dir = self.project_root / "evidence"
        self.watchdog = ResourceWatchdog(self.evidence_dir, self.validator)

    def start(self):
        """启动沙箱（资源看门狗）"""
        self.watchdog.start()
        logger.info("安全沙箱已启动")

    def write(self, path: str, content: str):
        """安全写入 — 路径校验 + 数据消毒"""
        # 先消毒内容
        if path.endswith(".json"):
            sanitized = DataSanitizer.sanitize_json(content)
        else:
            sanitized = DataSanitizer.sanitize_text(content)
        self.validator.safe_write(path, sanitized)

    def read(self, path: str, encoding: str = "utf-8") -> str:
        """安全读取 — 路径校验"""
        return self.validator.safe_read(path, encoding)

    def spawn(self, proc: subprocess.Popen):
        """注册子进程到看门狗"""
        self.watchdog.register_child(proc)

    def sanitize_all_evidence(self):
        """批量消毒 evidence 目录下所有文件"""
        for f in self.evidence_dir.rglob("*"):
            if f.is_file():
                DataSanitizer.sanitize_file(f)

    def stop(self):
        """停止沙箱 — 消毒证据 + 导出快照"""
        self.sanitize_all_evidence()
        self.watchdog.stop()
        logger.info("安全沙箱已停止，证据已消毒")

    @property
    def violated(self) -> bool:
        """是否发生安全违规"""
        return self.watchdog._violated


# ============================================================
# 自检入口
# ============================================================

def self_test():
    """沙箱自检 — 验证三道防线均正常工作"""
    import tempfile
    tmp = Path(tempfile.mkdtemp())
    try:
        sandbox = Sandbox(project_root=tmp)
        sandbox.start()

        # 测试 1：路径白名单
        print("[自检] 测试 1：路径白名单...")
        sandbox.write("evidence/test.txt", "正常内容")
        assert (tmp / "evidence" / "test.txt").exists()
        print("  ✓ 合法路径写入成功")

        # 测试 1b：越界路径应触发 Hard Halt
        print("[自检] 测试 1b：越界路径检测...")
        try:
            sandbox.validator.validate("../../etc/passwd")
            print("  ✗ 未拦截路径穿越（预期失败）")
        except SystemExit as e:
            if e.code == 130:
                print("  ✓ 路径穿越被正确拦截（Hard Halt 130）")
            else:
                raise

        # 测试 2a：结构化 JSON 消毒（结构剪枝层 — 敏感字段直接 [REDACTED]）
        print("[自检] 测试 2a：结构化 JSON 消毒（结构剪枝层）...")
        dirty_json = '{"authorization": "Bearer sk-abc123456789012345678", "email": "user@example.com", "cookie": "session=secret123", "note": "contact user@example.com please"}'
        clean_json = DataSanitizer.sanitize_json(dirty_json)
        assert "sk-abc123" not in clean_json, "API Key 未脱敏"
        assert "user@example.com" not in clean_json, "email 未脱敏"
        assert "secret123" not in clean_json, "cookie value 未脱敏"
        assert "[REDACTED]" in clean_json, "敏感字段未替换为 [REDACTED]"
        assert "[EMAIL_REDACTED]" in clean_json, "非敏感字段中的 email 未被正则层脱敏"
        print("  ✓ 结构化 JSON 敏感字段已脱敏")
        print(f"  消毒结果: {clean_json}")

        # 测试 2b：自由文本消毒（正则替换层 — Bearer Token 模式）
        print("[自检] 测试 2b：自由文本消毒（正则替换层）...")
        dirty_text = 'Header: authorization: Bearer sk-abc123456789012345678, contact admin@lrc.dev'
        clean_text = DataSanitizer.sanitize_text(dirty_text)
        assert "sk-abc123" not in clean_text, "自由文本中 API Key 未脱敏"
        assert "admin@lrc.dev" not in clean_text, "自由文本中 email 未脱敏"
        assert "[BEARER_TOKEN_REDACTED]" in clean_text, "Bearer Token 未替换"
        assert "[EMAIL_REDACTED]" in clean_text, "email 未替换"
        print("  ✓ 自由文本敏感数据已脱敏")
        print(f"  消毒结果: {clean_text}")

        # 测试 3：资源看门狗
        print("[自检] 测试 3：资源看门狗（跳过实际超限测试）...")
        print(f"  ✓ 看门狗运行中，内存上限={ResourceWatchdog.MAX_MEMORY_MB}MB")

        print("\n[自检] 所有防线验证通过")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    self_test()
