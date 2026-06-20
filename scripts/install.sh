#!/usr/bin/env bash
# Loong Recall (LRC) v0.5.0 Linux/macOS 一键安装脚本
# 用法: curl -fsSL https://raw.githubusercontent.com/zhibaiYingChuan/LRC/main/scripts/install.sh | bash

set -euo pipefail

INSTALL_PATH="${HOME}/.lrc"
SKIP_BUILD=false
VERSION="latest"

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${CYAN}========================================${NC}"
echo -e "${CYAN}  Loong Recall (LRC) v0.5.0 安装程序${NC}"
echo -e "${CYAN}========================================${NC}"
echo ""

# 检查 Rust 工具链
echo -e "${YELLOW}[1/5] 检查 Rust 工具链...${NC}"
if command -v rustc &> /dev/null; then
    RUST_VERSION=$(rustc --version)
    echo -e "  ${GREEN}Rust 已安装: ${RUST_VERSION}${NC}"
else
    echo -e "  ${RED}Rust 未安装，请先安装 Rust: https://rustup.rs${NC}"
    echo -e "  ${YELLOW}运行: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh${NC}"
    exit 1
fi

CARGO_VERSION=$(cargo --version)
echo -e "  ${GREEN}Cargo 已安装: ${CARGO_VERSION}${NC}"

# 检查 git
if ! command -v git &> /dev/null; then
    echo -e "  ${RED}Git 未安装，请先安装 Git${NC}"
    exit 1
fi

# 创建安装目录
echo -e "${YELLOW}[2/5] 创建安装目录...${NC}"
mkdir -p "${INSTALL_PATH}"
echo -e "  ${GREEN}安装目录: ${INSTALL_PATH}${NC}"

# 克隆仓库
echo -e "${YELLOW}[3/5] 下载 LRC 源码...${NC}"
REPO_PATH="${INSTALL_PATH}/repo"
if [ -d "${REPO_PATH}" ]; then
    echo -e "  ${YELLOW}仓库已存在，正在更新...${NC}"
    cd "${REPO_PATH}" && git pull origin main
else
    git clone https://github.com/zhibaiYingChuan/LRC.git "${REPO_PATH}"
fi
echo -e "  ${GREEN}源码已下载到: ${REPO_PATH}${NC}"

# 编译
if [ "${SKIP_BUILD}" = false ]; then
    echo -e "${YELLOW}[4/5] 编译 LRC（可能需要几分钟）...${NC}"
    cd "${REPO_PATH}"
    cargo build --release --features server
    echo -e "  ${GREEN}编译完成${NC}"
fi

# 配置环境
echo -e "${YELLOW}[5/5] 配置环境...${NC}"
BIN_PATH="${REPO_PATH}/target/release"
EXE_PATH="${BIN_PATH}/code-memory"

if [ ! -f "${EXE_PATH}" ]; then
    echo -e "  ${RED}编译产物未找到，请检查编译是否成功${NC}"
    exit 1
fi

# 创建符号链接
LRC_LINK="${INSTALL_PATH}/lrc"
ln -sf "${EXE_PATH}" "${LRC_LINK}"
chmod +x "${LRC_LINK}"

# 添加到 PATH
SHELL_RC=""
if [ -f "${HOME}/.bashrc" ]; then
    SHELL_RC="${HOME}/.bashrc"
elif [ -f "${HOME}/.zshrc" ]; then
    SHELL_RC="${HOME}/.zshrc"
elif [ -f "${HOME}/.bash_profile" ]; then
    SHELL_RC="${HOME}/.bash_profile"
fi

if [ -n "${SHELL_RC}" ]; then
    if ! grep -q "${INSTALL_PATH}" "${SHELL_RC}"; then
        echo "export PATH=\"${INSTALL_PATH}:\$PATH\"" >> "${SHELL_RC}"
        echo -e "  ${GREEN}已添加到 PATH: ${SHELL_RC}${NC}"
    fi
fi

echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}  安装完成！${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo -e "  ${CYAN}使用方式:${NC}"
echo -e "    lrc --http                   启动 HTTP 服务"
echo -e "    lrc --src-dir .             启动 MCP 模式"
echo -e "    lrc --version               查看版本"
echo ""
echo -e "  ${CYAN}仪表盘: http://localhost:3099/dashboard${NC}"
echo ""
echo -e "  ${YELLOW}PATH 已更新，请运行 source ${SHELL_RC} 或重新打开终端后使用。${NC}"
echo ""