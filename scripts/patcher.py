#!/usr/bin/env python3
"""
Loong Recall (L-RC / 忆) 二进制完整性补丁工具
===============================================
后构建脚本：编译完成后自动计算 .text 段 CRC32 并嵌入二进制，
使运行时 PE 完整性校验 (verify_pe_integrity) 能够检测代码篡改。

工作流程:
  1. 解析 PE 二进制文件结构
  2. 定位 .text 段并计算其 CRC32 值
  3. 在 .rdata 段中查找魔术值 0xDEAD_BEEF（由 guard.rs 定义）
  4. 将该魔术值替换为实际 CRC32 值

用法:
  python scripts/patcher.py <二进制文件路径>

示例:
  python scripts/patcher.py target/release/code-memory-server.exe
"""

import struct
import sys
from pathlib import Path

MAGIC_VALUE = 0xDEAD_BEEF
CRC32_POLY = 0xEDB88320


def compute_crc32(data: bytes) -> int:
    """计算与 Rust CRC32 兼容的校验值"""
    crc = 0xFFFFFFFF
    table = _make_crc32_table()
    for byte in data:
        crc = table[(crc ^ byte) & 0xFF] ^ (crc >> 8)
    return crc ^ 0xFFFFFFFF


def _make_crc32_table() -> list:
    """生成 CRC32 查找表"""
    table = []
    for i in range(256):
        crc = i
        for _ in range(8):
            if crc & 1:
                crc = (crc >> 1) ^ CRC32_POLY
            else:
                crc >>= 1
        table.append(crc)
    return table


def parse_pe(binary_path: str) -> dict:
    """
    解析 PE 文件结构，返回节表信息。
    """
    with open(binary_path, "rb") as f:
        data = f.read()

    # DOS Header
    if data[:2] != b"MZ":
        raise ValueError("不是有效的 PE 文件（缺少 MZ 签名）")

    # e_lfanew 在偏移 0x3C 处（4 字节小端）
    pe_offset = struct.unpack_from("<I", data, 0x3C)[0]

    # PE 签名
    pe_sig = struct.unpack_from("<I", data, pe_offset)[0]
    if pe_sig != 0x00004550:
        raise ValueError(f"无效的 PE 签名: 0x{pe_sig:08X}")

    # COFF 文件头（PE 签名后 4 字节 + 20 字节）
    coff_offset = pe_offset + 4
    machine, num_sections, _, _, _, size_of_optional = struct.unpack_from(
        "<HHIIIH", data, coff_offset
    )

    # 可选头偏移
    opt_offset = coff_offset + 20
    # 跳过可选头，定位节表
    section_offset = opt_offset + size_of_optional

    sections = []
    for i in range(num_sections):
        sec_off = section_offset + i * 40
        name_raw = data[sec_off:sec_off + 8]
        name = name_raw.rstrip(b"\x00").decode("ascii", errors="replace")
        (
            virtual_size,
            virtual_address,
            size_of_raw_data,
            pointer_to_raw_data,
        ) = struct.unpack_from("<IIII", data, sec_off + 8)
        sections.append(
            {
                "name": name,
                "virtual_size": virtual_size,
                "virtual_address": virtual_address,
                "size_of_raw_data": size_of_raw_data,
                "pointer_to_raw_data": pointer_to_raw_data,
                "offset": sec_off,
            }
        )

    return {
        "data": data,
        "sections": sections,
        "size": len(data),
    }


def find_magic_offset(data: bytes) -> int:
    """在二进制数据中查找魔术值的位置"""
    magic_bytes = struct.pack("<I", MAGIC_VALUE)
    offset = data.find(magic_bytes)
    if offset == -1:
        raise ValueError(
            f"未找到魔术值 0x{MAGIC_VALUE:08X}，"
            f"请确认 guard.rs 中 PE_TEXT_CRC 静态变量已正确声明"
        )
    return offset


def patch_binary(binary_path: str) -> bool:
    """
    对二进制文件执行 CRC 补丁：
    1. 计算 .text 段 CRC32
    2. 将 CRC32 写入魔术值位置
    """
    print(f"[patcher] 正在处理: {binary_path}")
    print(f"[patcher] 文件大小: {Path(binary_path).stat().st_size / 1024:.1f} KB")

    # 1. 解析 PE
    pe_info = parse_pe(binary_path)
    data = bytearray(pe_info["data"])

    # 2. 找到 .text 段
    text_section = None
    for sec in pe_info["sections"]:
        if sec["name"] == ".text":
            text_section = sec
            break

    if text_section is None:
        raise ValueError("未找到 .text 段")

    print(
        f"[patcher] .text 段: 偏移 0x{text_section['pointer_to_raw_data']:X}, "
        f"大小 {text_section['size_of_raw_data']} 字节"
    )

    # 3. 计算 .text 段 CRC32
    text_start = text_section["pointer_to_raw_data"]
    text_size = text_section["size_of_raw_data"]
    text_data = data[text_start:text_start + text_size]
    crc_value = compute_crc32(text_data)
    print(f"[patcher] .text 段 CRC32: 0x{crc_value:08X}")

    # 4. 查找魔术值并替换
    patch_offset = find_magic_offset(bytes(data))
    print(f"[patcher] 魔术值位置: 偏移 0x{patch_offset:X}")

    # 写入实际 CRC32（小端）
    crc_bytes = struct.pack("<I", crc_value)
    data[patch_offset:patch_offset + 4] = crc_bytes

    # 5. 写回文件
    with open(binary_path, "wb") as f:
        f.write(data)

    print(f"[patcher] 已写入 CRC32 0x{crc_value:08X} → 偏移 0x{patch_offset:X}")
    print("[patcher] 二进制完整性补丁完成 ✓")
    return True


def main():
    if len(sys.argv) < 2:
        print("用法: python scripts/patcher.py <二进制文件路径>")
        print("示例: python scripts/patcher.py target/release/code-memory-server.exe")
        sys.exit(1)

    binary_path = sys.argv[1]
    if not Path(binary_path).exists():
        print(f"[patcher] 错误: 文件不存在: {binary_path}")
        sys.exit(1)

    try:
        patch_binary(binary_path)
    except Exception as e:
        print(f"[patcher] 错误: {e}")
        sys.exit(1)


if __name__ == "__main__":
    main()