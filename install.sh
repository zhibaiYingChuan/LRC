#!/bin/bash
set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPT_DIR"

echo "========================================"
echo "   Loong Recall (L-RC) 一键安装脚本"
echo "========================================"
echo ""
echo "本脚本将依次完成：检测 Rust 环境 → 编译 → 配置 IDE MCP 连接"
echo ""

# 1. 检测 Rust 环境
if ! command -v cargo &> /dev/null; then
    echo "[错误] 未检测到 Rust 环境。"
    echo "请安装 Rust：curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    echo "国内用户建议使用镜像：https://mirrors.ustc.edu.cn/rust-static/rustup/"
    exit 1
fi

# 2. 编译项目
echo "[1/3] 正在编译 Loong Recall（首次编译约 2-5 分钟）..."
cargo build --release --features server

SERVER_PATH="$SCRIPT_DIR/target/release/code-memory-server"
SRC_DIR="$SCRIPT_DIR/src"
echo "编译完成！"

# 3. 模型下载指引（Smart Match 模式需要）
echo ""
echo "[提示] 如果使用 Smart Match 模式（语义搜索），首次启动需要下载模型。"
echo "自动使用国内镜像 hf-mirror.com，约 500MB。"
echo ""
echo "如果下载缓慢，可手动下载模型放到 models/ 目录："
echo "  1. 访问 https://hf-mirror.com/microsoft/graphcodebert-base"
echo "  2. 下载所有文件到: $SCRIPT_DIR/models/microsoft--graphcodebert-base/"
echo "  3. 重启服务即可（LRC 自动优先加载本地模型）"
echo ""
echo "如果使用代理，启动时添加 --proxy http://127.0.0.1:端口"
echo ""

# 4. 查找可用的 IDE 并配置 MCP
echo "[2/3] 正在搜索本地 IDE..."

# 辅助函数：配置 IDE 的 MCP 连接
config_ide() {
    local ide_name="$1"
    local config_file="$2"
    local config_dir
    config_dir="$(dirname "$config_file")"

    echo "   发现 $ide_name，正在配置 MCP..."

    # 确保配置目录存在
    mkdir -p "$config_dir"

    if [ -f "$config_file" ]; then
        # 检查是否已有 loong-recall 配置
        if grep -q "loong-recall" "$config_file" 2>/dev/null; then
            echo "   已存在 loong-recall 配置，跳过。"
            return
        fi
        # 文件存在但不含 loong-recall，提示手动合并
        echo ""
        echo "   [提示] $config_file 已存在其他 MCP 配置。"
        echo "   请手动将以下内容合并到该文件的 \"mcpServers\" 对象中："
        echo ""
        echo "   \"loong-recall\": {"
        echo "     \"command\": \"$SERVER_PATH\","
        echo "     \"args\": [\"--src-dir\", \"$SRC_DIR\", \"--stdio\"]"
        echo "   }"
        echo ""
    else
        # 配置文件不存在，直接创建
        cat > "$config_file" << EOF
{
  "mcpServers": {
    "loong-recall": {
      "command": "$SERVER_PATH",
      "args": ["--src-dir", "$SRC_DIR", "--stdio"]
    }
  }
}
EOF
        echo "   已创建 $config_file。"
    fi
}

# 3a. Trae
if [ -d "$HOME/Library/Application Support/Trae" ]; then
    config_ide "Trae" "$HOME/Library/Application Support/Trae/User/mcp.json"
elif [ -d "$HOME/.config/Trae" ]; then
    config_ide "Trae" "$HOME/.config/Trae/User/mcp.json"
fi

# 3b. Trae CN
if [ -d "$HOME/Library/Application Support/Trae CN" ]; then
    config_ide "Trae CN" "$HOME/Library/Application Support/Trae CN/User/mcp.json"
elif [ -d "$HOME/.config/Trae CN" ]; then
    config_ide "Trae CN" "$HOME/.config/Trae CN/User/mcp.json"
fi

# 3c. Cursor
if [ -d "$HOME/Library/Application Support/Cursor" ]; then
    config_ide "Cursor" "$HOME/Library/Application Support/Cursor/mcp.json"
elif [ -d "$HOME/.config/Cursor" ]; then
    config_ide "Cursor" "$HOME/.config/Cursor/mcp.json"
fi

# 3d. VS Code
if [ -d "$HOME/Library/Application Support/Code" ]; then
    config_ide "VS Code" "$HOME/Library/Application Support/Code/User/mcp.json"
elif [ -d "$HOME/.config/Code" ]; then
    config_ide "VS Code" "$HOME/.config/Code/User/mcp.json"
fi

echo ""
echo "[3/3] 安装完成！"
echo "========================================"
echo ""
echo "请重启你的 IDE（Trae / Cursor / VS Code），"
echo "AI 助手将自动识别 Loong Recall 工具。"
echo ""
echo "你可以手动启动服务测试："
echo "  \"$SERVER_PATH\" --src-dir \"$SRC_DIR\" --port 3099"
echo ""