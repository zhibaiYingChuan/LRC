// Loong Recall (L-RC / 忆) 构建脚本
// =======================================
// 在编译时生成完整性校验哈希，用于运行时防篡改检测。
//
// 许可证: Apache 2.0 (公开层构建脚本)

use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    let mut hasher = Sha256::new();

    let src_dir = manifest_dir.join("src");
    if src_dir.exists() {
        hash_directory(&src_dir, &mut hasher);
    }

    let cargo_toml = manifest_dir.join("Cargo.toml");
    if cargo_toml.exists() {
        let content = fs::read_to_string(&cargo_toml).unwrap_or_default();
        hasher.update(content.as_bytes());
    }

    let source_hash = format!("{:x}", hasher.finalize());

    let guard_rs = out_dir.join("integrity_hash.rs");
    let code = format!(
        "// 自动生成，请勿手动编辑\n\
         pub const SOURCE_INTEGRITY_HASH: &str = \"{source_hash}\";\n"
    );
    fs::write(&guard_rs, code).expect("写入完整性哈希失败");

    println!("cargo:rerun-if-changed=src/");
    println!("cargo:rerun-if-changed=Cargo.toml");
}

fn hash_directory(dir: &PathBuf, hasher: &mut Sha256) {
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap().filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let path = entry.path();
        let file_name = entry.file_name();

        hasher.update(file_name.to_string_lossy().as_bytes());

        if path.is_dir() {
            hash_directory(&path, hasher);
        } else if path
            .extension()
            .is_some_and(|ext| ext == "rs" || ext == "toml")
        {
            let content = fs::read_to_string(&path).unwrap_or_default();
            hasher.update(content.as_bytes());
        }
    }
}
