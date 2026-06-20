# ============================================================
# Loong Recall (L-RC / 忆) — Dockerfile
# ============================================================
# 多阶段构建：编译 → 运行时镜像
# ============================================================

# ---- Stage 1: 编译阶段 ----
FROM rust:1.85-slim-bookworm AS builder

# 安装编译依赖
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# 先复制依赖清单（利用 Docker 层缓存）
COPY Cargo.toml Cargo.lock* ./
COPY build.rs ./

# 创建占位 src 以满足 cargo 预下载
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo fetch || true
RUN rm -rf src

# 复制源码和静态资源（编译时通过 include_str! 嵌入）
COPY src/ ./src/
COPY static/ ./static/

# 编译 release 版本（仅 server feature）
RUN cargo build --release --features server --bin code-memory-server \
    && strip target/release/code-memory-server

# ---- Stage 2: 运行时阶段 ----
FROM debian:bookworm-slim AS runtime

# 安装运行时依赖
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# 创建非 root 用户
RUN useradd -m -s /bin/bash loong

WORKDIR /app

# 从 builder 复制编译产物
COPY --from=builder /app/target/release/code-memory-server /app/code-memory-server

# 创建数据目录
RUN mkdir -p /app/.loong-recall /app/data && \
    chown -R loong:loong /app

# 切换到非 root 用户
USER loong

# 暴露端口
EXPOSE 3099

# 健康检查
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:3099/health || exit 1

# 环境变量配置
ENV LRC_HOST=0.0.0.0
ENV LRC_PORT=3099
ENV LRC_MODE=fast

# 启动服务
ENTRYPOINT ["/app/code-memory-server"]
CMD ["--host", "0.0.0.0", "--port", "3099", "--mode", "fast", "--src-dir", "/app/data"]