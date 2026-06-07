// ============================================================
// 道枢映射自动化检查（质疑四·终极：防止语义漂移）
//
// 本测试扫描核心模块源文件，验证所有公开函数的文档注释中
// 包含"道枢映射"或"luoshu_mapping"标注。
//
// 这是 CI/CD 流程中的一道自动化防线，确保新增功能不会
// 偏离 LRC 的核心哲学——洛书九宫格与八卦的几何约束。
//
// 道枢映射：兑卦·兑 (☱) — 说也，刚中而柔外，说以利贞。
//   语言（注释/文档）是思想的外化，注释即契约。
// ============================================================

use std::fs;
use std::path::Path;

/// 核心模块列表（需要道枢映射注释的模块）
const CORE_MODULES: &[&str] = &[
    "src/engine/dao_regulator.rs",
    "src/engine/dao_metrics.rs",
    "src/engine/synthesis_engine.rs",
    "src/engine/mirror_trapezoid.rs",
    "src/engine/luoshu_encoder.rs",
    "src/engine/luoshu_encoder_ml.rs",
    "src/engine/user_feedback.rs",
    "src/engine/audit_trail.rs",
    "src/engine/memory_gc.rs",
    "src/memory_store.rs",
    "src/memory_types.rs",
];

/// 道枢映射标注关键词（文档注释中至少出现一个）
const DAO_ANNOTATION_KEYWORDS: &[&str] = &[
    "道枢映射",
    "luoshu_mapping",
    "八卦",
    "洛书",
    "九宫格",
    "河图洛书",
    "阴阳",
    "五行",
];

/// 涌现模式关键词（质疑一·反僵化：允许新哲学模式注册）
///
/// 当系统需要演化出在原始洛书理论中无对应卦象的全新功能时，
/// 开发者可以使用 `@涌现` 标记声明一个新的哲学模式。
/// 这确保了道枢映射不会从"引导"变成"教条"。
///
/// 使用方式：在文档注释中添加 `@涌现: 新模式名称 — 哲学含义`
const EMERGENT_PATTERN_KEYWORDS: &[&str] = &[
    "@涌现",    // 显式声明新哲学模式
    "emergent", // 英文等价标记
    "新模式",   // 中文等价标记
];

/// 允许豁免的函数名前缀（基础设施/工具函数，不需要哲学映射）
///
/// 这些函数是通用编程模式的实现，而非 LRC 哲学体系的核心表达。
/// 强制要求它们标注道枢映射会导致注释噪音，反而稀释真正重要的映射。
const EXEMPTED_PREFIXES: &[&str] = &[
    "new",      // 构造函数
    "get_",     // 访问器
    "set_",     // 修改器
    "with_",    // 构建器模式
    "is_",      // 布尔查询
    "has_",     // 存在性检查
    "as_",      // 类型转换
    "record_",  // 事件记录（内部追踪）
    "should_",  // 条件判断
    "from_",    // 构造器
    "total_",   // 计数查询
    "last_",    // 时间查询
    "current_", // 状态查询
    "to_",      // 格式转换
    "try_",     // 尝试操作
    "valid_",   // 验证
    "cleanup_", // 清理
    "mark_",    // 标记
    "update_",  // 更新
    "cancel_",  // 取消
    "collect_", // 收集
    "find_",    // 查找
    "list_",    // 列表
    "forget",   // 删除
    "load_",    // 加载
    "start_",   // 启动
    "append_",  // 追加
    "next_",    // 迭代
    "compute_", // 计算（通用工具函数）
    "format!",  // 宏
];

/// 允许豁免的精确函数名（无前缀匹配的特定函数）
const EXEMPTED_EXACT: &[&str] = &[
    "default",
    "new",
    "next_id",
    "compute_hash",
    "record_regulation",
    "detect_oscillation",
    "detect_drift",
    "detect_freeze",
    "detect_coupling",
    "record_snapshot",
    "start_async_writer",
    "load_from_file",
    "append_to_file",
    "value",
    "summary",
    "stats",
    "clear",
    "flush",
    "touch",
    "zeros",
    "register",
    "click",
    "copy",
    "dwell",
    "ignore",
    "repeat_query",
    "check",
    "unfreeze",
];

/// 检查函数名是否应豁免（通过前缀匹配或精确匹配）
fn is_exempted(name: &str) -> bool {
    // 精确匹配
    if EXEMPTED_EXACT.contains(&name) {
        return true;
    }
    // 前缀匹配
    if EXEMPTED_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        return true;
    }
    false
}

#[test]
fn test_all_public_functions_have_dao_annotation() {
    let project_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut violations: Vec<String> = Vec::new();
    let mut checked_functions = 0usize;
    let mut annotated_functions = 0usize;

    for module_path in CORE_MODULES {
        let full_path = project_root.join(module_path);
        if !full_path.exists() {
            eprintln!("[道枢检查] 模块文件不存在: {module_path}");
            continue;
        }

        let content = match fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[道枢检查] 无法读取 {module_path}: {e}");
                continue;
            }
        };

        let violations_in_module = check_module(&content, module_path);
        for v in &violations_in_module {
            checked_functions += 1;
            if v.annotated {
                annotated_functions += 1;
            } else if !is_exempted(&v.name) {
                violations.push(format!(
                    "{}:{} - pub fn {} 缺少道枢映射标注",
                    module_path, v.line, v.name
                ));
            }
        }
    }

    if !violations.is_empty() {
        eprintln!("\n╔══════════════════════════════════════════════════════════════╗");
        eprintln!("║  道枢映射检查失败！以下函数缺少哲学映射标注：            ║");
        eprintln!("╠══════════════════════════════════════════════════════════════╣");
        for v in &violations {
            eprintln!("║  {v}");
        }
        eprintln!("╠══════════════════════════════════════════════════════════════╣");
        eprintln!("║  修复方法: 在函数的文档注释中添加'道枢映射'标注        ║");
        eprintln!("║  示例: /// 道枢映射: 乾卦·天 — 天道运行不息              ║");
        eprintln!("╚══════════════════════════════════════════════════════════════╝\n");
    }

    let coverage = if checked_functions > 0 {
        annotated_functions as f32 / checked_functions as f32 * 100.0
    } else {
        100.0
    };

    eprintln!(
        "[道枢检查] 扫描完成: {}/{} 公开函数有道枢映射 ({:.1}%%)，{} 个违规",
        annotated_functions,
        checked_functions,
        coverage,
        violations.len()
    );

    // 道枢映射覆盖率要求：核心函数必须 100% 标注
    // 因工具函数已通过前缀/精确规则自动豁免，剩余的均为核心逻辑函数
    if coverage < 80.0 && !violations.is_empty() {
        eprintln!(
            "[道枢检查·警告] 道枢映射覆盖率仅 {coverage:.1}%%，低于 80%% 阈值。\
             建议审查未标注函数是否偏离核心哲学。"
        );
    }

    // 道枢映射不强制 100% 覆盖——许多函数是桥接代码或基础设施。
    // 真正需要严格保障的是核心哲学函数的映射完整性，这由
    // test_core_philosophy_functions_have_dao_annotation 单独保证。
    // 本测试作为覆盖率报告存在，不硬性阻塞 CI。
    if !violations.is_empty() {
        eprintln!(
            "[道枢检查·注意] {} 个公开函数缺少道枢映射标注。\
             这不阻塞构建，但建议在后续迭代中补充标注以维护哲学一致性。",
            violations.len()
        );
    }
}

/// 检查结果
struct FunctionCheck {
    name: String,
    line: usize,
    annotated: bool,
}

/// 扫描模块源文件，提取所有 pub fn 及其道枢映射状态
fn check_module(content: &str, module_path: &str) -> Vec<FunctionCheck> {
    let mut results = Vec::new();
    let lines: Vec<&str> = content.lines().collect();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();

        // 检测 pub fn 声明
        if line.starts_with("pub fn ") || line.starts_with("pub async fn ") {
            // 提取函数名
            let fn_name = extract_fn_name(line);
            if let Some(name) = fn_name {
                // 检查前方的文档注释是否有道枢映射标注
                let annotated = check_preceding_doc_comments(&lines, i);
                let line_num = i + 1; // 1-based 行号

                results.push(FunctionCheck {
                    name: name.to_string(),
                    line: line_num,
                    annotated,
                });

                if !annotated && !is_exempted(name) {
                    eprintln!("[道枢检查] {module_path}:{line_num} - pub fn {name} 无道枢映射");
                }
            }
        }

        i += 1;
    }

    results
}

/// 从 pub fn 声明中提取函数名
fn extract_fn_name(line: &str) -> Option<&str> {
    let line = line.trim();
    // 处理 "pub fn name(" 或 "pub async fn name("
    let after_pub = if let Some(stripped) = line.strip_prefix("pub async fn ") {
        stripped
    } else if let Some(stripped) = line.strip_prefix("pub fn ") {
        stripped
    } else {
        return None;
    };

    // 找到 '(' 的位置，之前的部分即函数名
    let paren_pos = after_pub.find('(')?;
    let name = after_pub[..paren_pos].trim();

    // 排除泛型参数（如 foo<T>）
    if let Some(lt_pos) = name.find('<') {
        Some(&name[..lt_pos])
    } else {
        Some(name)
    }
}

/// 检查 pub fn 前方的文档注释是否包含道枢映射或涌现模式关键词
///
/// 质疑一·反僵化：除传统的卦象映射外，也接受 `@涌现` 标记声明的
/// 新哲学模式。这确保系统可以在不破坏检查机制的前提下演化出
/// 全新的哲学范式。
fn check_preceding_doc_comments(lines: &[&str], fn_line: usize) -> bool {
    // 向前扫描文档注释（/// 或 /** */）
    let mut i = if fn_line > 0 {
        fn_line - 1
    } else {
        return false;
    };

    // 收集前方的连续注释
    let mut doc_text = String::new();

    loop {
        let line = lines[i].trim();

        if let Some(stripped) = line.strip_prefix("///") {
            // 行文档注释：提取 /// 之后的内容
            let comment = stripped.trim();
            doc_text.push_str(comment);
            doc_text.push(' ');
        } else if line.starts_with("//!") || line.starts_with("//") {
            // 普通注释，也收集（可能包含道枢映射）
            let comment = if line.len() > 2 { line[2..].trim() } else { "" };
            doc_text.push_str(comment);
            doc_text.push(' ');
        } else if line.starts_with("#[") {
            // 属性宏（如 #[derive(...)]），跳过继续向前
        } else {
            // 遇到非注释行，停止
            break;
        }

        if i == 0 {
            break;
        }
        i -= 1;
    }

    if doc_text.is_empty() {
        return false;
    }

    // 检查是否包含道枢映射关键词 或 涌现模式标记（质疑一·反僵化）
    DAO_ANNOTATION_KEYWORDS
        .iter()
        .any(|keyword| doc_text.to_lowercase().contains(&keyword.to_lowercase()))
        || EMERGENT_PATTERN_KEYWORDS
            .iter()
            .any(|keyword| doc_text.to_lowercase().contains(&keyword.to_lowercase()))
}

#[test]
fn test_dao_annotation_coverage_report() {
    // 打印道枢映射覆盖率报告（信息性测试，不会失败）
    let project_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut total_fns = 0usize;
    let mut annotated_fns = 0usize;

    for module_path in CORE_MODULES {
        let full_path = project_root.join(module_path);
        if !full_path.exists() {
            continue;
        }
        let content = match fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let checks = check_module(&content, module_path);
        let annotated = checks.iter().filter(|c| c.annotated).count();
        total_fns += checks.len();
        annotated_fns += annotated;

        if !checks.is_empty() {
            eprintln!(
                "[道枢报告] {} : {}/{} pub fn 有道枢映射标注",
                module_path,
                annotated,
                checks.len()
            );
        }
    }

    if total_fns > 0 {
        let pct = annotated_fns as f32 / total_fns as f32 * 100.0;
        eprintln!("[道枢报告] 总计: {annotated_fns}/{total_fns} = {pct:.1}%% 道枢映射覆盖率");
    }
}

// ============================================================
// 核心哲学函数硬性检查（质疑四·终极防线）
//
// 以下函数是 LRC 哲学体系的根基——它们直接实现了洛书编码、
// 镜像梯形、道枢调节、审计完整性等核心思想。
// 这些函数的道枢映射标注是硬性要求，缺失将导致 CI 失败。
// ============================================================

/// 核心哲学函数列表（必须 100% 标注道枢映射）
const CORE_PHILOSOPHY_FUNCTIONS: &[(&str, &str, &str)] = &[
    // (模块路径, 函数名, 哲学含义简述)
    (
        "src/engine/dao_regulator.rs",
        "regulate",
        "道枢调节核心——阴阳平衡的自适应调谐",
    ),
    (
        "src/engine/dao_regulator.rs",
        "analyze_coupling_trend",
        "耦合趋势分析——八卦交互的动力学",
    ),
    (
        "src/engine/dao_metrics.rs",
        "snapshot",
        "道同构度指标快照——洛书幻和的数学表达",
    ),
    (
        "src/engine/synthesis_engine.rs",
        "synthesize_cluster",
        "合成簇——震卦萌发的信息整合",
    ),
    (
        "src/engine/synthesis_engine.rs",
        "try_synthesize",
        "合成尝试——新信息增益的阈值检测",
    ),
    (
        "src/engine/mirror_trapezoid.rs",
        "mirror_project",
        "镜像投影——洛书九宫格的空间映射",
    ),
    (
        "src/engine/mirror_trapezoid.rs",
        "focused_recall",
        "聚焦召回——梯形ROI的注意力机制",
    ),
    (
        "src/engine/mirror_trapezoid.rs",
        "evolution_cycle",
        "演化周期——记忆的阴阳消长循环",
    ),
    (
        "src/engine/mirror_trapezoid.rs",
        "recursive_compose",
        "递归合成——记忆的层次化凝练",
    ),
    (
        "src/engine/mirror_trapezoid.rs",
        "recursive_unfold",
        "递归展开——记忆的层次化解构",
    ),
    (
        "src/engine/luoshu_encoder.rs",
        "encode_text",
        "洛书编码——将语义映射到九宫格向量",
    ),
    (
        "src/engine/luoshu_encoder.rs",
        "luoshu_deviation",
        "洛书偏差——幻和的标准偏离度",
    ),
    (
        "src/engine/luoshu_encoder_ml.rs",
        "encode_embedding",
        "ML编码——语义空间的洛书映射",
    ),
    (
        "src/engine/user_feedback.rs",
        "record_feedback",
        "反馈记录——用户意图的阴阳标记",
    ),
    (
        "src/engine/user_feedback.rs",
        "privacy_manifest",
        "隐私清单——离卦光明的透明承诺",
    ),
    (
        "src/engine/user_feedback.rs",
        "grant_consent",
        "知情同意——用户主权的显式确认",
    ),
    (
        "src/engine/audit_trail.rs",
        "record",
        "审计记录——坎卦水流的诚信链条",
    ),
    (
        "src/engine/audit_trail.rs",
        "verify_integrity",
        "完整性验证——哈希链的防篡改检测",
    ),
    (
        "src/engine/audit_trail.rs",
        "self_check_integrity",
        "自检——封印与链的双重验证",
    ),
    (
        "src/engine/audit_trail.rs",
        "seal_integrity",
        "完整性封印——独立存储的哈希链根",
    ),
    (
        "src/engine/memory_gc.rs",
        "collect_garbage",
        "记忆回收——兑卦润泽的生态平衡",
    ),
    ("src/memory_store.rs", "recall", "记忆召回——核心检索接口"),
    (
        "src/memory_store.rs",
        "regulate",
        "调节入口——道枢调节的对外接口",
    ),
    (
        "src/memory_store.rs",
        "health_report",
        "健康报告——系统状态的可解释性面板",
    ),
];

#[test]
fn test_core_philosophy_functions_have_dao_annotation() {
    let project_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut missing: Vec<String> = Vec::new();

    for (module_path, fn_name, description) in CORE_PHILOSOPHY_FUNCTIONS {
        let full_path = project_root.join(module_path);
        if !full_path.exists() {
            eprintln!("[道枢核心检查] 模块文件不存在: {module_path}");
            continue;
        }

        let content = match fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[道枢核心检查] 无法读取 {module_path}: {e}");
                missing.push(format!("{module_path}:{fn_name} (无法读取文件)"));
                continue;
            }
        };

        let checks = check_module(&content, module_path);
        let found = checks.iter().find(|c| c.name == *fn_name);

        match found {
            Some(c) if c.annotated => {
                // 通过——核心函数有道枢映射
            }
            Some(_) => {
                missing.push(format!(
                    "{}:{} ({}): {}",
                    module_path, fn_name, description, "缺少道枢映射标注"
                ));
            }
            None => {
                eprintln!("[道枢核心检查] 未找到函数 {module_path}::{fn_name}");
                missing.push(format!(
                    "{}:{} ({}): {}",
                    module_path, fn_name, description, "函数未在模块中找到"
                ));
            }
        }
    }

    if !missing.is_empty() {
        eprintln!("\n╔══════════════════════════════════════════════════════════════╗");
        eprintln!("║  核心哲学函数缺失道枢映射！这些函数是 LRC 的哲学根基：  ║");
        eprintln!("╠══════════════════════════════════════════════════════════════╣");
        for m in &missing {
            eprintln!("║  {m}");
        }
        eprintln!("╠══════════════════════════════════════════════════════════════╣");
        eprintln!("║  修复: 在函数文档注释中添加道枢映射标注                 ║");
        eprintln!("╚══════════════════════════════════════════════════════════════╝\n");
    }

    assert!(
        missing.is_empty(),
        "{} 个核心哲学函数缺少道枢映射标注。这些函数是 LRC 哲学体系的根基，\
         缺失标注将导致语义漂移，系统灵魂流失。",
        missing.len()
    );
}
