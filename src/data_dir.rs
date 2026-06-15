// ============================================================
// 许可证: Apache 2.0
// 本文件实现统一数据目录管理，属于公开层 (Layer 1)。
// ============================================================
//
// 统一数据目录管理模块 — 管理 LRC 记忆数据目录的标准化路径
//
// 数据目录结构（V2）：
//   ~/.loong-recall/
//   ├── projects/
//   │   ├── {sha256_fingerprint}/    # 项目A
//   │   │   └── data/
//   │   │       ├── memories.json
//   │   │       ├── chunks.json
//   │   │       └── archive.json
//   │   └── {sha256_fingerprint}/    # 项目B
//   │       └── data/
//   ├── global/                      # --global 模式
//   │   └── data/
//   ├── config.json                  # 全局配置
//   ├── exports/                     # 导出文件存放
//   └── .lrc.lock                    # 全局服务锁（V2 移至根目录）
//
// 核心能力:
//   1. 获取 LRC 根目录 (~/.loong-recall/)
//   2. 根据项目指纹计算数据目录
//   3. 支持 --global 模式、--data-dir 覆盖、--db-path 兼容
//   4. 自动创建必要的子目录

use crate::project_id;
use std::path::{Path, PathBuf};

/// LRC 数据目录管理器
///
/// 负责确定和管理 LRC 统一数据目录结构。
/// 支持多种数据目录策略：项目指纹模式、全局模式、自定义路径。
#[derive(Debug, Clone)]
pub struct DataDir {
    /// 数据根目录（如 ~/.loong-recall/）
    root: PathBuf,
    /// 实际存储记忆数据的目标目录
    data_path: PathBuf,
    /// 模式标识
    mode: DataDirMode,
}

/// 数据目录模式
#[derive(Debug, Clone, PartialEq)]
pub enum DataDirMode {
    /// 项目指纹模式：数据存储在 ~/.loong-recall/projects/{fp}/data/
    Project { fingerprint: String },
    /// 全局模式：数据存储在 ~/.loong-recall/global/data/
    Global,
    /// 自定义路径（--db-path 或 --data-dir 显式指定）
    Custom,
    /// 旧版兼容模式（src_dir/.loong-recall/data/）
    Legacy { src_dir: PathBuf },
}

// ==================== 公共 API ====================

impl DataDir {
    /// 获取 LRC 根目录（~/.loong-recall/）
    ///
    /// 跨平台兼容：
    ///   - Windows: %USERPROFILE%/.loong-recall/
    ///   - Linux:   ~/.loong-recall/
    ///   - macOS:   ~/.loong-recall/
    pub fn root_dir() -> PathBuf {
        dirs_next::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".loong-recall")
    }

    /// 根据项目源码目录创建数据目录管理器（V2 项目指纹模式）
    ///
    /// 数据路径：~/.loong-recall/projects/{fingerprint}/data/
    pub fn for_project(src_dir: &Path) -> Self {
        let root = Self::root_dir();
        let (fingerprint, _canonical_path) = project_id::project_fingerprint_with_path(src_dir);
        let data_path = root.join("projects").join(&fingerprint).join("data");

        Self {
            root,
            data_path,
            mode: DataDirMode::Project { fingerprint },
        }
    }

    /// 创建全局模式数据目录管理器
    ///
    /// 数据路径：~/.loong-recall/global/data/
    pub fn for_global() -> Self {
        let root = Self::root_dir();
        let data_path = root.join("global").join("data");

        Self {
            root,
            data_path,
            mode: DataDirMode::Global,
        }
    }

    /// 创建自定义数据目录管理器（--data-dir 或 --db-path）
    ///
    /// 数据路径由用户显式指定，不经过根目录。
    pub fn for_custom<P: AsRef<Path>>(custom_path: P) -> Self {
        let data_path = custom_path.as_ref().to_path_buf();
        let root = Self::root_dir(); // 根目录仍用于配置和导出

        Self {
            root,
            data_path,
            mode: DataDirMode::Custom,
        }
    }

    /// 创建旧版兼容模式（src_dir/.loong-recall/data/）
    ///
    /// 用于向后兼容，也用于未迁移的旧项目。
    pub fn for_legacy(src_dir: &Path) -> Self {
        let data_path = src_dir.join(".loong-recall").join("data");
        let root = Self::root_dir();

        Self {
            root,
            data_path,
            mode: DataDirMode::Legacy {
                src_dir: src_dir.to_path_buf(),
            },
        }
    }

    /// 获取实际数据存储路径
    pub fn data_path(&self) -> &Path {
        &self.data_path
    }

    /// 获取 LRC 根目录
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 获取数据目录模式
    pub fn mode(&self) -> &DataDirMode {
        &self.mode
    }

    /// 获取项目指纹（仅在 Project 模式下有效）
    pub fn fingerprint(&self) -> Option<&str> {
        match &self.mode {
            DataDirMode::Project { fingerprint } => Some(fingerprint.as_str()),
            _ => None,
        }
    }

    /// 确保数据目录存在（递归创建所有父目录）
    ///
    /// 返回创建后的数据路径。
    pub fn ensure(&self) -> std::io::Result<&Path> {
        std::fs::create_dir_all(&self.data_path)?;
        Ok(&self.data_path)
    }

    /// 获取导出目录路径
    ///
    /// 导出文件存放在 ~/.loong-recall/exports/
    pub fn exports_dir(&self) -> PathBuf {
        self.root.join("exports")
    }

    /// 确保导出目录存在
    pub fn ensure_exports_dir(&self) -> std::io::Result<PathBuf> {
        let dir = self.exports_dir();
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// 获取全局锁文件路径（V2 移至根目录）
    ///
    /// 锁文件路径：~/.loong-recall/.lrc.lock
    pub fn global_lock_path(&self) -> PathBuf {
        self.root.join(".lrc.lock")
    }

    /// 获取项目级锁文件路径（向后兼容旧版）
    ///
    /// 锁文件路径：{data_path}/.lrc.lock
    pub fn legacy_lock_path(&self) -> PathBuf {
        self.data_path.join(".lrc.lock")
    }

    /// 获取旧版数据目录路径（src_dir/.loong-recall/data/）
    ///
    /// 用于迁移检测：检查旧版数据是否存在。
    pub fn legacy_data_path(src_dir: &Path) -> PathBuf {
        src_dir.join(".loong-recall").join("data")
    }

    /// 检查旧版数据目录是否存在
    pub fn has_legacy_data(src_dir: &Path) -> bool {
        Self::legacy_data_path(src_dir).exists()
    }

    /// 获取迁移标记文件路径
    ///
    /// 迁移完成后在旧版目录创建此文件，防止重复迁移。
    pub fn migration_marker_path(src_dir: &Path) -> PathBuf {
        src_dir.join(".loong-recall").join(".migrated_to_v2")
    }

    /// 检查是否已迁移过
    pub fn is_migrated(src_dir: &Path) -> bool {
        Self::migration_marker_path(src_dir).exists()
    }

    /// 获取项目目录路径（~/.loong-recall/projects/{fingerprint}/）
    pub fn project_dir(&self) -> Option<PathBuf> {
        match &self.mode {
            DataDirMode::Project { fingerprint } => {
                Some(self.root.join("projects").join(fingerprint))
            }
            _ => None,
        }
    }
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试: root_dir 返回以 .loong-recall 结尾的路径
    #[test]
    fn test_root_dir_ends_with_loong_recall() {
        let root = DataDir::root_dir();
        let path_str = root.to_string_lossy();
        assert!(
            path_str.contains(".loong-recall"),
            "根目录应包含 .loong-recall，实际: {path_str}"
        );
    }

    /// 测试: for_project 使用正确的子目录结构
    #[test]
    fn test_for_project_structure() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let dd = DataDir::for_project(tmp.path());

        let data_path = dd.data_path().to_string_lossy();
        assert!(
            data_path.contains("projects"),
            "数据路径应包含 'projects' 子目录，实际: {data_path}"
        );
        assert!(
            data_path.ends_with("data"),
            "数据路径应以 'data' 结尾，实际: {data_path}"
        );

        // 验证指纹不为空
        let fp = dd.fingerprint().expect("Project 模式应有指纹");
        assert_eq!(fp.len(), 16, "指纹长度应为 16");
        assert!(data_path.contains(fp), "数据路径应包含指纹");
    }

    /// 测试: for_global 使用正确的子目录结构
    #[test]
    fn test_for_global_structure() {
        let dd = DataDir::for_global();
        let data_path = dd.data_path().to_string_lossy();
        assert!(
            data_path.contains("global"),
            "全局模式数据路径应包含 'global'，实际: {data_path}"
        );
        assert!(
            data_path.ends_with("data"),
            "数据路径应以 'data' 结尾，实际: {data_path}"
        );
        assert_eq!(*dd.mode(), DataDirMode::Global);
    }

    /// 测试: for_custom 使用用户指定的路径
    #[test]
    fn test_for_custom_path() {
        let custom = if cfg!(target_os = "windows") {
            "D:\\custom-lrc-data"
        } else {
            "/tmp/custom-lrc-data"
        };
        let dd = DataDir::for_custom(custom);
        assert_eq!(
            dd.data_path().to_string_lossy(),
            custom,
            "自定义路径应完全匹配"
        );
        assert_eq!(*dd.mode(), DataDirMode::Custom);
    }

    /// 测试: for_legacy 使用旧版目录结构
    #[test]
    fn test_for_legacy_structure() {
        let src = Path::new("/tmp/my-project");
        let dd = DataDir::for_legacy(src);
        let data_path = dd.data_path().to_string_lossy();
        // 在 Windows 上路径分隔符可能不同，但都应包含 .loong-recall
        assert!(
            data_path.contains(".loong-recall"),
            "旧版数据路径应包含 .loong-recall"
        );
        assert!(
            data_path.contains("my-project"),
            "旧版数据路径应包含源码目录"
        );
    }

    /// 测试: ensure 创建数据目录
    #[test]
    fn test_ensure_creates_directory() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let dd = DataDir::for_project(tmp.path());

        // 确保数据目录存在
        assert!(!dd.data_path().exists(), "测试前目录不应存在");
        dd.ensure().expect("ensure 应成功创建目录");
        assert!(dd.data_path().exists(), "ensure 后目录应存在");
        assert!(dd.data_path().is_dir(), "ensure 创建的应是目录");
    }

    /// 测试: exports_dir 路径正确
    #[test]
    fn test_exports_dir() {
        let dd = DataDir::for_global();
        let exports = dd.exports_dir();
        let path_str = exports.to_string_lossy();
        assert!(
            path_str.contains(".loong-recall"),
            "导出目录应在 .loong-recall 下"
        );
        assert!(path_str.contains("exports"), "导出目录应包含 'exports'");
    }

    /// 测试: ensure_exports_dir 创建目录
    #[test]
    fn test_ensure_exports_dir() {
        let dd = DataDir::for_global();
        let exports = dd.ensure_exports_dir().expect("创建导出目录失败");
        assert!(exports.exists(), "导出目录应被创建");
        assert!(exports.is_dir(), "导出目录应是目录");
    }

    /// 测试: global_lock_path 在根目录下
    #[test]
    fn test_global_lock_path() {
        let dd = DataDir::for_global();
        let lock = dd.global_lock_path();
        let path_str = lock.to_string_lossy();
        assert!(path_str.contains(".lrc.lock"), "锁文件应以 .lrc.lock 结尾");
    }

    /// 测试: legacy_lock_path 在数据目录下
    #[test]
    fn test_legacy_lock_path() {
        let dd = DataDir::for_global();
        let lock = dd.legacy_lock_path();
        let path_str = lock.to_string_lossy();
        assert!(path_str.contains(".lrc.lock"), "锁文件应以 .lrc.lock 结尾");
    }

    /// 测试: legacy_data_path 构造正确
    #[test]
    fn test_legacy_data_path() {
        let src = Path::new("/home/user/project");
        let legacy = DataDir::legacy_data_path(src);
        let path_str = legacy.to_string_lossy();
        assert!(
            path_str.contains(".loong-recall"),
            "旧版路径应包含 .loong-recall"
        );
        assert!(path_str.contains("data"), "旧版路径应以 data 结尾");
    }

    /// 测试: has_legacy_data 检测旧版数据
    #[test]
    fn test_has_legacy_data() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let src = tmp.path().join("project");

        // 创建旧版数据目录结构
        let legacy_dir = DataDir::legacy_data_path(&src);
        std::fs::create_dir_all(&legacy_dir).expect("创建旧版目录失败");

        assert!(DataDir::has_legacy_data(&src), "应检测到旧版数据目录");
    }

    /// 测试: has_legacy_data 对不存在的目录返回 false
    #[test]
    fn test_no_legacy_data() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let src = tmp.path().join("empty_project");
        assert!(!DataDir::has_legacy_data(&src), "空目录不应有旧版数据");
    }

    /// 测试: migration_marker_path 构造正确
    #[test]
    fn test_migration_marker_path() {
        let src = Path::new("/home/user/project");
        let marker = DataDir::migration_marker_path(src);
        let path_str = marker.to_string_lossy();
        assert!(
            path_str.contains(".migrated_to_v2"),
            "迁移标记应包含 .migrated_to_v2"
        );
    }

    /// 测试: is_migrated 检测迁移标记
    #[test]
    fn test_is_migrated() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let src = tmp.path().join("migrated_project");

        // 创建迁移标记
        let marker = DataDir::migration_marker_path(&src);
        std::fs::create_dir_all(marker.parent().unwrap()).expect("创建父目录失败");
        std::fs::write(&marker, "v2").expect("写入标记文件失败");

        assert!(DataDir::is_migrated(&src), "应检测到迁移标记");
    }

    /// 测试: project_dir 仅在 Project 模式下返回 Some
    #[test]
    fn test_project_dir() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let dd = DataDir::for_project(tmp.path());
        assert!(dd.project_dir().is_some(), "Project 模式应有 project_dir");

        let dd_global = DataDir::for_global();
        assert!(
            dd_global.project_dir().is_none(),
            "Global 模式不应有 project_dir"
        );

        let dd_custom = DataDir::for_custom("/tmp/custom");
        assert!(
            dd_custom.project_dir().is_none(),
            "Custom 模式不应有 project_dir"
        );
    }

    /// 测试: 同一项目路径两次调用 for_project 返回相同路径
    #[test]
    fn test_same_project_idempotent() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let dd1 = DataDir::for_project(tmp.path());
        let dd2 = DataDir::for_project(tmp.path());
        assert_eq!(
            dd1.data_path(),
            dd2.data_path(),
            "同一项目应返回相同数据路径"
        );
        assert_eq!(
            dd1.fingerprint(),
            dd2.fingerprint(),
            "同一项目应返回相同指纹"
        );
    }
}
