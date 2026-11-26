//! Trae 全局自动记忆钩子安装（v0.11.2）
//!
//! # 为什么需要本模块
//!
//! 桌面端此前只写「规则文件」：规则只能**建议** AI 去调用记忆工具，
//! 而真正让「每轮结论自动落库」发生的是宿主钩子。Trae 没有 PreCompact，
//! 正确接法是每轮结束的 `Stop` 钩子（WAL：先落库，压缩可以有损）。
//!
//! 本模块把三个 PowerShell 钩子写入 Trae 的全局钩子目录，并**幂等合并**
//! `hooks.json`：
//!   - 保留用户已有的其他钩子（probe / gate 等）；
//!   - 已存在 LRC 条目时更新为当前绝对路径（修复手工配置的相对路径）；
//!   - 写回前备份原配置（只备份一次，保留用户原始版本）；
//!   - 原子写（临时文件 + rename）并进程内串行化；
//!   - 任何失败只返回错误，由调用方告警，不阻断桌面端启动。

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SESSION_START: &str = include_str!("../../../integrations/hooks/trae/session-start.ps1");
const USER_PROMPT_SUBMIT: &str =
    include_str!("../../../integrations/hooks/trae/user-prompt-submit.ps1");
const STOP: &str = include_str!("../../../integrations/hooks/trae/stop.ps1");

/// 进程内串行锁：`start_sidecar` 与 `switch_project` 可能重叠触发。
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

/// 安装结果（供日志与前端展示）。
#[derive(Debug, Clone)]
pub struct HookInstallReport {
    /// 本次是否改动了配置（false = 已是最新，幂等命中）。
    pub changed: bool,
    /// hooks.json 路径。
    pub hooks_path: PathBuf,
    /// 已写入的脚本路径。
    pub scripts: Vec<PathBuf>,
    /// 首次改动前的配置备份（已存在则不覆盖）。
    pub backup: Option<PathBuf>,
}

/// 安装/修复 Trae 全局自动记忆钩子。
///
/// `tool_id` 必须是 `"trae"` 或 `"trae-cn"`：两者全局目录不同
/// （`~/.trae` vs `~/.trae-cn`），混用会让钩子落在无人读取的目录。
pub fn install_trae_hooks(home: &Path, tool_id: &str) -> Result<HookInstallReport, String> {
    if !cfg!(windows) {
        return Err("Trae 钩子安装暂仅支持 Windows（脚本为 PowerShell）".to_string());
    }
    let dir_name = match tool_id {
        "trae" => ".trae",
        "trae-cn" => ".trae-cn",
        other => return Err(format!("不支持的 Trae 标识: {other}")),
    };

    let _guard = INSTALL_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let trae_dir = home.join(dir_name);
    let hooks_dir = trae_dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir)
        .map_err(|e| format!("创建钩子目录失败: {} ({e})", hooks_dir.display()))?;

    let scripts = [
        ("lrc-session-start.ps1", SESSION_START),
        ("lrc-user-prompt-submit.ps1", USER_PROMPT_SUBMIT),
        ("lrc-stop.ps1", STOP),
    ];
    let mut script_paths = Vec::with_capacity(scripts.len());
    for (name, content) in scripts {
        let path = hooks_dir.join(name);
        write_ps1(&path, content)?;
        script_paths.push(path);
    }

    let hooks_path = trae_dir.join("hooks.json");
    let (mut cfg, existed) = read_config(&hooks_path)?;
    if !cfg.is_object() {
        return Err(format!(
            "{} 顶层不是 JSON 对象，拒绝覆盖（请手动检查）",
            hooks_path.display()
        ));
    }
    let before = cfg.clone();

    let hooks_dir_str = hooks_dir.to_string_lossy().replace('\\', "/");
    {
        let obj = cfg.as_object_mut().expect("上面已保证是对象");
        // 仅新建配置时补 Trae 的全局字段（本机 Trae 生成的 hooks.json 含此字段）；
        // 已有配置绝不注入未知顶层键，避免破坏用户原有钩子。
        if !existed {
            obj.insert("enabled_folders".to_string(), json!([hooks_dir_str]));
            obj.insert("version".to_string(), json!(1));
        }

        let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
        if !hooks.is_object() {
            return Err(format!(
                "{} 的 hooks 字段不是对象，拒绝覆盖（请手动检查）",
                hooks_path.display()
            ));
        }
        let hooks_obj = hooks.as_object_mut().expect("上面已保证是对象");
        ensure_entry(
            hooks_obj,
            "SessionStart",
            "lrc-session-start.ps1",
            command_for(&script_paths[0]),
            15,
            None,
        )?;
        ensure_entry(
            hooks_obj,
            "UserPromptSubmit",
            "lrc-user-prompt-submit.ps1",
            command_for(&script_paths[1]),
            10,
            None,
        )?;
        ensure_entry(
            hooks_obj,
            "Stop",
            "lrc-stop.ps1",
            command_for(&script_paths[2]),
            8,
            Some(3),
        )?;
    }

    let changed = cfg != before;
    let backup = if existed && changed {
        let backup_path = hooks_path.with_extension("json.lrc-backup");
        // 只备份一次：保留用户**原始**（未含 LRC）的配置，避免被后续升级覆盖。
        if !backup_path.exists() {
            std::fs::copy(&hooks_path, &backup_path)
                .map_err(|e| format!("备份 hooks.json 失败: {} ({e})", backup_path.display()))?;
        }
        Some(backup_path)
    } else {
        None
    };

    if changed || !existed {
        let text = serde_json::to_string_pretty(&cfg)
            .map_err(|e| format!("序列化 hooks.json 失败: {e}"))?;
        write_atomic(&hooks_path, text.as_bytes())?;
        // 回读校验：写坏配置比不写更糟。
        let _: Value = serde_json::from_slice(
            &std::fs::read(&hooks_path).map_err(|e| format!("回读 hooks.json 失败: {e}"))?,
        )
        .map_err(|e| format!("hooks.json 写入后无法解析: {e}"))?;
    }

    Ok(HookInstallReport {
        changed: changed || !existed,
        hooks_path,
        scripts: script_paths,
        backup,
    })
}

fn read_config(path: &Path) -> Result<(Value, bool), String> {
    if !path.exists() {
        return Ok((json!({}), false));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| format!("读取 hooks.json 失败: {} ({e})", path.display()))?;
    // PowerShell 5.1 / 记事本会写 UTF-8 BOM；UTF-16 则明确报错（不静默覆盖）。
    let body = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        rest
    } else if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(format!(
            "{} 是 UTF-16 编码，暂不支持；请另存为 UTF-8 后重试",
            path.display()
        ));
    } else {
        &bytes
    };
    let value: Value = serde_json::from_slice(body)
        .map_err(|e| format!("hooks.json 不是合法 JSON: {} ({e})", path.display()))?;
    Ok((value, true))
}

/// 原子写：同目录临时文件 + rename，避免 Trae 读到半个 JSON。
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("非法路径: {}", path.display()))?;
    let tmp = path.with_file_name(format!("{file_name}.lrc-tmp"));
    std::fs::write(&tmp, bytes).map_err(|e| format!("写入临时配置失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("替换 hooks.json 失败: {e}")
    })
}

/// 写 PowerShell 脚本：PS 5.1 需要 UTF-8 BOM，否则中文按 ANSI 解码会语法错误。
fn write_ps1(path: &Path, content: &str) -> Result<(), String> {
    let body = content.trim_start_matches('\u{feff}');
    let mut bytes = Vec::with_capacity(body.len() + 3);
    bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    bytes.extend_from_slice(body.as_bytes());
    std::fs::write(path, bytes).map_err(|e| format!("写入 {} 失败: {e}", path.display()))
}

fn command_for(script: &Path) -> String {
    let path = script.display();
    format!("powershell -NoProfile -ExecutionPolicy Bypass -File \"{path}\"")
}

/// 幂等写入：已有 LRC 条目则把命令更新为当前绝对路径（保留 timeout/loop_limit）。
fn ensure_entry(
    hooks_obj: &mut serde_json::Map<String, Value>,
    event: &str,
    needle: &str,
    command: String,
    timeout: u64,
    loop_limit: Option<u64>,
) -> Result<(), String> {
    let arr = hooks_obj.entry(event).or_insert_with(|| json!([]));
    if !arr.is_array() {
        return Err(format!("事件 {event} 不是数组，拒绝覆盖（请手动检查）"));
    }
    let entries = arr.as_array_mut().expect("上面已保证是数组");
    for entry in entries.iter_mut() {
        let Some(hooks) = entry.get_mut("hooks").and_then(Value::as_array_mut) else {
            continue;
        };
        for hook in hooks.iter_mut() {
            let is_lrc = hook
                .get("command")
                .and_then(Value::as_str)
                .map(|cmd| cmd.contains(needle))
                .unwrap_or(false);
            if is_lrc {
                // 修复手工配置的相对路径 / 旧安装路径。
                hook["command"] = json!(command);
                return Ok(());
            }
        }
    }
    let mut entry = json!({
        "hooks": [{
            "type": "command",
            "command": command,
            "timeout": timeout
        }]
    });
    if let Some(limit) = loop_limit {
        entry["loop_limit"] = json!(limit);
    }
    entries.push(entry);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lrc-hooks-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        dir
    }

    fn commands_of(cfg: &Value, event: &str) -> Vec<String> {
        cfg["hooks"][event]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .flat_map(|entry| {
                        entry
                            .get("hooks")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default()
                    })
                    .filter_map(|hook| {
                        hook.get("command")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn install_is_idempotent_and_preserves_existing_hooks() {
        let home = temp_home("idempotent");
        let trae_dir = home.join(".trae-cn");
        std::fs::create_dir_all(&trae_dir).unwrap();
        std::fs::write(
            trae_dir.join("hooks.json"),
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"node gate.js","timeout":30}]}]}}"#,
        )
        .unwrap();

        let first = install_trae_hooks(&home, "trae-cn").expect("首次安装");
        assert!(first.changed, "首次安装应改动配置");
        assert!(first.backup.is_some(), "已有配置必须备份");
        let cfg: Value =
            serde_json::from_str(&std::fs::read_to_string(&first.hooks_path).unwrap()).unwrap();
        let stop = commands_of(&cfg, "Stop");
        assert!(stop.iter().any(|c| c.contains("gate.js")), "用户已有钩子不得丢失");
        assert!(stop.iter().any(|c| c.contains("lrc-stop.ps1")));
        assert!(commands_of(&cfg, "SessionStart")
            .iter()
            .any(|c| c.contains("lrc-session-start.ps1")));
        assert!(commands_of(&cfg, "UserPromptSubmit")
            .iter()
            .any(|c| c.contains("lrc-user-prompt-submit.ps1")));
        for script in &first.scripts {
            assert!(script.exists(), "脚本应写入: {}", script.display());
            let bytes = std::fs::read(script).unwrap();
            assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF], "PS1 必须带 UTF-8 BOM");
        }

        let second = install_trae_hooks(&home, "trae-cn").expect("二次安装");
        assert!(!second.changed, "幂等：第二次不应再改动配置");
        let cfg2: Value =
            serde_json::from_str(&std::fs::read_to_string(&second.hooks_path).unwrap()).unwrap();
        assert_eq!(
            commands_of(&cfg2, "Stop").len(),
            stop.len(),
            "重复安装不得追加重复条目"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn install_creates_config_and_enabled_folders_when_missing() {
        let home = temp_home("fresh");
        let report = install_trae_hooks(&home, "trae-cn").expect("安装");
        assert!(report.changed);
        let cfg: Value =
            serde_json::from_str(&std::fs::read_to_string(&report.hooks_path).unwrap()).unwrap();
        let folders = cfg["enabled_folders"].as_array().expect("enabled_folders");
        assert!(
            folders
                .iter()
                .any(|v| v.as_str().unwrap_or("").contains("/.trae-cn/hooks")),
            "必须启用钩子目录: {folders:?}"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn install_repairs_relative_command_and_keeps_user_fields() {
        let home = temp_home("repair");
        let trae_dir = home.join(".trae");
        std::fs::create_dir_all(&trae_dir).unwrap();
        // README 方式 B 的相对路径写法：应被修复为绝对路径，且保留 timeout。
        std::fs::write(
            trae_dir.join("hooks.json"),
            r#"{"hooks":{"Stop":[{"loop_limit":9,"hooks":[{"type":"command","command":"powershell -File .\\.trae\\hooks\\lrc-stop.ps1","timeout":30}]}]}}"#,
        )
        .unwrap();
        let report = install_trae_hooks(&home, "trae").expect("安装");
        assert!(report.changed);
        let cfg: Value =
            serde_json::from_str(&std::fs::read_to_string(&report.hooks_path).unwrap()).unwrap();
        let entry = &cfg["hooks"]["Stop"][0];
        assert_eq!(entry["loop_limit"], 9, "用户字段不得丢失");
        let cmd = entry["hooks"][0]["command"].as_str().unwrap();
        assert!(cmd.contains("lrc-stop.ps1"));
        assert!(cmd.contains(".trae\\hooks\\lrc-stop.ps1"), "应为绝对路径: {cmd}");
        assert!(cmd.starts_with("powershell"), "{cmd}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn install_refuses_corrupt_or_non_object_config() {
        let home = temp_home("corrupt");
        let trae_dir = home.join(".trae-cn");
        std::fs::create_dir_all(&trae_dir).unwrap();
        let path = trae_dir.join("hooks.json");
        std::fs::write(&path, b"not-json").unwrap();
        let err = install_trae_hooks(&home, "trae-cn").expect_err("损坏 JSON 必须拒绝");
        assert!(err.contains("不是合法 JSON"), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), b"not-json", "原文件不得被覆盖");

        std::fs::write(&path, b"[]").unwrap();
        let err = install_trae_hooks(&home, "trae-cn").expect_err("顶层非对象必须拒绝");
        assert!(err.contains("顶层不是 JSON 对象"), "{err}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn install_reads_config_with_utf8_bom() {
        let home = temp_home("bom");
        let trae_dir = home.join(".trae-cn");
        std::fs::create_dir_all(&trae_dir).unwrap();
        let path = trae_dir.join("hooks.json");
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(br#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"node gate.js","timeout":30}]}]}}"#);
        std::fs::write(&path, bytes).unwrap();

        let report = install_trae_hooks(&home, "trae-cn").expect("BOM 配置应可读取");
        assert!(report.changed);
        let cfg: Value =
            serde_json::from_slice(&std::fs::read(&report.hooks_path).unwrap()).unwrap();
        assert!(commands_of(&cfg, "Stop")
            .iter()
            .any(|c| c.contains("gate.js")));
        let _ = std::fs::remove_dir_all(&home);
    }
}
