// ============================================================
// 许可证: Apache 2.0
// LRC 内置道体状态机 · 跨会话记忆丢失恢复集成测试
//
// 验证核心承诺：
//   1. remember 写入 = 当前语境 → 新记忆立即激活并持久化；
//   2. 下一次检索即使查询措辞改变（无词面重叠的语义相关），
//      活跃记忆能通过联想状态被召回——"记忆不丢失"；
//   3. 未激活的无关记忆不因活性偏置浮现（防污染）；
//   4. 状态机快照跨 store 实例持久化（restart 后活性保持）。
// ============================================================

use code_memory::memory_store::MemoryStore;
use code_memory::memory_types::{Importance, Memory, MemoryType};
use code_memory::JsonPersistence;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(tag: &str) -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let dir = std::env::temp_dir().join(format!("lrc_state_machine_{}_{}", tag, ts));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_str().unwrap().to_string()
}

fn new_store(dir: &str) -> MemoryStore<JsonPersistence> {
    let persistence = JsonPersistence::new(dir).unwrap();
    MemoryStore::new(persistence)
}

fn mk_memory(content: &str) -> Memory {
    Memory::new(
        content.to_string(),
        MemoryType::Conversation,
        None,
        vec![],
        Importance::new(5),
        None,
    )
}

fn recall_first(store: &mut MemoryStore<JsonPersistence>, query: &str) -> Vec<String> {
    let filter = code_memory::RecallFilter {
        top_k: 8,
        ..Default::default()
    };
    store
        .recall(query, &filter)
        .map(|r| {
            r.memories
                .iter()
                .map(|m| format!("{}:{}", m.id, m.content))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn 跨会话活性导航让无词重叠的关联记忆被找回() {
    // 用独立临时目录，隔离 LRC_STATE_BIAS 环境与库干扰
    let dir = temp_dir("nav_recall");
    let mut store = new_store(&dir);

    // 场景：昨晚写入"粤菜餐厅"记忆（今晚相关，但查询"今天吃什么"与其无词面重叠）
    let dinner_mem = mk_memory("我和小王昨晚去了珠江边的粤菜餐厅，潮汕牛肉丸很好吃");
    store.remember(dinner_mem).unwrap();

    // 第一轮：写入后新记忆激活（remember 激活语义）
    // 第二轮：换一个措辞完全不同的查询，验证活性偏置把该记忆带回
    let results = recall_first(&mut store, "今天晚饭吃什么好呢");
    assert!(
        results.iter().any(|c| c.contains("粤菜")),
        "活性导航应找回关联记忆，实际结果: {:?}",
        results
    );
}

#[test]
fn 无关记忆不因活性偏置浮动() {
    // 防止活性偏置造成"只要活跃就顶到前面"的污染
    let dir = temp_dir("nav_negative");
    let mut store = new_store(&dir);

    // 写入：一条高相关但非活跃的记忆（语义上应优先），一条不相关记忆
    let high_rel = mk_memory("加班到十点，买了份泡面当晚饭，明早还要开晨会");
    let unrelated = mk_memory("给绿萝浇水，每周换一次盆土");
    store.remember(high_rel).unwrap();
    store.remember(unrelated).unwrap();

    // 直接第一次查询（两条都未激活，只有内容分）
    let first = recall_first(&mut store, "晚饭吃了什么");
    // 高相关记忆应排在无关记忆前
    let idx_high = first.iter().position(|c| c.contains("泡面")).unwrap_or(99);
    let idx_unrel = first.iter().position(|c| c.contains("绿萝")).unwrap_or(99);
    assert!(
        idx_high < idx_unrel,
        "未激活时语义相关应优先，实际: {:?}",
        first
    );
}

#[test]
fn 活性状态跨store实例持久化() {
    // 验证 restart 后（新建 MemoryStore 指向同一数据目录）活性被加载
    let dir = temp_dir("nav_restart");
    {
        let mut store = new_store(&dir);
        let mem = mk_memory("儿子期末考数学，应用题和错题本要重点复习");
        store.remember(mem).unwrap();

        // 先激活它
        let _ = recall_first(&mut store, "期末数学复习");
    }

    // 模拟服务重启：全新 store 指向同一目录，应从 memory_state.json 恢复活性
    let store2 = new_store(&dir);

    // 存放路径由 JsonPersistence 控制，断言状态文件存在且可恢复
    let state_path = PathBuf::from(&dir).join("memory_state.json");
    assert!(
        state_path.exists(),
        "state machine snapshot 应持久化到 memory_state.json"
    );
    let state = store2.memory_state_machine.snapshot();
    assert!(
        !state.active.is_empty(),
        "重启后应恢复激活记忆: {:?}",
        state.active
    );
    assert!(
        state.active.iter().any(|a| a.hits >= 1),
        "激活记忆应带命中计数"
    );
}

#[test]
fn 联想桥扩散拉回无词面重叠的孤立记忆() {
    // 核心联想能力：通过活跃记忆内容的联想桥，找回与查询无词面重叠的孤立记忆。
    // 场景：
    //   1. 记下"朋友阿龙在泉州做木偶戏"（孤立记忆，与后续查询无词重叠）
    //   2. 查询"最近值得记录的人和事"（激活一个相关话题，如"出差见闻"）
    //   3. 通过活跃记忆内容作为联想桥，阿龙那条记忆应能进入结果
    let dir = temp_dir("nav_bridge");
    let mut store = new_store(&dir);

    // 两条活跃种子：都与"见闻/出差"话题相关，但内容不同词
    store
        .remember(mk_memory("上个月出差泉州，见到了做提线木偶的老艺人阿龙"))
        .unwrap();
    store
        .remember(mk_memory("阿龙的木偶坊在旧城区，传了三代"))
        .unwrap();

    // 用相关话题激活这些记忆（制造活跃状态）
    let _ = recall_first(&mut store, "出差 泉州 木偶");

    // 新查询与活跃记忆的词面重叠很低，但语义相关（"值得记录的事"）
    // 活跃记忆内容中的"阿龙/木偶坊/旧城区"作为联想桥，应把种子记忆带到前排
    let results = recall_first(&mut store, "有哪些值得写进周报的见闻");
    assert!(
        results.iter().any(|c| c.contains("阿龙")),
        "联想桥应把孤立但相关的记忆拉回: {:?}",
        results
    );
}

#[test]
fn 联想扩散不污染直接命中的记忆() {
    // 回归约束：原查询直接命中的记忆必须排在联想桥拉回的记忆之前，
    // 证明扩展词不干扰原查询主导地位。
    let dir = temp_dir("nav_no_pollute");
    let mut store = new_store(&dir);

    // 直接命中的记忆（含查询词"会议"）
    store
        .remember(mk_memory("今天下午三点开产品评审会议，讨论季度路线图"))
        .unwrap();
    // 联想种子（不含"会议"，但通过活跃桥可能被拉回）
    store
        .remember(mk_memory("季度路线图初稿已经发到群里了"))
        .unwrap();

    // 先激活联想种子
    let _ = recall_first(&mut store, "路线图 季度");

    // 查询明确包含"会议"
    let results = recall_first(&mut store, "下午的会议几点开始");
    let idx_direct = results
        .iter()
        .position(|c| c.contains("评审会议"))
        .unwrap_or(99);
    let idx_bridge = results
        .iter()
        .position(|c| c.contains("初稿"))
        .unwrap_or(99);
    assert!(
        idx_direct < idx_bridge,
        "原查询直接命中应优先于联想桥拉回: {:?}",
        results
    );
}

#[test]
fn deep路径活跃记忆免于八卦剪除() {
    // deep 路径：活跃记忆无论卦象一律进入候选（白名单），
    // 验证即使八卦分类不同也不会被硬剪除丢记忆。
    let dir = temp_dir("nav_deep_whitelist");
    let mut store = new_store(&dir);

    let mem = mk_memory("喜欢在雨天听坂本龙一的曲子，尤其是圣诞快乐劳伦斯先生");
    store.remember(mem).unwrap();

    // 用直接相关查询激活
    let _ = recall_first(&mut store, "雨天 音乐 坂本龙一");

    // deep 检索：即使查询词面差异大，活跃记忆也应在候选（白名单生效）
    let filter = code_memory::RecallFilter {
        top_k: 8,
        ..Default::default()
    };
    let result = store
        .trapezoid_focus_recall("有什么放松心情的方式", &filter, 2)
        .unwrap();
    assert!(
        result
            .memories
            .iter()
            .any(|m| m.content.contains("坂本龙一")),
        "deep 路径活跃记忆应被白名单保留: {:?}",
        result
            .memories
            .iter()
            .map(|m| m.content.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn 道体再次校验剔除碰巧共享词的噪声() {
    // 回归验证核心：联想桥词把一条"碰巧包含活跃记忆词但主题无关"的
    // 噪声记忆拉进候选，道体再次校验必须把它剔除。
    // 场景：
    //   1. 活跃记忆："出差泉州见了木偶艺人阿龙"（联想桥词：泉州/木偶/阿龙）
    //   2. 噪声记忆："泉州路是条老街"（碰巧含"泉州"，但与原查询、联想链都无关）
    //   3. 查询"有什么见闻值得记录"（原查询零重叠）
    //   → 噪声记忆桥命中仅 1（泉州），无其他共鸣 → 应被剔除
    let dir = temp_dir("nav_regression_reject");
    let mut store = new_store(&dir);

    store
        .remember(mk_memory("上个月出差泉州，见到了做提线木偶的老艺人阿龙"))
        .unwrap();
    store
        .remember(mk_memory("泉州路是条老街，两边种着梧桐树"))
        .unwrap();

    // 激活（联想桥词来自活跃记忆内容）
    let _ = recall_first(&mut store, "出差 泉州 木偶 阿龙");

    // 查询与原查询词零重叠
    let results = recall_first(&mut store, "有哪些值得写进周报的见闻");
    assert!(
        results.iter().any(|c| c.contains("阿龙")),
        "真正相关的联想记忆应保留: {:?}",
        results
    );
    assert!(
        !results.iter().any(|c| c.contains("梧桐树")),
        "仅碰巧共享一个桥词的噪声记忆应被道体再次校验剔除: {:?}",
        results
    );
}

#[test]
fn evidence_tags_alongside_recall_result() {
    // 可观测增强：联想扩散保留的记忆应携带"联想桥强关联"证据标签，
    // 调用方（server 联想链输出）能解释"为什么这条被联想回来"。
    let dir = temp_dir("nav_evidence_expose");
    let mut store = new_store(&dir);

    store
        .remember(mk_memory("出差泉州见了木偶艺人阿龙，他传了三代"))
        .unwrap();
    // 激活
    let _ = recall_first(&mut store, "出差 泉州 木偶 阿龙");

    // 联想桥查询（原查询零重叠）→ 应触发联想导航 + 再次校验 + 证据标签
    let filter = code_memory::RecallFilter {
        top_k: 8,
        ..Default::default()
    };
    let result = store.recall("有哪些值得写进周报的见闻", &filter).unwrap();

    // 阿龙记忆应被保留并携带回归证据
    let evidence_found = result
        .regression_evidence
        .iter()
        .any(|(_, evidence)| evidence.contains("联想桥强关联"));
    assert!(
        evidence_found,
        "联想桥拉回的记忆应带'联想桥强关联'证据: {:?}",
        result.regression_evidence
    );
    // 证据的 id 应指向实际返回的记忆
    let returned_ids: Vec<String> = result.memories.iter().map(|m| m.id.clone()).collect();
    assert!(
        result
            .regression_evidence
            .keys()
            .all(|id| returned_ids.contains(id)),
        "证据 id 应都在返回结果中: {:?}",
        result.regression_evidence
    );
}
