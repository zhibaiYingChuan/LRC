//! ============================================================
//! 许可证: Apache 2.0
//! 本文件定义「记忆持久化字段的单一真源」，属于公开层 (Layer 1)。
//! ============================================================
//!
//! ## 为什么需要（结构性修复，不是抽象洁癖）
//!
//! 修复前，各持久化后端**各自手写字段白名单**，三份清单互不校验：
//!
//! | 后端 | 修复前的问题 |
//! |---|---|
//! | JSON（默认） | 靠 `#[derive(Serialize, Deserialize)]` 覆盖全字段 ⇒ **无丢失** |
//! | Postgres | INSERT 列出 24 列，但 `ON CONFLICT DO UPDATE SET` **只更新 10 列** ⇒ 更新既有记忆时十余字段**静默丢弃**；且有 5 个字段（`information_gain` / `resolution` / `daoti_preview_*`）**从未建列** |
//! | Qdrant | payload 只写 12 字段，且读回时把 `created_at`/`updated_at`/`last_accessed` **无条件重置为 `now()`** |
//!
//! ⇒ 新增 `Memory` 字段时，"漏一路径"**没有任何机制能发现**（这正是本项目历史上
//! 反复出现的"字段白名单漏字段 ⇒ 静默假绿"故障模式）。
//!
//! 本模块提供：① 唯一字段清单 [`MEMORY_FIELDS`]；② 由清单**派生**的 SQL 片段生成器；
//! ③ 守卫测试（**默认 feature 即可运行，不需要连接任何数据库**）。
//!
//! ## 新增 `Memory` 字段时的义务
//! 1. 在 [`MEMORY_FIELDS`] 中登记（含 Postgres 列定义片段）；
//! 2. 跑 `cargo test --features server --lib persistence::fields` —— 守卫会校验
//!    清单与 `Memory` 的**序列化键集合**一致（漏登记即红）。

/// 主键字段名（不参与 upsert 的 `SET` 子句）
pub const MEMORY_PRIMARY_KEY: &str = "id";

/// `Memory` 的全部持久化字段：`(字段名, Postgres 列定义片段)`。
///
/// ## 顺序约定
/// **顺序即 Postgres `INSERT` 的绑定顺序**（与 `Memory` 结构体声明顺序无关）。
/// 守卫测试比对的是**集合**而非顺序；顺序只对"绑定位置"有意义，
/// 因此**新增字段一律追加到末尾**（老代码的绑定位置不变，改动面最小）。
///
/// ## 为什么把"列定义"也放在这里
/// 让 `CREATE TABLE` 与 `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` 由**同一份清单**派生
/// ⇒ 老库补列与新建表**不可能漂移**（此前 DDL 里根本没有 `information_gain` 等 5 列）。
pub const MEMORY_FIELDS: &[(&str, &str)] = &[
    ("id", "TEXT PRIMARY KEY"),
    ("content", "TEXT NOT NULL"),
    ("memory_type", "TEXT NOT NULL DEFAULT 'fact'"),
    ("project", "TEXT"),
    ("tags", "JSONB DEFAULT '[]'"),
    ("importance", "SMALLINT DEFAULT 5"),
    ("version", "INTEGER DEFAULT 1"),
    ("created_at", "TIMESTAMPTZ NOT NULL DEFAULT NOW()"),
    ("updated_at", "TIMESTAMPTZ NOT NULL DEFAULT NOW()"),
    ("last_accessed", "TIMESTAMPTZ NOT NULL DEFAULT NOW()"),
    ("ttl_days", "INTEGER"),
    ("luoshu_vector", "JSONB"),
    ("bagua_index", "SMALLINT"),
    ("bagua_category", "TEXT"),
    ("topological_depth", "REAL DEFAULT 0.5"),
    ("privacy_level", "TEXT DEFAULT 'user'"),
    ("session_id", "TEXT"),
    ("user_id", "TEXT"),
    ("event_id", "TEXT"),
    ("entities", "JSONB DEFAULT '[]'"),
    ("source", "TEXT"),
    // 修正：此前 DDL 写 `DEFAULT '{{}}'`，经 `format!` 渲染为 `'{}'`（空**对象**），
    // 而该列承载的是**数组**（`Vec<String>` / `Vec<MemoryVersion>`）⇒ 统一为 `'[]'`。
    ("source_ids", "JSONB DEFAULT '[]'"),
    ("confidence", "REAL"),
    ("version_history", "JSONB DEFAULT '[]'"),
    // ═══ v0.9.11 补齐（G2 / CR-08）：以下 5 个字段此前在 Postgres 上**完全未持久化** ═══
    ("information_gain", "REAL"),
    ("resolution", "TEXT NOT NULL DEFAULT 'detailed'"),
    ("daoti_preview_gua", "TEXT"),
    ("daoti_preview_bagua", "TEXT"),
    ("daoti_preview_version", "TEXT"),
];

/// 字段名清单（顺序同 [`MEMORY_FIELDS`]）
pub fn field_names() -> Vec<&'static str> {
    MEMORY_FIELDS.iter().map(|(name, _)| *name).collect()
}

/// `CREATE TABLE IF NOT EXISTS <table> (...)` —— 由单一真源派生
pub fn create_table_sql(table: &str) -> String {
    let cols: Vec<String> = MEMORY_FIELDS
        .iter()
        .map(|(name, def)| format!("{name} {def}"))
        .collect();
    format!("CREATE TABLE IF NOT EXISTS {table} ({})", cols.join(", "))
}

/// 老库补列语句：对每个字段生成 `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`。
///
/// **为什么必须有**：`CREATE TABLE IF NOT EXISTS` 对**已存在的表不会补列**
/// ⇒ 只改 DDL 的话，老部署永远不会拿到新字段（静默降级）。逐列 `ADD COLUMN IF NOT EXISTS`
/// 是幂等的，且能安全地作用于已有数据的表。
pub fn add_missing_columns_sql(table: &str) -> Vec<String> {
    MEMORY_FIELDS
        .iter()
        .map(|(name, def)| format!("ALTER TABLE {table} ADD COLUMN IF NOT EXISTS {name} {def}"))
        .collect()
}

/// `INSERT` 的列清单（`a, b, ...`）
pub fn insert_columns_sql() -> String {
    field_names().join(", ")
}

/// `INSERT` 的占位符（`$1, $2, ...`）
pub fn insert_placeholders_sql() -> String {
    (1..=MEMORY_FIELDS.len())
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `ON CONFLICT (id) DO UPDATE SET ...` 的 **SET 部分**（**全部非主键字段**）。
///
/// **这是 CR-07 的结构性修复**：此前该子句**手写且只覆盖 10 列**，
/// 导致更新既有记忆时 `project`/`tags`/`created_at`/`privacy_level`/`session_id`/
/// `user_id`/`source`/`source_ids`/`confidence`/`version_history` 等的变更**静默丢弃**。
/// 改为由单一真源派生后，"漏列"在构造上不可能发生。
pub fn upsert_set_clause_sql() -> String {
    field_names()
        .into_iter()
        .filter(|name| *name != MEMORY_PRIMARY_KEY)
        .map(|name| format!("{name} = EXCLUDED.{name}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_types::{
        EntityKind, EventEntity, Importance, Memory, MemoryType, MemoryVersion, PrivacyLevel,
    };
    use chrono::{TimeZone, Utc};
    use std::collections::HashSet;

    /// 构造一条**所有字段均为非默认值**的记忆（用于序列化键集合比对）。
    ///
    /// 必须全部非默认：`Memory` 上多处用了 `skip_serializing_if`，
    /// 若留空则键会**被跳过**，守卫会误判"字段不存在"。
    fn fully_populated_memory() -> Memory {
        let mut m = Memory::new(
            "内容".to_string(),
            MemoryType::Experience,
            Some("proj".to_string()),
            vec!["t1".to_string()],
            Importance::new(7),
            Some(30),
        )
        .with_privacy(
            PrivacyLevel::Session,
            Some("sess".to_string()),
            Some("user".to_string()),
        )
        .with_event(Some("evt".to_string()))
        .with_entities(vec![EventEntity {
            name: "西湖".to_string(),
            kind: EntityKind::Place,
        }])
        .with_source("unit-test")
        .with_created_at(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap());
        m.updated_at = Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap();
        m.last_accessed = Utc.with_ymd_and_hms(2026, 1, 3, 0, 0, 0).unwrap();
        m.source_ids = vec!["src-1".to_string()];
        m.confidence = Some(0.8);
        m.information_gain = Some(0.4);
        m.resolution = "synthesized".to_string();
        m.luoshu_vector = Some([0.1; 9]);
        m.bagua_index = Some(3);
        m.bagua_category = Some("离".to_string());
        m.daoti_preview_gua = Some("乾".to_string());
        m.daoti_preview_bagua = Some("乾为天".to_string());
        m.daoti_preview_version = Some("pv1".to_string());
        m.topological_depth = 0.25;
        m.version = 2;
        m.version_history = vec![MemoryVersion {
            version: 1,
            content: "旧内容".to_string(),
            updated_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            reason: Some("修正".to_string()),
        }];
        m
    }

    /// ★核心守卫：单一真源必须与 `Memory` 的**实际可序列化字段**完全一致。
    ///
    /// 失败含义：① 结构体加了字段但没登记 ⇒ 键多出；② 清单登记了不存在的字段 ⇒ 键缺失。
    /// 两种都必须修 —— 这正是"字段白名单漏字段"这类静默故障的自动防线。
    #[test]
    fn persisted_field_list_matches_serialization() {
        let value = serde_json::to_value(fully_populated_memory()).expect("序列化失败");
        let obj = value.as_object().expect("Memory 必须序列化为对象");
        let serialized: HashSet<&str> = obj.keys().map(String::as_str).collect();
        let declared: HashSet<&str> = field_names().into_iter().collect();

        let missing_in_list: Vec<&&str> = serialized.difference(&declared).collect();
        let missing_in_struct: Vec<&&str> = declared.difference(&serialized).collect();
        assert!(
            missing_in_list.is_empty(),
            "Memory 有字段未登记进 MEMORY_FIELDS（新增字段必须登记）：{missing_in_list:?}"
        );
        assert!(
            missing_in_struct.is_empty(),
            "MEMORY_FIELDS 登记了 Memory 上不存在的字段（清单过期）：{missing_in_struct:?}"
        );
    }

    /// 守卫：`CREATE TABLE` 必须覆盖全部字段（此前 5 列从未建过）
    #[test]
    fn create_table_covers_all_fields() {
        let sql = create_table_sql("t_");
        for name in field_names() {
            assert!(
                sql.contains(name),
                "CREATE TABLE 缺少字段列：{name}\n实际：{sql}"
            );
        }
        assert!(sql.contains(MEMORY_PRIMARY_KEY), "必须含主键");
    }

    /// ★守卫：老库补列必须覆盖全部字段（`CREATE TABLE IF NOT EXISTS` 不会给已有表补列）
    #[test]
    fn add_missing_columns_covers_all_fields() {
        let stmts = add_missing_columns_sql("t_");
        assert_eq!(
            stmts.len(),
            MEMORY_FIELDS.len(),
            "每个字段都应有一条补列语句"
        );
        for name in field_names() {
            assert!(
                stmts
                    .iter()
                    .any(|s| s.contains(&format!("ADD COLUMN IF NOT EXISTS {name} "))),
                "缺补列语句：{name}"
            );
        }
    }

    /// ★守卫：`INSERT` 的列数 / 占位符数必须一致（绑定错位会导致写到错列）
    #[test]
    fn insert_columns_and_placeholders_are_aligned() {
        // 先把 String 落到变量上：`split` 借用其缓冲区，直接对临时值取切片会 E0716。
        let cols_sql = insert_columns_sql();
        let marks_sql = insert_placeholders_sql();
        let cols: Vec<&str> = cols_sql.split(", ").collect();
        let marks: Vec<&str> = marks_sql.split(", ").collect();
        assert_eq!(cols.len(), MEMORY_FIELDS.len(), "列数与字段清单不一致");
        assert_eq!(
            marks.len(),
            cols.len(),
            "占位符数与列数不一致（绑定会错位）"
        );
        assert_eq!(marks.first().copied(), Some("$1"));
        let expected_last = format!("${}", MEMORY_FIELDS.len());
        assert_eq!(marks.last().copied(), Some(expected_last.as_str()));
    }

    /// ★守卫（CR-07 的回归防线）：upsert 的 SET 子句必须覆盖**全部非主键字段**。
    ///
    /// 失败含义：又出现了"只更新部分列"⇒ 更新既有记忆时字段静默丢失。
    #[test]
    fn upsert_set_clause_covers_all_non_key_fields() {
        let set = upsert_set_clause_sql();
        for name in field_names() {
            if name == MEMORY_PRIMARY_KEY {
                continue;
            }
            assert!(
                set.contains(&format!("{name} = EXCLUDED.{name}")),
                "SET 子句缺少字段 {name}（更新既有记忆时该字段会静默丢失）\n实际：{set}"
            );
        }
        assert!(
            !set.contains("id = EXCLUDED.id"),
            "主键不得出现在 SET 子句中（会破坏 upsert 语义）"
        );
    }
}
