// ============================================================
// v0.9.7 structural refactor (GLOBAL_CODE_REVIEW_REPORT P2-4:
//   "v1_api.rs test ratio 50%").
// ============================================================
// This file used to be the inline test island at the end of
// src/v1_api.rs (old lines 4530..EOF, about half of that file).
// After extraction:
//   - v1_api.rs keeps production code only; the "test ratio" metric
//     is no longer inflated;
//   - it is included from v1_api.rs via:
//       #[cfg(test)] #[path = "v1_api_tests.rs"] mod v1_api_tests;
//   - former `use super::*;` became `use crate::v1_api::*;` so that
//     item resolution does not depend on nesting depth (byte-equivalent
//     semantics to before).
// Behaviour impact: none. Test count, names and assertions are unchanged.

#[cfg(test)]
mod version_tests {
    use crate::v1_api::*;

    #[test]
    fn test_compare_versions() {
        assert!(compare_versions("0.2.0", "0.1.0"));
        assert!(!compare_versions("0.1.0", "0.2.0"));
        assert!(!compare_versions("0.2.0", "0.2.0"));
        assert!(compare_versions("1.0.0", "0.9.9"));
        assert!(!compare_versions("0.1.0", "1.0.0"));
        assert!(compare_versions("0.2.1", "0.2.0"));
    }

    // v0.7.1 P4-1 补充：compare_versions 边界场景
    #[test]
    fn test_compare_versions_edge_cases() {
        // 空字符串应安全降级为 false（不升级）
        assert!(!compare_versions("", "0.1.0"));
        assert!(!compare_versions("0.1.0", ""));
        // 非法格式（含非数字段）应过滤后返回 false
        assert!(!compare_versions("x.y.z", "0.1.0"));
        // 多段版本号：补 0 对齐比较
        assert!(compare_versions("0.2.0.1", "0.2.0"));
        assert!(!compare_versions("0.2.0", "0.2.0.1"));
        // 大版本号跨越
        assert!(compare_versions("2.0.0", "1.9.9.9"));
    }
}

// ──────────────────────────────────────────────────────────────
// v0.7.1 P4-1：v1_api.rs 核心端点单元测试
// ──────────────────────────────────────────────────────────────
// 说明：22 个端点均以闭包形式注册到 Router，handler 未暴露为命名函数，
//       因此无法直接调用 handler 做纯单元测试。此处采用三层测试策略：
//   1. 纯函数测试（default_* 系列）
//   2. 请求体 serde 默认值测试（验证 API 契约的向后兼容性）
//   3. 响应体序列化字段名测试（确保前端可正确解析 JSON）
// 完整端点级集成测试由 server.rs 中的 axum integration tests 覆盖。
#[cfg(test)]
mod api_contracts_tests {
    use crate::v1_api::*;

    // ===== 1. 纯函数测试：default_* 系列确保默认值稳定 =====

    #[test]
    fn test_default_synthesis_similarity() {
        assert_eq!(default_synthesis_similarity(), 0.4);
    }

    #[test]
    fn test_default_min_cluster() {
        assert_eq!(default_min_cluster(), 3);
    }

    #[test]
    fn test_default_memory_type() {
        assert_eq!(default_memory_type(), "fact");
    }

    #[test]
    fn test_default_importance() {
        assert_eq!(default_importance(), 5);
    }

    #[test]
    fn test_default_privacy() {
        assert_eq!(default_privacy(), "user");
    }

    #[test]
    fn test_default_top_k() {
        assert_eq!(default_top_k(), 5);
    }

    #[test]
    fn test_default_min_activation() {
        assert_eq!(default_min_activation(), 0.1);
    }

    // ===== 2. 请求体 serde 默认值测试（API 契约向后兼容性） =====

    #[test]
    fn test_consolidate_request_serde_defaults() {
        // 最小请求体：仅提供 memories 字段，其余字段应使用 serde default
        let json = r#"{"memories":[{"content":"测试记忆"}]}"#;
        let req: ConsolidateRequest = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(req.memories.len(), 1);
        assert_eq!(
            req.synthesis_similarity, 0.4,
            "synthesis_similarity 默认值应为 0.4"
        );
        assert_eq!(req.min_cluster, 3, "min_cluster 默认值应为 3");
        // ConsolidateMemory 的默认值
        assert_eq!(req.memories[0].memory_type, "fact");
        assert_eq!(req.memories[0].importance, 5);
        assert_eq!(req.memories[0].privacy_level, "user");
        assert!(req.memories[0].tags.is_empty());
        assert!(req.memories[0].project.is_none());
        assert!(req.memories[0].session_id.is_none());
        assert!(req.memories[0].user_id.is_none());
    }

    #[test]
    fn test_consolidate_request_explicit_values() {
        // 显式提供所有字段，确保不被默认值覆盖
        let json = r#"{
            "memories":[{"content":"显式","memory_type":"decision","importance":9,"privacy_level":"team"}],
            "synthesis_similarity":0.6,
            "min_cluster":5
        }"#;
        let req: ConsolidateRequest = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(req.synthesis_similarity, 0.6);
        assert_eq!(req.min_cluster, 5);
        assert_eq!(req.memories[0].memory_type, "decision");
        assert_eq!(req.memories[0].importance, 9);
        assert_eq!(req.memories[0].privacy_level, "team");
    }

    #[test]
    fn test_enrich_request_serde_defaults() {
        let json = r#"{"query":"Rust 开发"}"#;
        let req: EnrichRequest = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(req.query, "Rust 开发");
        assert_eq!(req.top_k, 5, "top_k 默认值应为 5");
        assert!(req.session_id.is_none());
        assert!(req.user_id.is_none());
    }

    #[test]
    fn test_unfold_request_serde_defaults() {
        let json = r#"{"memory_id":"mem-001"}"#;
        let req: UnfoldRequest = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(req.memory_id, "mem-001");
        assert_eq!(req.min_activation, 0.1, "min_activation 默认值应为 0.1");
    }

    #[test]
    fn test_correct_request_serde_defaults() {
        // reason 字段 #[serde(default)]，缺失时应为 None
        let json = r#"{"memory_id":"mem-001","content":"修正内容"}"#;
        let req: CorrectRequest = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(req.memory_id, "mem-001");
        assert_eq!(req.content, "修正内容");
        assert!(req.reason.is_none(), "reason 默认应为 None");
    }

    #[test]
    fn test_correct_request_with_reason() {
        let json = r#"{"memory_id":"mem-001","content":"修正","reason":"用户指出错误"}"#;
        let req: CorrectRequest = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(req.reason, Some("用户指出错误".to_string()));
    }

    #[test]
    fn test_forget_request_serde() {
        // 最小请求体：仅提供 memory_id
        let json = r#"{"memory_id":"mem-001"}"#;
        let req: ForgetRequest = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(req.memory_id, "mem-001");
    }

    #[test]
    fn test_forget_request_requires_memory_id() {
        // memory_id 是必填字段，缺失应反序列化失败
        let json = r#"{}"#;
        let result: Result<ForgetRequest, _> = serde_json::from_str(json);
        assert!(result.is_err(), "缺失 memory_id 字段应反序列化失败");
    }

    #[test]
    fn test_associations_request_serde() {
        // 最小请求体：仅 memory_id，relation 缺省表示"全部类型并存"
        let json = r#"{"memory_id":"mem-001"}"#;
        let req: MemoryAssociationsRequest = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(req.memory_id, "mem-001");
        assert!(
            req.relation.is_none(),
            "relation 缺省应为 None（返回全部类型）"
        );
    }

    #[test]
    fn test_associations_request_with_relation_filter() {
        let json = r#"{"memory_id":"mem-001","relation":"same_event"}"#;
        let req: MemoryAssociationsRequest = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(req.relation.as_deref(), Some("same_event"));
    }

    #[test]
    fn test_associations_request_requires_memory_id() {
        let json = r#"{}"#;
        let result: Result<MemoryAssociationsRequest, _> = serde_json::from_str(json);
        assert!(result.is_err(), "缺失 memory_id 字段应反序列化失败");
    }

    #[test]
    fn test_encode_request_required_fields() {
        // text 是必填字段，缺失应反序列化失败
        let json = r#"{}"#;
        let result: Result<EncodeRequest, _> = serde_json::from_str(json);
        assert!(result.is_err(), "缺失 text 字段应反序列化失败");
    }

    // ===== 3. 响应体序列化字段名测试（前端契约稳定性） =====

    #[test]
    fn test_encode_response_field_names() {
        let resp = EncodeResponse {
            luoshu_vector: [0.5; 9],
            bagua_index: 3,
            bagua_category: "震".to_string(),
            center_value: 0.5,
            topological_depth: 0.5,
        };
        let json = serde_json::to_value(&resp).expect("序列化失败");
        // 验证字段名与前端预期一致（snake_case）
        assert!(
            json.get("luoshu_vector").is_some(),
            "字段名应为 luoshu_vector"
        );
        assert!(json.get("bagua_index").is_some());
        assert!(json.get("bagua_category").is_some());
        assert!(json.get("center_value").is_some());
        assert!(json.get("topological_depth").is_some());
        // 验证数组长度为 9
        assert_eq!(json["luoshu_vector"].as_array().unwrap().len(), 9);
    }

    #[test]
    fn test_consolidate_response_field_names() {
        let resp = ConsolidateResponse {
            stored: 3,
            synthesized: 1,
            total_memories: 4,
            synthesis_summaries: vec!["合成摘要".to_string()],
        };
        let json = serde_json::to_value(&resp).expect("序列化失败");
        assert_eq!(json["stored"], 3);
        assert_eq!(json["synthesized"], 1);
        assert_eq!(json["total_memories"], 4);
        assert!(json["synthesis_summaries"].is_array());
    }

    #[test]
    fn test_dao_metrics_response_field_names() {
        // v0.8.1：契约对齐后，响应包装为 {ok, data, raw} 嵌套结构
        let resp = DaoMetricsResponse {
            ok: true,
            data: DaoMetricsData {
                yin_yang_balance: 85.5,
                luoshu_deviation: 14.5,
                bagua_balance: 97.9,
                synthesis_ratio: 30.0,
                dao_isomorphism_score: 0.855,
                active_memories: 100,
                crystallized_memories: 30,
                status: "healthy".to_string(),
                regulator_heartbeat: None,
            },
            raw: DaoMetricsRaw {
                bagua_entropy: 2.1,
                archived_memories: 5,
                encodings_total: 1000,
                compositions_total: 50,
                recalls_total: 200,
                corrections_total: 10,
            },
        };
        let json = serde_json::to_value(&resp).expect("序列化失败");
        // 验证顶层嵌套结构（前端 loadDaoMetrics 依赖 ok/data）
        assert_eq!(json["ok"], true);
        // f32 → JSON 存在精度损失，浮点字段用容差比较
        let approx_eq = |a: f64, b: f64| (a - b).abs() < 1e-3;
        // 验证 data 字段（前端 dashboard 依赖这些名称）
        let data = &json["data"];
        assert!(approx_eq(data["yin_yang_balance"].as_f64().unwrap(), 85.5));
        assert!(approx_eq(data["luoshu_deviation"].as_f64().unwrap(), 14.5));
        assert!(approx_eq(data["bagua_balance"].as_f64().unwrap(), 97.9));
        assert!(approx_eq(data["synthesis_ratio"].as_f64().unwrap(), 30.0));
        assert!(approx_eq(
            data["dao_isomorphism_score"].as_f64().unwrap(),
            0.855
        ));
        assert_eq!(data["active_memories"], 100);
        assert_eq!(data["crystallized_memories"], 30);
        assert_eq!(data["status"], "healthy");
        // 验证 raw 字段（保留原始诊断信息）
        let raw = &json["raw"];
        assert!(approx_eq(raw["bagua_entropy"].as_f64().unwrap(), 2.1));
        assert_eq!(raw["archived_memories"], 5);
        assert_eq!(raw["encodings_total"], 1000);
        assert_eq!(raw["compositions_total"], 50);
        assert_eq!(raw["recalls_total"], 200);
        assert_eq!(raw["corrections_total"], 10);
    }

    #[test]
    fn test_unfold_response_field_names() {
        let resp = UnfoldResponse {
            success: true,
            source_memory_id: "mem-001".to_string(),
            sub_vectors_count: 3,
            fidelity: 0.95,
            sub_memories: vec![],
        };
        let json = serde_json::to_value(&resp).expect("序列化失败");
        assert_eq!(json["success"], true);
        assert_eq!(json["source_memory_id"], "mem-001");
        assert_eq!(json["sub_vectors_count"], 3);
        // f32 → JSON 精度损失，用容差比较
        assert!(
            (json["fidelity"].as_f64().unwrap() - 0.95).abs() < 1e-5,
            "fidelity 字段值异常"
        );
        assert!(json["sub_memories"].is_array());
    }

    #[test]
    fn test_correct_response_field_names() {
        let resp = CorrectResponse {
            success: true,
            memory_id: "mem-001".to_string(),
            new_version: 2,
            history_versions: 1,
        };
        let json = serde_json::to_value(&resp).expect("序列化失败");
        assert_eq!(json["success"], true);
        assert_eq!(json["memory_id"], "mem-001");
        assert_eq!(json["new_version"], 2);
        assert_eq!(json["history_versions"], 1);
    }

    #[test]
    fn test_enriched_memory_field_names() {
        let mem = EnrichedMemory {
            id: "mem-001".to_string(),
            content: "内容".to_string(),
            memory_type: "fact".to_string(),
            score: 0.85,
            bagua_category: Some("震".to_string()),
            daoti_preview_gua: None,
            daoti_preview_bagua: None,
            daoti_preview_version: None,
            importance: 7,
            topological_depth: 0.5,
            version: 1,
            created_at: "2026-07-29T00:00:00Z".to_string(),
            event_id: None,
            entities: Vec::new(),
            why: None,
        };
        let json = serde_json::to_value(&mem).expect("序列化失败");
        assert_eq!(json["id"], "mem-001");
        assert_eq!(json["memory_type"], "fact");
        // f32 → JSON 精度损失，用容差比较
        assert!(
            (json["score"].as_f64().unwrap() - 0.85).abs() < 1e-5,
            "score 字段值异常"
        );
        assert_eq!(json["bagua_category"], "震");
        assert_eq!(json["importance"], 7);
        assert_eq!(json["version"], 1);
    }

    /// 阶段D：EnrichResponse 联想解释块字段契约（只观测，不参与排序）。
    #[test]
    fn test_enrich_explanation_block_fields() {
        let resp = EnrichResponse {
            memories: vec![],
            fast_path_hits: 6,
            deep_path_hits: 4,
            total: 3,
            trail: vec![],
            regression_evidence: HashMap::new(),
            filtered_count: 0,
            association_mode: "state_machine_navigation".to_string(),
            explanation: EnrichExplanation {
                query: "Rust 编译失败 ModuleNotFoundError 怎么排查".to_string(),
                weights: ExplanationWeights {
                    fast: 1.0,
                    deep: 1.8,
                },
                rrf_k: 60.0,
                fast_path_hits: 6,
                deep_path_hits: 4,
                total_candidates: 3,
                items: vec![EnrichExplanationItem {
                    id: "mem-001".to_string(),
                    rank: 1,
                    score: 1.0,
                    fused_contrib: 0.0311,
                    fast_contrib: 0.0164,
                    deep_contrib: 0.0147,
                    fast_rank: Some(1),
                    deep_rank: Some(3),
                    hit_paths: vec!["fast", "deep"],
                }],
            },
            associated: vec![],
        };
        let json = serde_json::to_value(&resp).expect("序列化失败");
        let exp = &json["explanation"];
        assert_eq!(exp["query"], "Rust 编译失败 ModuleNotFoundError 怎么排查");
        assert!((exp["weights"]["fast"].as_f64().unwrap() - 1.0).abs() < 1e-5);
        assert!((exp["weights"]["deep"].as_f64().unwrap() - 1.8).abs() < 1e-5);
        assert!((exp["rrf_k"].as_f64().unwrap() - 60.0).abs() < 1e-5);
        assert_eq!(exp["fast_path_hits"], 6);
        assert_eq!(exp["deep_path_hits"], 4);
        assert_eq!(exp["total_candidates"], 3);
        let item = &exp["items"][0];
        assert_eq!(item["id"], "mem-001");
        assert_eq!(item["rank"], 1);
        assert_eq!(item["fast_rank"], 1);
        assert_eq!(item["deep_rank"], 3);
        assert_eq!(item["hit_paths"], serde_json::json!(["fast", "deep"]));
        assert!((item["fast_contrib"].as_f64().unwrap() - 0.0164).abs() < 1e-5);
        assert!((item["deep_contrib"].as_f64().unwrap() - 0.0147).abs() < 1e-5);
        // 顶层与解释块保持一致
        assert_eq!(json["total"], 3);
    }

    /// 阶段D：单路命中的解释条目 hit_paths 只含命中的通路。
    #[test]
    fn test_enrich_explanation_item_single_path() {
        let resp = EnrichResponse {
            memories: vec![],
            fast_path_hits: 1,
            deep_path_hits: 0,
            total: 1,
            trail: vec![],
            regression_evidence: HashMap::new(),
            filtered_count: 0,
            association_mode: "state_machine_navigation".to_string(),
            explanation: EnrichExplanation {
                query: "仅快速命中".to_string(),
                weights: ExplanationWeights {
                    fast: 1.0,
                    deep: 1.0,
                },
                rrf_k: 60.0,
                fast_path_hits: 1,
                deep_path_hits: 0,
                total_candidates: 1,
                items: vec![EnrichExplanationItem {
                    id: "mem-002".to_string(),
                    rank: 1,
                    score: 1.0,
                    fused_contrib: 0.0164,
                    fast_contrib: 0.0164,
                    deep_contrib: 0.0,
                    fast_rank: Some(1),
                    deep_rank: None,
                    hit_paths: vec!["fast"],
                }],
            },
            associated: vec![],
        };
        let json = serde_json::to_value(&resp).expect("序列化失败");
        let item = &json["explanation"]["items"][0];
        assert_eq!(item["deep_rank"], serde_json::Value::Null);
        assert_eq!(item["hit_paths"], serde_json::json!(["fast"]));
    }

    // ===== 4. 联想执行活动聚合纯函数测试（v0.9.6 首屏仪表盘契约） =====

    /// 构造一条 RetrievalExecuted 审计事件（metadata 为字符串映射，模拟真实落盘格式）
    fn retrieval_event(ts: u64, fast: &str, deep: &str, total: &str) -> AuditEvent {
        let mut md = std::collections::HashMap::new();
        md.insert("fast_hits".to_string(), fast.to_string());
        md.insert("deep_hits".to_string(), deep.to_string());
        md.insert("total_candidates".to_string(), total.to_string());
        AuditEvent {
            id: format!("audit-{ts}"),
            timestamp_ms: ts,
            event_type: AuditEventType::RetrievalExecuted,
            description: String::new(),
            reason: String::new(),
            affected_memory_ids: vec![],
            metadata: md,
            previous_hash: String::new(),
            event_hash: String::new(),
            hash_format: String::new(),
        }
    }

    #[test]
    fn test_association_activity_field_names_and_aggregation() {
        // 双通路命中 2 条 + 仅快速 1 条 + 仅深度 1 条，候选 10/20/30/40
        let e1 = retrieval_event(100, "5", "5", "10");
        let e2 = retrieval_event(300, "3", "3", "20");
        let e3 = retrieval_event(200, "4", "0", "30");
        let e4 = retrieval_event(50, "0", "2", "40");
        let refs: Vec<&AuditEvent> = vec![&e1, &e2, &e3, &e4];
        let agg = aggregate_association_activity(&refs);

        // 字段名契约（前端消费依赖这些 snake_case 键）
        for key in [
            "total_executions",
            "avg_candidates",
            "fast_only_count",
            "deep_only_count",
            "both_count",
            "last_execution_ms",
            "recent",
        ] {
            assert!(agg.get(key).is_some(), "缺少字段: {key}");
        }

        assert_eq!(agg["total_executions"], 4);
        assert_eq!(agg["both_count"], 2);
        assert_eq!(agg["fast_only_count"], 1);
        assert_eq!(agg["deep_only_count"], 1);
        // 平均候选 = (10+20+30+40)/4 = 25
        assert_eq!(agg["avg_candidates"], 25.0);
        // last_execution_ms 取最大时间戳（非数组末位），验证乱序鲁棒
        assert_eq!(agg["last_execution_ms"], 300);
        // recent 最多 10 条，此处 4 条全含
        assert_eq!(agg["recent"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn test_association_activity_empty_is_safe() {
        // 空事件集不得 panic，且各计数为 0、avg 为 0、last 为 null
        let agg = aggregate_association_activity(&[]);
        assert_eq!(agg["total_executions"], 0);
        assert_eq!(agg["avg_candidates"], 0.0);
        assert!(agg["last_execution_ms"].is_null());
        assert!(agg["recent"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_association_activity_ignores_unparseable_metadata() {
        // metadata 缺字段或非数字时按未命中处理，不污染统计
        let mut md = std::collections::HashMap::new();
        md.insert("fast_hits".to_string(), "abc".to_string()); // 不可解析
        let ev = AuditEvent {
            id: "x".into(),
            timestamp_ms: 10,
            event_type: AuditEventType::RetrievalExecuted,
            description: String::new(),
            reason: String::new(),
            affected_memory_ids: vec![],
            metadata: md,
            previous_hash: String::new(),
            event_hash: String::new(),
            hash_format: String::new(),
        };
        let refs: Vec<&AuditEvent> = vec![&ev];
        let agg = aggregate_association_activity(&refs);
        assert_eq!(agg["total_executions"], 1);
        assert_eq!(agg["both_count"], 0);
        assert_eq!(agg["fast_only_count"], 0);
        assert_eq!(agg["avg_candidates"], 0.0);
    }

    // ===== 5. 结晶历史时间线纯函数测试（v0.9.6 首屏时间线契约） =====

    /// 构造一条指定创建时间的记忆（Synthesis 或对照类型），覆盖默认随机时间
    fn timed_memory(id: &str, ts_ms: u64, content: &str, mtype: MemoryType) -> Memory {
        let mut m = Memory::new(
            content.to_string(),
            mtype,
            Some("test".to_string()),
            vec![],
            Importance::default(),
            None,
        );
        m.id = id.to_string();
        let ts = chrono::DateTime::from_timestamp_millis(ts_ms as i64).unwrap();
        m.created_at = ts;
        m.updated_at = ts;
        m.last_accessed = ts;
        m
    }

    #[test]
    fn test_synthesis_timeline_field_names_and_order() {
        let m1 = timed_memory("a", 100, "第一次结晶", MemoryType::Synthesis);
        let m2 = timed_memory("b", 300, "第二次结晶", MemoryType::Synthesis);
        // 非合成记忆必须被过滤，即使创建时间更晚
        let fact = timed_memory("c", 999, "普通记忆", MemoryType::Fact);
        // 故意乱序传入，验证防御性倒序排序
        let agg = build_synthesis_timeline(&[m1, fact, m2], 10);

        // 顶层字段契约（前端消费依赖这些 snake_case 键）
        for key in ["items", "total"] {
            assert!(agg.get(key).is_some(), "缺少字段: {key}");
        }
        assert_eq!(agg["total"], 2, "应只统计 Synthesis 记忆");
        let items = agg["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        // 倒序契约：b(300) 在 a(100) 前
        assert_eq!(items[0]["id"], "b");
        assert_eq!(items[1]["id"], "a");
        // 条目字段契约
        for key in [
            "id",
            "content",
            "memory_type",
            "project",
            "created_at_ms",
            "importance",
            // v0.9.6 P1-1：结晶成长链路字段
            "source_count",
            "confidence",
            "information_gain",
        ] {
            assert!(items[0].get(key).is_some(), "条目缺少字段: {key}");
        }
        assert_eq!(items[0]["source_count"], 0, "默认记忆无来源应记 0");
        assert_eq!(items[0]["memory_type"], "synthesis");
        assert_eq!(items[0]["created_at_ms"], 300);
    }

    #[test]
    fn test_synthesis_timeline_limit_and_empty() {
        // limit 截断契约
        let mems: Vec<Memory> = (0..20)
            .map(|i| {
                timed_memory(
                    &format!("m{i}"),
                    i as u64,
                    &format!("结晶{i}"),
                    MemoryType::Synthesis,
                )
            })
            .collect();
        let agg = build_synthesis_timeline(&mems, 5);
        assert_eq!(agg["total"], 20);
        assert_eq!(agg["items"].as_array().unwrap().len(), 5);
        // 空输入安全
        let empty = build_synthesis_timeline(&[], 10);
        assert_eq!(empty["total"], 0);
        assert!(empty["items"].as_array().unwrap().is_empty());
    }

    // ===== 6. 联想中心探索契约测试（v0.9.7 产品化） =====

    #[test]
    fn test_association_explore_request_defaults() {
        // 不传 depth/width 时使用默认：4 层、3 分支
        let req: AssociationExploreRequest =
            serde_json::from_value(serde_json::json!({ "query": "今晚吃什么" })).unwrap();
        assert_eq!(req.depth, 4);
        assert_eq!(req.width, 3);
        assert_eq!(req.query.as_deref(), Some("今晚吃什么"));

        // query 与 memory_id 均可作为起点
        let req2: AssociationExploreRequest = serde_json::from_value(
            serde_json::json!({ "memory_id": "mem-001", "depth": 2, "width": 1 }),
        )
        .unwrap();
        assert_eq!(req2.query, None);
        assert_eq!(req2.memory_id.as_deref(), Some("mem-001"));
        assert_eq!(req2.depth, 2);
        assert_eq!(req2.width, 1);

        // depth 越界时在 handler 入口被 clamp 到 1..=4、width clamp 到 1..=3
        let req3: AssociationExploreRequest =
            serde_json::from_value(serde_json::json!({ "query": "x", "depth": 99, "width": 99 }))
                .unwrap();
        assert_eq!(req3.depth.clamp(1, 4), 4);
        assert_eq!(req3.width.clamp(1, 3), 3);
    }

    #[test]
    fn test_association_explore_response_field_names() {
        use crate::engine::memory_state_machine::AssociationStep;
        let resp = AssociationExploreResponse {
            root: Some("mem-root".to_string()),
            depth: 2,
            width: 2,
            nodes: vec![
                ExploreNode {
                    id: "mem-root".into(),
                    content: "起点记忆".into(),
                    depth: 0,
                    score: 1.0,
                    source: "root".into(),
                    evidence: None,
                    relation: None,
                    why: None,
                },
                ExploreNode {
                    id: "mem-child".into(),
                    content: "联想记忆".into(),
                    depth: 1,
                    score: 0.7,
                    source: "expanded".into(),
                    evidence: Some("联想桥强关联".into()),
                    relation: None,
                    why: None,
                },
            ],
            edges: vec![ExploreEdge {
                from: "mem-root".into(),
                to: "mem-child".into(),
                score: 0.7,
                evidence: Some("联想桥强关联".into()),
                relation: None,
            }],
            trail: vec![AssociationStep {
                from_id: Some("mem-root".into()),
                to_id: "mem-child".into(),
                score: 0.7,
                depth: 1,
            }],
            total_expanded: 2,
            interrupted: false,
            // 存在通过校验的扩展节点 → 非弱匹配
            weak_match: false,
            // 测试用例：词面门禁通过，语义旁路未触发
            semantic_bypass: "unused".to_string(),
        };
        let json = serde_json::to_value(&resp).expect("序列化失败");
        assert_eq!(json["root"], "mem-root");
        assert_eq!(json["depth"], 2);
        assert_eq!(json["width"], 2);
        assert_eq!(json["interrupted"], false);
        // v0.9.7：weak_match 契约——有扩展节点时必须为 false
        assert_eq!(json["weak_match"], false);
        // P8.2c：语义旁路活性字段可序列化
        assert_eq!(json["semantic_bypass"], "unused");
        for key in ["nodes", "edges", "trail"] {
            assert!(json[key].is_array(), "缺少数组字段: {key}");
        }
        let node = &json["nodes"][1];
        for key in ["id", "content", "depth", "score", "source", "evidence"] {
            assert!(node.get(key).is_some(), "节点缺少字段: {key}");
        }
        assert_eq!(node["source"], "expanded");
        assert_eq!(node["evidence"], "联想桥强关联");
        let edge = &json["edges"][0];
        for key in ["from", "to", "score", "evidence"] {
            assert!(edge.get(key).is_some(), "边缺少字段: {key}");
        }
    }

    /// 探索集成测试：BFS 有界性、visited 去重、节点深度合法。
    #[test]
    fn test_run_association_explore_bfs_bounds() {
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_bfs_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        let mut store = MemoryStore::new(JsonPersistence::new(&dir_str).unwrap());

        // 写入一组生活联想链相关记忆：食物、餐馆、和谁吃、在哪吃
        for content in [
            "今晚吃什么好呢",
            "潮汕牛肉火锅和粤式烧腊都是不错的选项",
            "楼下新开的粤菜餐馆珠江新城店环境很好",
            "和小王一起去吃火锅很开心",
            "中山路的老字号肠粉店排队很久",
            "周末和老婆去郊外野餐带了三明治",
            "完全无关的记忆：绿萝每周换一次盆土",
        ] {
            let memory = Memory::new(
                content.to_string(),
                MemoryType::Conversation,
                None,
                vec![],
                Importance::new(5),
                None,
            );
            store.remember(memory).unwrap();
        }

        let cancel = AtomicBool::new(false);
        let resp = run_association_explore(
            &mut store,
            Some("今晚吃什么"),
            None,
            3, // 最多 3 层
            2, // 每层最多 2 个方向
            &cancel,
            None,
        );

        assert!(resp.root.is_some(), "应找到起点记忆");
        assert!(!resp.nodes.is_empty(), "应至少返回起点节点");
        // 深度合法性：节点 depth 不超过 3
        for node in &resp.nodes {
            assert!(node.depth <= 3, "节点深度越界: {}", node.depth);
            if node.depth == 0 {
                assert_eq!(node.source, "root", "起点节点来源应为 root");
            } else {
                assert_eq!(node.source, "expanded", "发散节点来源应为 expanded");
            }
        }
        // visited 去重：节点 id 不得重复
        let mut ids = std::collections::HashSet::new();
        for node in &resp.nodes {
            assert!(
                ids.insert(node.id.clone()),
                "探索出现重复节点 id: {}",
                node.id
            );
        }
        // 边端点必须都出现在节点集合中
        for edge in &resp.edges {
            assert!(
                ids.contains(&edge.from),
                "边起点不在节点集合: {}",
                edge.from
            );
            assert!(ids.contains(&edge.to), "边终点不在节点集合: {}", edge.to);
        }
        // 有界性：每层分支 <= width（2），总节点数 <= 1 + 2 + 4 + 8 = 15
        assert!(
            resp.nodes.len() <= 15,
            "探索节点数量超过有界上限: {}",
            resp.nodes.len()
        );
        assert!(!resp.interrupted, "小规模探索不应中断");
    }

    /// 探索集成测试：从记忆 ID 出发且目标不存在时安全返回空树。
    #[test]
    fn test_run_association_explore_missing_memory_id_safe() {
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_missing_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        let mut store = MemoryStore::new(JsonPersistence::new(&dir_str).unwrap());
        let cancel = AtomicBool::new(false);
        let resp = run_association_explore(
            &mut store,
            None,
            Some("mem-does-not-exist"),
            2,
            1,
            &cancel,
            None,
        );
        assert!(resp.root.is_none(), "不存在的记忆 ID 不应产生根节点");
        assert!(resp.nodes.is_empty());
        assert!(resp.edges.is_empty());
    }

    /// v0.9.7 精确度修复：多跳扩散的回归校验必须锚定起点主题。
    /// 深层记忆与"漂移陷阱"（与 hop1 共享词、但与起点主题无关）不得出现。
    #[test]
    fn test_run_association_explore_anchored_regression_no_drift() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_anchor_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        let mut store = MemoryStore::new(JsonPersistence::new(&dir_str).unwrap());

        // 食物主题链 + 漂移陷阱：
        // 陷阱与 hop1 记忆共享"浇水"，但与起点"今晚吃番茄炒蛋"主题无关。
        // 若回归校验随父内容漂移（旧行为），深层会把陷阱拉进结果。
        for content in [
            "今晚吃番茄炒蛋，简单好吃",
            "番茄苗要每日浇水补光才能长好",
            "浇水的计算机程序实现细节与源码",
        ] {
            let memory = Memory::new(
                content.to_string(),
                MemoryType::Conversation,
                None,
                vec![],
                Importance::new(5),
                None,
            );
            store.remember(memory).unwrap();
        }

        let cancel = AtomicBool::new(false);
        let resp = run_association_explore(
            &mut store,
            Some("今晚吃什么"),
            None,
            3, // 允许 3 层，确保会走到深层扩散
            2,
            &cancel,
            None,
        );

        assert!(resp.root.is_some(), "应找到起点记忆");
        // 锚定校验：任何层级的节点都不得包含漂移陷阱内容
        for node in &resp.nodes {
            assert!(
                !node.content.contains("计算机"),
                "深层联想漂移：无关记忆混入联想结果: {}",
                node.content
            );
        }
        // hop1 的主题内记忆（番茄苗）应被保留——锚定只杀漂移，不杀主题
        assert!(
            resp.nodes
                .iter()
                .any(|node| node.content.contains("番茄苗")),
            "锚定校验不应误杀主题内记忆"
        );
        assert!(!resp.weak_match, "存在主题内扩展节点时不应标记弱匹配");
    }

    /// v0.9.7 精确度修复：根节点主导性门禁。
    /// 仅凭单个泛指词（"什么"）重叠的代码记忆不得上位当起点，
    /// 实质共鸣（≥2 个 token）的主题记忆必须胜出。
    #[test]
    fn test_run_association_explore_root_dominance_gate() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_root_gate_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        let mut store = MemoryStore::new(JsonPersistence::new(&dir_str).unwrap());

        // 干扰记忆：与"今晚吃什么"只共享泛指 bigram「什么」1 个 token；
        // 主题记忆：命中「今晚」「晚吃」2 个 token，实质共鸣；
        // 扩展记忆：与起点"今晚吃火锅还是点外卖"共鸣（火锅），供发散层使用。
        // 旧行为（无条件取 top1）在分数接近时会把代码记忆当起点。
        for content in [
            "这个报错是什么意思",
            "今晚吃火锅还是点外卖",
            "周末和朋友也想吃火锅，虾滑必点",
        ] {
            let memory = Memory::new(
                content.to_string(),
                MemoryType::Conversation,
                None,
                vec![],
                Importance::new(5),
                None,
            );
            store.remember(memory).unwrap();
        }

        let cancel = AtomicBool::new(false);
        let resp =
            run_association_explore(&mut store, Some("今晚吃什么"), None, 2, 2, &cancel, None);

        assert!(resp.root.is_some(), "应存在实质共鸣的起点记忆");
        let root_node = resp
            .nodes
            .iter()
            .find(|node| node.depth == 0)
            .expect("起点节点必须存在");
        assert!(
            root_node.content.contains("火锅"),
            "起点应为食物主题记忆，而不是泛指词重叠的无关记忆: {}",
            root_node.content
        );
        // 干扰记忆不得以任何身份（起点或发散）出现在联想结果里
        assert!(
            resp.nodes.iter().all(|node| !node.content.contains("报错")),
            "泛指词重叠的无关记忆混入联想结果"
        );
        assert!(!resp.weak_match, "存在主题记忆与扩展节点时不应标记弱匹配");
    }

    /// 根节点门禁的诚实空态：记忆库里只有泛指词重叠的无关记忆时，
    /// 不硬凑起点，直接标记 weak_match。
    #[test]
    fn test_run_association_explore_root_gate_weak_match() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_root_weak_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        // 统计模式确定性构造：语义旁路在本用例中必须保持失效，
        // 泛指词重叠的无关记忆无论是否装有 ML 模型都不应上位
        let mut store = new_statistical_store(&dir_str);

        // 只有与查询共享单个泛指词的记忆——旧行为会把它硬凑成起点
        let memory = Memory::new(
            "这个报错是什么意思".to_string(),
            MemoryType::Conversation,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        store.remember(memory).unwrap();

        let cancel = AtomicBool::new(false);
        let resp =
            run_association_explore(&mut store, Some("今晚吃什么"), None, 2, 2, &cancel, None);

        assert!(resp.root.is_none(), "泛指词重叠的无关记忆不应成为起点");
        assert!(resp.nodes.is_empty(), "弱匹配时不应产生任何节点");
        assert!(resp.weak_match, "无实质共鸣候选时应标记弱匹配");
    }

    /// P8.2c：语义旁路活性观测——统计模式（默认构建）下，词面无通过者的
    /// 查询会触发旁路但编码器无向量 → semantic_bypass="unavailable"（旁路实际
    /// 未参与）；词面通过者 → "unused"（旁路未触发）。供评测自检，避免把
    /// "旁路缺席"误读为"旁路无效"（P7.4 教训的代码防呆）。
    #[test]
    fn test_assoc_explore_semantic_bypass_activity_probe() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_bypass_probe_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        let mut store = MemoryStore::new(JsonPersistence::new(&dir_str).unwrap());

        // 用例1：词面无通过候选的查询 → 触发旁路，但统计模式无向量 → unavailable
        let _ = store.remember(Memory::new(
            "周末想去看红叶，香山那边全红了".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(5),
            None,
        ));
        let cancel = AtomicBool::new(false);
        let resp1 = run_association_explore(
            &mut store,
            Some("量子物理是什么"),
            None,
            2,
            2,
            &cancel,
            None,
        );
        assert_eq!(
            resp1.semantic_bypass, "unavailable",
            "统计模式下旁路触发但编码器无向量应标记 unavailable（weak={}）",
            resp1.weak_match
        );
        assert!(resp1.weak_match, "无关联查询应诚实空态");

        // 用例2：词面通过候选的查询 → 旁路未触发 → unused
        let _ = store.remember(Memory::new(
            "Rust 借用检查器报 E0597 时先检查所有权转移位置".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(5),
            None,
        ));
        let resp2 = run_association_explore(
            &mut store,
            Some("Rust 借用检查器"),
            None,
            2,
            2,
            &cancel,
            None,
        );
        assert_eq!(
            resp2.semantic_bypass,
            "unused",
            "词面通过者不应触发旁路: {:?}",
            resp2.nodes.iter().map(|n| &n.content).collect::<Vec<_>>()
        );
        assert!(!resp2.weak_match, "词面命中的代码查询不应空态");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 泛指 bigram 组合不得虚假共鸣（v0.9.7 CDP 回归发现的真实缺陷）：
    /// "量子物理是什么"与含测试文本"是什么"的代码 chunk 共享「是什」
    /// 「什么」两个泛指 bigram——修复前刚好凑满 min_required=2，代码
    /// chunk 抢走起点并让整条联想链陷入代码邻域（depth=4 扩散超时 503）。
    /// 修复后：泛指 bigram 不计入门禁，实义 token（量子/物理）零重叠
    /// → 诚实空态。对照查询验证实义 bigram 不被误伤。
    #[test]
    fn test_run_association_explore_generic_bigram_gate() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_generic_gate_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        // 统计模式（确定性）：不依赖 ML 是否可用
        let mut store = new_statistical_store(&dir_str);

        // 干扰记忆：含"是什么"字样的代码记忆（模拟代码 chunk 里的测试文本）
        let noisy = Memory::new(
            "来源：src/lib.rs:1-10；主题：rs。项目的数据库和缓存架构是什么".to_string(),
            MemoryType::CodeContext,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        store.remember(noisy).unwrap();

        // 查询 1：库里没有量子物理相关内容 → 不硬凑代码起点，诚实空态
        let cancel = AtomicBool::new(false);
        let resp = run_association_explore(
            &mut store,
            Some("量子物理是什么"),
            None,
            2,
            2,
            &cancel,
            None,
        );
        assert!(
            resp.root.is_none() && resp.weak_match,
            "泛指 bigram 组合不应让代码记忆虚假上位: root={:?}",
            resp.root
        );

        // 查询 2（对照）：实义 bigram（数据库/据库/缓存）命中时正常上位，
        // 证明过滤只剔除泛指组合，不伤害实义共鸣。
        let resp2 = run_association_explore(
            &mut store,
            Some("数据库和缓存架构"),
            None,
            2,
            2,
            &cancel,
            None,
        );
        assert!(
            resp2.root.is_some() && !resp2.weak_match,
            "实义 token 重叠的代码记忆应正常成为起点"
        );
    }

    /// 构造统计模式（确定性）记忆库：ML 模型存在与否不影响测试预期。
    /// 本机装有 bge 模型时，`MemoryStore::new` 会加载 ML 编码器并激活
    /// 语义旁路；需要验证"纯词面门禁"的测试必须显式固定为统计模式。
    fn new_statistical_store(dir_str: &str) -> MemoryStore<JsonPersistence> {
        #[cfg(feature = "ml")]
        {
            MemoryStore::new_with_encoder(
                JsonPersistence::new(dir_str).unwrap(),
                crate::engine::luoshu_encoder_ml::HybridLuoShuEncoder::new_statistical(),
            )
        }
        #[cfg(not(feature = "ml"))]
        {
            // 非 ml 构建本身就是统计编码器
            MemoryStore::new(JsonPersistence::new(dir_str).unwrap())
        }
    }

    /// 构造 ML 能力记忆库（镜像 bin/server.rs 的启动链路：
    /// LuoShuMlEncoder::load → new_with_ml）。模型不可用时返回 None，
    /// 调用方跳过用例——语义旁路只能在真实 ML 环境验证。
    #[cfg(feature = "ml")]
    fn new_ml_capable_store(dir_str: &str) -> Option<MemoryStore<JsonPersistence>> {
        let ml = crate::engine::luoshu_encoder_ml::LuoShuMlEncoder::load().ok()?;
        Some(MemoryStore::new_with_encoder(
            JsonPersistence::new(dir_str).unwrap(),
            crate::engine::luoshu_encoder_ml::HybridLuoShuEncoder::new_with_ml(ml),
        ))
    }

    /// 语义旁路阈值标定（仅 ml 构建且模型可用时执行）：
    /// 断言"语义强相关对"的余弦显著高于"泛指结构相似对"，
    /// 保证 ASSOCIATION_ROOT_MIN_SEMANTIC_SIM 存在可分离的取值区间，
    /// 并打印具体数值供阈值调整参考。
    #[test]
    #[cfg(feature = "ml")]
    fn test_semantic_similarity_threshold_calibration() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_sem_calib_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        // 显式加载 ML 编码器（默认构造恒为统计模式，不会加载模型）
        let Some(store) = new_ml_capable_store(&dir_str) else {
            eprintln!("[标定] ML 编码器不可用，跳过语义阈值标定");
            return;
        };

        let anniversary = Memory::new(
            "结婚纪念日是 10 月 20 号，每年都要一起吃顿好的。".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(9),
            None,
        );
        let decoy = Memory::new(
            "这个报错是什么意思".to_string(),
            MemoryType::Conversation,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        let hiking = Memory::new(
            "周末去白云山徒步，山顶风景很好。".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(6),
            None,
        );

        let pairs = [
            ("我以前记过什么重要日子？", &anniversary),
            ("今晚吃什么", &decoy),
            ("周末去哪儿玩", &hiking),
        ];
        let sims = pairs
            .iter()
            .map(|(q, m)| store.semantic_similarities(q, &[m])[0].unwrap_or(f32::NAN));
        let (sim_days, sim_decoy, sim_hiking) = {
            let v: Vec<f32> = sims.collect();
            (v[0], v[1], v[2])
        };
        eprintln!(
            "[标定] 重要日子↔结婚纪念日 = {sim_days:.4}；今晚吃什么↔报错 = {sim_decoy:.4}；周末去哪儿玩↔白云山 = {sim_hiking:.4}；当前阈值 = {ASSOCIATION_ROOT_MIN_SEMANTIC_SIM}"
        );
        // 相关对必须与泛指结构对可分离，阈值才有存在意义
        assert!(
            sim_days > sim_decoy,
            "语义强相关对的余弦必须高于泛指结构对：{sim_days} vs {sim_decoy}"
        );
        assert!(
            sim_hiking > sim_decoy,
            "语义强相关对的余弦必须高于泛指结构对：{sim_hiking} vs {sim_decoy}"
        );
    }

    /// 语义旁路行为：查询"重要日子"与"结婚纪念日"记忆词面零重叠，
    /// 纯词面门禁下必然弱匹配；ML 编码器可用时语义旁路应挽救该起点。
    #[test]
    #[cfg(feature = "ml")]
    fn test_run_association_explore_semantic_bypass_life_days() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_sem_bypass_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        // 显式加载 ML 编码器（默认构造恒为统计模式，不会加载模型）
        let Some(mut store) = new_ml_capable_store(&dir_str) else {
            eprintln!("[语义旁路] ML 编码器不可用，本用例仅在 ml 环境执行");
            return;
        };

        let memory = Memory::new(
            "结婚纪念日是 10 月 20 号，每年都要一起吃顿好的。".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(9),
            None,
        );
        store.remember(memory).unwrap();

        let cancel = AtomicBool::new(false);
        let resp = run_association_explore(
            &mut store,
            Some("我以前记过什么重要日子？"),
            None,
            2,
            2,
            &cancel,
            None,
        );

        assert!(!resp.weak_match, "语义强相关的纪念日记忆不应被误判为弱匹配");
        let root_node = resp
            .nodes
            .iter()
            .find(|node| node.depth == 0)
            .expect("起点节点必须存在");
        assert!(
            root_node.content.contains("结婚纪念日"),
            "语义旁路应召回结婚纪念日记忆: {}",
            root_node.content
        );
    }

    // ============================================================
    // P8.2h：离线 bge 批量编码 —— root 语义旁路救回率实测（H1-H4）
    //
    // 方案 A：直接调用真实 run_association_explore（零镜像口径）。
    // 语料 / 查询 / 判定口径逐字对齐 daoti/_fair_corpus.py 与
    // daoti/_assoc_accuracy_eval.py；A 臂（统计编码器）复现 P7.4 基线，
    // B 臂（ML 编码器 + 真实 bge 权重）实测语义旁路救回。
    //
    // 运行（需本地 bge 权重，如 ~/.loong-recall/models/BAAI--bge-base-zh）：
    //   $env:LRC_LUOSHU_MODEL_ID='BAAI/bge-base-zh'
    //   cargo test --offline --features ml --lib test_assoc_p82h -- --nocapture
    // ============================================================

    /// 公平语料·联想链（6 链 × 16 条，逐字对齐 _fair_corpus.py::CHAINS）
    #[cfg(feature = "ml")]
    const P82H_CHAINS: &[(&str, &[(u8, &str)])] = &[
        (
            "dinner",
            &[
                (1, "今晚想吃火锅，上次念叨的那家海底捞一直还没去"),
                (1, "冰箱里有半盒鸡蛋两个西红柿，实在不行就下碗面"),
                (2, "室友小周不吃香菜，点锅底要提前跟她说"),
                (2, "楼下新开那家日料周三前会员打八折"),
                (2, "外卖起送要三十，满四十五才减八块"),
                (2, "这个月外食预算只剩三百出头"),
                (3, "上回吃太辣第二天胃不舒服，买了盒达喜"),
                (3, "商场停车两小时后每小时六块，饭点地库排队"),
                (3, "她收藏了一家深夜食堂风格的居酒屋在老仓库那边"),
                (3, "周末在家包了次饺子，饺子皮是菜场买的"),
                (4, "老仓库那家店老板说翻台慢建议工作日来"),
                (4, "体检报告说要少油少盐，外卖备注清淡"),
                (4, "家里那口炒锅涂层花了，炒菜总粘底"),
                (4, "她生日那顿说好吃的话下次带爸妈来"),
                (5, "从小家里晚饭都固定一荤一汤，习惯了"),
                (5, "吃完晚饭轮流水池，周五轮到她洗"),
            ],
        ),
        (
            "hike",
            &[
                (1, "周末想去香山看红叶，听说这周全红了"),
                (1, "约了老陈一家周六上午西山绿道徒步"),
                (2, "预报周六多云十八度，傍晚起风"),
                (2, "绿道后山出口有条野路通到停车场"),
                (2, "老陈家娃五岁，走快了要抱"),
                (2, "徒步群里说东门外拼车AA每人十五"),
                (3, "去年爬香山东门堵了一个半小时没上去"),
                (3, "山上小卖部矿泉水八块，泡面十五"),
                (3, "她新买的登山杖是碳纤维的要显摆"),
                (3, "看红叶最佳时段是十点半前和四点半后"),
                (4, "野路出来那段碎石坡容易打滑"),
                (4, "山顶信号差，约定失联就在观景台等"),
                (4, "老陈上次扭过脚腕，护踝让他戴上"),
                (4, "回程顺路能到那个大集，收摊早"),
                (5, "家里规矩是户外活动前夜必须早睡"),
                (5, "她背包里永远有一块压缩饼干应急"),
            ],
        ),
        (
            "wedding",
            &[
                (1, "下周六小赵结婚，请柬写的是香格里宴会厅"),
                (1, "随份子群里接龙统一六百"),
                (2, "婚礼要求正装出席，男士深色西装"),
                (2, "午宴一点半入场，合影在仪式后"),
                (2, "部门五个人拼一辆七座商务车"),
                (2, "她让我帮忙带个礼物盒包装"),
                (3, "上次婚宴那家酒店海鲜不新鲜有人拉肚子"),
                (3, "宴会厅在城东，早高峰地铁更稳"),
                (3, "司仪要随机采访同事讲新人故事"),
                (3, "小赵说过敬酒环节他替新娘挡酒"),
                (4, "婚礼车库限高一点八米，底盘低别下"),
                (4, "份子钱让财务统一转，微信别单独发"),
                (4, "上次酒桌游戏输了被起哄唱了一首"),
                (4, "她西装是去年买的裤脚短了一截"),
                (5, "家里长辈要求红白喜事必须到场"),
                (5, "每次参加婚礼她都要记人家的礼服穿法"),
            ],
        ),
        (
            "cat",
            &[
                (1, "猫这两天不吃东西老趴着，得带去看医生"),
                (1, "家附近评价最好的是爱康宠物医院"),
                (2, "它体检疫苗本和驱虫记录在一个袋里"),
                (2, "航空箱在阳台储物柜，钥匙挂门后"),
                (2, "周五它吐了一次毛球样的东西"),
                (2, "医生说换粮要七天渐进掺着换"),
                (3, "上回打疫苗它应激，回家躲沙发底一天"),
                (3, "宠物医院节假日加收三十块急诊费"),
                (3, "它以前最爱钻快递纸箱，见箱就进"),
                (3, "猫砂快见底了，囤的那袋是豆腐砂"),
                (4, "医生说过舔毛过猛可能是皮肤过敏信号"),
                (4, "医院用的猫条零食是某牌子，家里没有了"),
                (4, "她妈对猫毛过敏，过年不能带它回老家"),
                (4, "上次输液留置针挂了三天，伊丽莎白圈要戴"),
                (5, "从小家里养猫，看病先翻旧病历的习惯随家里"),
                (5, "它一听到开罐头声就从任何角落出现"),
            ],
        ),
        (
            "car",
            &[
                (1, "这辆车的年检到期了，得赶这个月办完"),
                (1, "朋友推荐一家检测站能预约不用排队"),
                (2, "交强险和车船税一起续，保单在抽屉"),
                (2, "行驶证副本上写着注册日期是月底"),
                (2, "上次有个未处理违停要先缴掉"),
                (2, "尾气复检要空滤干净，四万八该换的了"),
                (3, "检测站门口那条路单行，绕辅路进"),
                (3, "验车师傅说灯光改装过要还原才能过线"),
                (3, "她练车那会儿最怕灯光检测那一项"),
                (3, "去年检完贴的合格标在挡风玻璃右上角"),
                (4, "检测排队时引擎别熄火，怠速也要测"),
                (4, "车内三角警示牌过期了，顺手换新的"),
                (4, "保险过户后第一年就少了两次免费道路救援"),
                (4, "那家老检测站后来被查到出具虚假报告关了"),
                (5, "家里买车惯例是提前两个月开始张罗年检"),
                (5, "每次办手续他都要把单据按日期排好夹起来"),
            ],
        ),
        (
            "exam",
            &[
                (1, "孩子下周三期末考，数学应用题是他的坎"),
                (1, "老师说考前把错题本重新过一遍就行"),
                (2, "她每晚八点陪读一小时，雷打不动"),
                (2, "语文古诗默写还差两首长的没背熟"),
                (2, "考试要带的两B铅笔和尺子放笔袋了"),
                (2, "学校通知周四下午提前放学"),
                (3, "上次他紧张把计算器忘家里，半路回去取"),
                (3, "他同桌考前发烧没考好，全班都知道了"),
                (3, "家里那盏护眼台灯是买给他写作业的"),
                (3, "班主任群里说这次成绩家长不排名"),
                (4, "她一考试前就失眠，安眠药是老毛病了"),
                (4, "爷爷那辈人信奉考前别吃太饱，七分饱"),
                (4, "他房间朝西，下午晒得书桌滚烫"),
                (4, "去年暑假班老师留了套压轴题卷一直没做"),
                (5, "家里惯例考完试周六全家去放风放松"),
                (5, "她妈当年考前要亲手包一顿馄饨图吉利"),
            ],
        ),
    ];

    /// 公平语料·干扰池（8 技术 + 2 生活噪声，逐字对齐 _fair_corpus.py::DISTRACTORS）
    #[cfg(feature = "ml")]
    const P82H_DISTRACTORS: &[(u8, &str)] = &[
        (9, "Rust 借用检查器报 E0597 时先检查所有权转移位置"),
        (9, "数据库慢查询先跑 EXPLAIN 看是否走了索引"),
        (9, "CI 流水线 YAML 锚点可以复用公共步骤定义"),
        (9, "前端大列表渲染要用虚拟滚动避免 DOM 节点过多"),
        (9, "Docker 镜像分层缓存能显著缩短构建时间"),
        (9, "Kubernetes 就绪探针配置不当会造成滚动发布卡死"),
        (9, "Nginx 反向代理超时默认六十秒要显式配置"),
        (9, "SQL 注入防护必须参数化查询不能拼字符串"),
        (9, "阳台绿萝养了两年长得特别疯要修剪"),
        (9, "双十一囤的洗衣液还有两大桶没用完"),
    ];

    /// 30 个查询（逐字对齐 _fair_corpus.py::QUERIES）
    #[cfg(feature = "ml")]
    const P82H_QUERIES: &[(&str, &str)] = &[
        ("dinner", "今晚吃什么好呢"),
        ("dinner", "晚饭怎么解决"),
        ("dinner", "肚子饿了晚上吃点啥"),
        ("dinner", "今晚的饭有谱了吗"),
        ("dinner", "晚饭后去哪吃比较好"),
        ("hike", "周末爬山安排得怎么样了"),
        ("hike", "这周六出去走走怎么计划"),
        ("hike", "想去看红叶什么时候去合适"),
        ("hike", "周末徒步那次都约了谁怎么过去"),
        ("hike", "爬山那天都注意点啥"),
        ("wedding", "下周六小赵的婚礼怎么办"),
        ("wedding", "同事结婚我要准备些什么"),
        ("wedding", "婚礼那天流程是啥样的"),
        ("wedding", "参加婚礼有什么讲究来着"),
        ("wedding", "份子钱和车那事都齐了吗"),
        ("cat", "猫这两天不对劲怎么办"),
        ("cat", "要带猫去看医生得准备啥"),
        ("cat", "猫咪不吃饭这事后来怎么样"),
        ("cat", "宠物医院那趟有什么要注意的"),
        ("cat", "猫生病家里还缺什么物资"),
        ("car", "车的年检这个月能办完吗"),
        ("car", "去检测站要带什么东西"),
        ("car", "年检前车辆还要弄哪些"),
        ("car", "验车那次有什么坑"),
        ("car", "保险和年检怎么一起搞"),
        ("exam", "孩子下周三期末考试怎么备战"),
        ("exam", "期末考前家里要配合什么"),
        ("exam", "考试那几天有什么要准备的"),
        ("exam", "孩子数学应用题那关怎么过"),
        ("exam", "考前孩子状态不对怎么办"),
    ];

    /// 无关联查询（对齐 _assoc_accuracy_eval.py::UNRELATED_QUERIES）
    /// P8.2l 扩容复测：2 → 22 条，覆盖天体物理/历史/气象/医疗/体育/数学/化学/
    /// 法律/生物/艺术/影视/环保/宗教/棋类/语言文字/地质/音乐/考古/贸易等跨域语义；
    /// 逐条避开公平语料的 6 大生活链（dinner/hike/wedding/cat/car/exam）与
    /// 10 条干扰语义；保留原有 2 条以维持与 P8.2h–P8.2k 的可比性。
    #[cfg(feature = "ml")]
    const P82H_UNRELATED: &[&str] = &[
        "量子物理是什么",
        "今天股市行情怎么样",
        "黑洞是怎么形成的",
        "明朝灭亡的原因是什么",
        "台风路径是怎么预测的",
        "心脏搭桥手术恢复要多久",
        "马拉松训练计划怎么安排",
        "三角函数公式怎么推导",
        "化学反应放热如何判断",
        "合同法里的违约责任怎么算",
        "光合作用的过程是怎样的",
        "水墨画的皴法有哪些讲究",
        "电影分级制度是怎么规定的",
        "大气污染治理有哪些手段",
        "佛教禅宗的核心思想是什么",
        "围棋的胜负怎么判定",
        "糖尿病早期症状有哪些",
        "汉字简化是什么时候开始的",
        "地震预警系统是怎么工作的",
        "交响乐团的编制是怎样的",
        "考古地层学的基本原理",
        "跨境电商的关税怎么计算",
    ];

    /// P7.4 A 臂（统计模式）实测的 16 个 MISS 查询（对齐 assoc_accuracy_results.json）
    #[cfg(feature = "ml")]
    const P82H_DOC_MISS: &[&str] = &[
        "今晚吃什么好呢",
        "晚饭怎么解决",
        "肚子饿了晚上吃点啥",
        "今晚的饭有谱了吗",
        "晚饭后去哪吃比较好",
        "周末爬山安排得怎么样了",
        "这周六出去走走怎么计划",
        "爬山那天都注意点啥",
        "同事结婚我要准备些什么",
        "婚礼那天流程是啥样的",
        "猫咪不吃饭这事后来怎么样",
        "猫生病家里还缺什么物资",
        "年检前车辆还要弄哪些",
        "验车那次有什么坑",
        "考试那几天有什么要准备的",
        "考前孩子状态不对怎么办",
    ];

    /// 全语料（6 链 × 16 + 10 干扰 = 106 条，对齐 _fair_corpus.py::all_memories）
    #[cfg(feature = "ml")]
    fn p82h_all_memories() -> Vec<(&'static str, u8, &'static str)> {
        let mut out = Vec::new();
        for (chain, items) in P82H_CHAINS {
            for (hop, content) in items.iter() {
                out.push((*chain, *hop, *content));
            }
        }
        for (hop, content) in P82H_DISTRACTORS {
            out.push(("none", *hop, *content));
        }
        out
    }

    /// 内容 → (链, hop)，逐字对齐 _assoc_accuracy_eval.py::classify
    /// （完全相等或前 12 个字符前缀命中；未命中归入 "none"/9）
    #[cfg(feature = "ml")]
    fn p82h_classify(content: &str) -> (&'static str, u8) {
        for (chain, items) in P82H_CHAINS {
            for (hop, text) in items.iter() {
                let prefix: String = text.chars().take(12).collect();
                if content == *text || (!prefix.is_empty() && content.starts_with(&prefix)) {
                    return (*chain, *hop);
                }
            }
        }
        ("none", 9)
    }

    /// 单臂评测记录
    #[cfg(feature = "ml")]
    struct P82hRecord {
        query: &'static str,
        root_chain: &'static str,
        root_correct: bool,
        weak_match: bool,
        semantic_bypass: String,
    }

    /// 写入公平语料（对齐 _assoc_accuracy_eval.py::seed 的字段口径）
    #[cfg(feature = "ml")]
    fn p82h_seed(store: &mut MemoryStore<JsonPersistence>) {
        use crate::memory_types::{Importance, Memory, MemoryType};
        for &(chain, hop, content) in p82h_all_memories().iter() {
            let memory = Memory::new(
                content.to_string(),
                MemoryType::Fact,
                Some("p7-4-association-accuracy".to_string()),
                vec![format!("chain:{chain}"), format!("hop:{hop}")],
                Importance::new(if hop <= 2 { 7 } else { 5 }),
                None,
            );
            store.remember(memory).expect("公平语料写入失败");
        }
    }

    /// 单臂跑完全部 30 查询（depth=4 / width=3，对齐评测脚本常量）
    #[cfg(feature = "ml")]
    fn p82h_run_all(store: &mut MemoryStore<JsonPersistence>) -> Vec<P82hRecord> {
        use std::sync::atomic::AtomicBool;
        let mut records = Vec::new();
        for &(gold, query) in P82H_QUERIES {
            let cancel = AtomicBool::new(false);
            let resp = run_association_explore(store, Some(query), None, 4, 3, &cancel, None);
            let root_chain = resp
                .nodes
                .iter()
                .find(|node| node.depth == 0)
                .map(|node| p82h_classify(&node.content).0)
                .unwrap_or("none");
            records.push(P82hRecord {
                query,
                root_chain,
                root_correct: root_chain == gold,
                weak_match: resp.weak_match,
                semantic_bypass: resp.semantic_bypass.clone(),
            });
        }
        records
    }

    /// 无关联查询的诚实空态探测（返回 (查询, 是否诚实空态, 旁路状态, 被拉入的 root 内容)）
    #[cfg(feature = "ml")]
    fn p82h_probe_unrelated(
        store: &mut MemoryStore<JsonPersistence>,
    ) -> Vec<(&'static str, bool, String, String)> {
        use std::sync::atomic::AtomicBool;
        let mut out = Vec::new();
        for query in P82H_UNRELATED {
            let cancel = AtomicBool::new(false);
            let resp = run_association_explore(store, Some(*query), None, 4, 3, &cancel, None);
            // 误召回根因：旁路放行后 depth==0 的起点内容（诚实空态时应为空）
            let root_content = resp
                .nodes
                .iter()
                .find(|node| node.depth == 0)
                .map(|node| node.content.clone())
                .unwrap_or_default();
            out.push((
                *query,
                resp.weak_match && resp.nodes.is_empty(),
                resp.semantic_bypass.clone(),
                root_content,
            ));
        }
        out
    }

    /// P8.2h 根因探针：对给定查询，打印全库余弦 Top-3（判定 H2 FAIL 是
    /// "阈值标定"还是"机制失效"——误召回余弦若仅略高于 0.55 属标定问题）。
    #[cfg(feature = "ml")]
    fn p82h_cosine_probe(store: &MemoryStore<JsonPersistence>, query: &str) -> Vec<(f32, String)> {
        let filter = crate::memory_store::ListFilter {
            limit: 1000,
            ..Default::default()
        };
        let Ok((memories, _)) = store.list_memories(&filter) else {
            return Vec::new();
        };
        let refs: Vec<&crate::memory_types::Memory> = memories.iter().collect();
        let sims = store.semantic_similarities(query, &refs);
        let mut scored: Vec<(f32, String)> = sims
            .into_iter()
            .enumerate()
            .filter_map(|(i, s)| s.map(|v| (v, memories[i].content.clone())))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(3);
        scored
    }

    /// P8.2m 单口径分布统计（去中心化余弦）：Top-3 + 均值 + 标准差 + 有效条数。
    #[cfg(feature = "ml")]
    struct P82mDistribution {
        /// Top-3（(余弦, 内容前 24 字)），按余弦降序
        top3: Vec<(f32, String)>,
        /// 该口径下有效候选的余弦均值
        mean: f32,
        /// 该口径下有效候选的余弦总体标准差
        std: f32,
        /// 该口径下有效候选条数（编码成功者）
        count: usize,
    }

    /// P8.2m：由余弦序列与对应内容构造分布统计（Top-3 + 均值 + 标准差）。
    #[cfg(feature = "ml")]
    fn p82m_distribution(sims: &[Option<f32>], contents: &[String]) -> P82mDistribution {
        let values: Vec<f32> = sims.iter().filter_map(|s| *s).collect();
        let count = values.len();
        let mean = if count == 0 {
            0.0
        } else {
            values.iter().sum::<f32>() / count as f32
        };
        let std = if count == 0 {
            0.0
        } else {
            (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / count as f32).sqrt()
        };
        let mut top3: Vec<(f32, String)> = sims
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.map(|v| (v, contents[i].chars().take(24).collect::<String>())))
            .collect();
        top3.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        top3.truncate(3);
        P82mDistribution {
            top3,
            mean,
            std,
            count,
        }
    }

    /// P8.2m：构造与生产 root 分支逐字一致的候选池（`top_k=ASSOCIATION_ROOT_POOL_TOPK`
    /// ＋ `explore_pure`）——作为"池内均值"口径的候选来源，与旁路实际传入
    /// `semantic_similarities_debiased` 的 `mem_refs`（`candidates`）同源。
    #[cfg(feature = "ml")]
    fn p82m_root_pool(
        store: &mut MemoryStore<JsonPersistence>,
        query: &str,
    ) -> Vec<crate::memory_types::Memory> {
        let filter = crate::memory_store::RecallFilter {
            memory_type: None,
            project: None,
            tags: Vec::new(),
            min_importance: None,
            top_k: ASSOCIATION_ROOT_POOL_TOPK,
            privacy_context: None,
            explore_pure: true,
            regression_query: None,
            read_only: false,
        };
        store
            .recall(query, &filter)
            .map(|result| result.memories)
            .unwrap_or_default()
    }

    /// P8.2m 口径对照探针（承接文档 §10.15 锁定项 2）：对**同一 store、同一查询**，
    /// 分别以「全库 106 条」与「候选池 8 条」为 `memories` 调用**去中心化**余弦，
    /// 得到两口径各自的 Top-3 与均值/标准差——单变量对照（唯一变量 = 均值基准池）。
    ///
    /// 均值基准即调用方传入的 `memories`（`semantic_similarities_impl` 内 `mean`
    /// 完全由 `memories` 的编码结果累积求得），故本对照**无需改动 `memory_store.rs`**。
    /// 返回 `(全库分布, 池内分布)`；同时保留 P8.2j 的全库去中心化 Top-3 语义
    /// （与旧 `p82j_cosine_probe` 同源同值），不破坏既有诊断口径。
    #[cfg(feature = "ml")]
    fn p82m_pool_vs_global_probe(
        store: &mut MemoryStore<JsonPersistence>,
        query: &str,
    ) -> (P82mDistribution, P82mDistribution) {
        // ① 全库口径：106 条语料（6 链 × 16 + 10 干扰），均值基准 = 全库
        let filter = crate::memory_store::ListFilter {
            limit: 1000,
            ..Default::default()
        };
        let global_dist = match store.list_memories(&filter) {
            Ok((memories, _)) => {
                let refs: Vec<&crate::memory_types::Memory> = memories.iter().collect();
                let sims =
                    store.semantic_similarities_debiased(query, &refs, assoc_suppress_lambda());
                let contents: Vec<String> = memories.iter().map(|m| m.content.clone()).collect();
                p82m_distribution(&sims, &contents)
            }
            Err(_) => P82mDistribution {
                top3: Vec::new(),
                mean: 0.0,
                std: 0.0,
                count: 0,
            },
        };
        // ② 池内口径：与生产旁路同源的候选池（recall top_k=8），均值基准 = 池内
        let pool = p82m_root_pool(store, query);
        let pool_refs: Vec<&crate::memory_types::Memory> = pool.iter().collect();
        let pool_sims =
            store.semantic_similarities_debiased(query, &pool_refs, assoc_suppress_lambda());
        let pool_contents: Vec<String> = pool.iter().map(|m| m.content.clone()).collect();
        let pool_dist = p82m_distribution(&pool_sims, &pool_contents);
        (global_dist, pool_dist)
    }

    /// P8.2m：单一"池内口径"分布（承接 §10.15 锁定项 1 的池级判据数据采集）。
    ///
    /// 与 `p82m_pool_vs_global_probe` 的 ② 口径逐字一致（`recall top_k=ASSOCIATION_ROOT_POOL_TOPK`
    /// ＋ 去中心化余弦），单独抽出以便对**关联查询侧**（30 条 `P82H_QUERIES`）复用。
    #[cfg(feature = "ml")]
    fn p82m_pool_dist(store: &mut MemoryStore<JsonPersistence>, query: &str) -> P82mDistribution {
        let pool = p82m_root_pool(store, query);
        let pool_refs: Vec<&crate::memory_types::Memory> = pool.iter().collect();
        let sims = store.semantic_similarities_debiased(query, &pool_refs, assoc_suppress_lambda());
        let contents: Vec<String> = pool.iter().map(|m| m.content.clone()).collect();
        p82m_distribution(&sims, &contents)
    }

    /// P8.2m：池内分布派生标量——Top-1 / Top-2 / 间隔 / z 分数。
    ///
    /// `z = (Top-1 − 池内均值) / 池内标准差`，刻画"Top-1 相对池内分布中心的偏离度"，
    /// 是**池级/分布判据**的候选形式（据 §10.8 已排除"池内单条余弦绝对排序"族）。
    #[cfg(feature = "ml")]
    struct P82mPoolStats {
        top1: f32,
        top2: f32,
        gap: f32,
        mean: f32,
        std: f32,
        z: f32,
        count: usize,
        top1_content: String,
    }

    #[cfg(feature = "ml")]
    fn p82m_pool_stats(dist: &P82mDistribution) -> P82mPoolStats {
        let top1 = dist.top3.first().map(|(v, _)| *v).unwrap_or(0.0);
        let top2 = dist.top3.get(1).map(|(v, _)| *v).unwrap_or(0.0);
        let top1_content = dist
            .top3
            .first()
            .map(|(_, c)| c.clone())
            .unwrap_or_default();
        let z = if dist.std > 0.0 {
            (top1 - dist.mean) / dist.std
        } else {
            0.0
        };
        P82mPoolStats {
            top1,
            top2,
            gap: top1 - top2,
            mean: dist.mean,
            std: dist.std,
            z,
            count: dist.count,
            top1_content,
        }
    }

    /// P8.2m：按池内去中心化余弦降序的完整排名（承接 §10.15 锁定项 1 的"链级共识"形式）。
    ///
    /// 与 `p82m_pool_dist` 同口径（`recall top_k=ASSOCIATION_ROOT_POOL_TOPK` ＋ 去中心化余弦），
    /// 但保留**完整内容**与链分类，供"Top-K 同链共识票数"与"跨查询高频 Top-1 条目"统计复用。
    #[cfg(feature = "ml")]
    fn p82m_pool_ranked(
        store: &mut MemoryStore<JsonPersistence>,
        query: &str,
    ) -> Vec<(&'static str, f32, String)> {
        let pool = p82m_root_pool(store, query);
        let refs: Vec<&crate::memory_types::Memory> = pool.iter().collect();
        let sims = store.semantic_similarities_debiased(query, &refs, assoc_suppress_lambda());
        let mut ranked: Vec<(&'static str, f32, String)> = pool
            .iter()
            .zip(sims.iter())
            .filter_map(|(m, s)| s.map(|v| (p82h_classify(&m.content).0, v, m.content.clone())))
            .collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked
    }

    /// P8.2m：池内 Top-K 中"同一链"的最大票数（排除 none）——链级共识判据的候选标量。
    #[cfg(feature = "ml")]
    fn p82m_best_chain_votes(ranked: &[(&'static str, f32, String)], k: usize) -> usize {
        let mut counts: std::collections::BTreeMap<&'static str, usize> =
            std::collections::BTreeMap::new();
        for (chain, _, _) in ranked.iter().take(k) {
            if *chain != "none" {
                *counts.entry(*chain).or_insert(0) += 1;
            }
        }
        counts.values().copied().max().unwrap_or(0)
    }

    /// P8.2m：单查询的链级共识采集记录（承接 §10.15 锁定项 1）。
    #[cfg(feature = "ml")]
    struct P82mChainStats {
        gold: &'static str,
        query: &'static str,
        /// Top-3 中最大非 none 同链票数（1..=3）
        c3: usize,
        /// 全池（8 条）中最大非 none 同链票数（1..=8）
        c8: usize,
        /// Top-1 所属链（未命中语料为 "none"）
        top1_chain: &'static str,
        /// Top-1 内容前 24 字（供跨查询霸榜统计，避免重复编码）
        top1_content: String,
    }

    /// P8.2h：离线 bge 批量编码验证 root 语义旁路救回率。
    ///
    /// 同进程内跑两臂（同一语料、同一代码路径）：
    /// - A 臂：统计编码器（旁路必然 unavailable）→ 复现 P7.4 基线；
    /// - B 臂：ML 编码器（bge 真实参与旁路）→ 实测救回率。
    ///
    /// 判据计算逐条对齐 _assoc_accuracy_eval.py::evaluate_h（H1-H4）。
    #[test]
    #[cfg(feature = "ml")]
    fn test_assoc_p82h_offline_bge_rescue_rates() {
        use std::collections::{BTreeMap, HashMap, HashSet};
        use std::time::{Instant, SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();

        // P8.2j 复测口径标注：本用例复用 P8.2h 骨架，靠门控切换实现通路。
        // 门控开启时输出 P8.2j 去中心化实测；关闭时即为 P8.2h 基线复现。
        eprintln!(
            "[P8.2j] 去中心化门控 LRC_ASSOC_DEBIAS={}（开启=候选池均值双侧对称去中心化）",
            if assoc_debias_enabled() {
                "ON"
            } else {
                "OFF（P8.2h 基线）"
            }
        );
        // P8.2k 阈值重标定：去中心化门控开启时生效阈值为 0.18（去中心化空间
        // 标定值），关闭时恒为 0.55（历史安全下限）——H3 判据据此修正为
        // "阈值须与所用向量空间一致"，而非"阈值恒等于 0.55"。
        eprintln!(
            "[P8.2k] 旁路生效阈值 assoc_bypass_min_sim()={:.2}（去中心化重标定值 {:.2}；历史下限 {:.2}）",
            assoc_bypass_min_sim(),
            ASSOC_DEBIAS_MIN_SEMANTIC_SIM,
            ASSOCIATION_ROOT_MIN_SEMANTIC_SIM
        );

        // ---------- A 臂：统计编码器（旁路缺席，复现 P7.4 基线）----------
        let dir_a = std::env::temp_dir().join(format!("lrc_p82h_a_{ts}"));
        let _ = std::fs::remove_dir_all(&dir_a);
        std::fs::create_dir_all(&dir_a).unwrap();
        let dir_a_str = dir_a.to_str().unwrap().to_string();
        let mut store_a = new_statistical_store(&dir_a_str);
        let t_seed_a = Instant::now();
        p82h_seed(&mut store_a);
        let t_a = Instant::now();
        let arm_a = p82h_run_all(&mut store_a);
        let a_elapsed = t_a.elapsed();
        let unrelated_a = p82h_probe_unrelated(&mut store_a);
        let a_correct = arm_a.iter().filter(|r| r.root_correct).count();
        let a_miss: Vec<&str> = arm_a
            .iter()
            .filter(|r| !r.root_correct)
            .map(|r| r.query)
            .collect();
        eprintln!(
            "[P8.2h][A 臂·统计] 写入 {} 条（{:.1}s）→ 30 查询耗时 {:.1}s；root 正确 {}/30，MISS {}",
            p82h_all_memories().len(),
            t_seed_a.elapsed().as_secs_f32(),
            a_elapsed.as_secs_f32(),
            a_correct,
            a_miss.len()
        );
        let set_a: HashSet<&str> = a_miss.iter().copied().collect();
        let set_doc: HashSet<&str> = P82H_DOC_MISS.iter().copied().collect();
        if set_a != set_doc {
            let extra: Vec<&&str> = set_a.difference(&set_doc).collect();
            let missing: Vec<&&str> = set_doc.difference(&set_a).collect();
            eprintln!(
                "[P8.2h][A 臂·统计] ⚠ MISS 集合与 P7.4 文档不一致：多出 {extra:?}，缺失 {missing:?}"
            );
        }

        // ---------- B 臂：ML 编码器（bge 真实参与语义旁路）----------
        let dir_b = std::env::temp_dir().join(format!("lrc_p82h_b_{ts}"));
        let _ = std::fs::remove_dir_all(&dir_b);
        std::fs::create_dir_all(&dir_b).unwrap();
        let dir_b_str = dir_b.to_str().unwrap().to_string();
        let Some(mut store_b) = new_ml_capable_store(&dir_b_str) else {
            eprintln!(
                "[P8.2h] ML 编码器不可用（bge 权重缺失或环境未设 LRC_LUOSHU_MODEL_ID）\
                 ——本用例仅在 ml 环境执行；请设置 LRC_LUOSHU_MODEL_ID=BAAI/bge-base-zh"
            );
            return;
        };
        let t_seed_b = Instant::now();
        p82h_seed(&mut store_b);
        let t_b = Instant::now();
        let arm_b = p82h_run_all(&mut store_b);
        let b_elapsed = t_b.elapsed();
        let unrelated_b = p82h_probe_unrelated(&mut store_b);
        let b_correct = arm_b.iter().filter(|r| r.root_correct).count();
        // 根因探针：无关联查询的全库余弦 Top-3（判定 H2 FAIL 属"阈值标定"还是"机制失效"）
        // P8.2j：同时打印去中心化余弦（全库均值口径）。
        // P8.2m：再叠加"池内均值"口径，形成同库同查询的【全库均值 vs 池内均值】单变量对照
        //        （承接 §10.15 锁定项 2；两口径唯一差异 = 去中心化均值基准所用候选集）。
        for query in P82H_UNRELATED {
            let top = p82h_cosine_probe(&store_b, query);
            eprintln!("[P8.2h][B 臂·余弦] 「{query}」全库 Top-3：");
            for (sim, content) in &top {
                eprintln!("  · 余弦={sim:.4} 内容={content}");
            }
            let (global_dist, pool_dist) = p82m_pool_vs_global_probe(&mut store_b, query);
            eprintln!(
                "[P8.2m][全库口径] 「{query}」n={} 均值={:.4} 标准差={:.4} Top-3：",
                global_dist.count, global_dist.mean, global_dist.std
            );
            for (sim, content) in &global_dist.top3 {
                eprintln!("  · 余弦={sim:.4} 内容={content}");
            }
            eprintln!(
                "[P8.2m][池内口径] 「{query}」n={} 均值={:.4} 标准差={:.4} Top-3：",
                pool_dist.count, pool_dist.mean, pool_dist.std
            );
            for (sim, content) in &pool_dist.top3 {
                eprintln!("  · 余弦={sim:.4} 内容={content}");
            }
            // 口径对照：池内 Top-1 与全库 Top-1 的差值（>0 表示池内口径放大，<0 表示池内口径收紧）
            let g_top1 = global_dist.top3.first().map(|(v, _)| *v).unwrap_or(0.0);
            let p_top1 = pool_dist.top3.first().map(|(v, _)| *v).unwrap_or(0.0);
            eprintln!(
                "[P8.2m][口径差] 池内 Top-1 − 全库 Top-1 = {:.4}（池内 {:.4} / 全库 {:.4}；\
                 池内标准差 {:.4} / 全库标准差 {:.4}）",
                p_top1 - g_top1,
                p_top1,
                g_top1,
                pool_dist.std,
                global_dist.std
            );
        }
        eprintln!(
            "[P8.2h][B 臂·ml ] 写入 {} 条（{:.1}s）→ 30 查询耗时 {:.1}s；root 正确 {}/30",
            p82h_all_memories().len(),
            t_seed_b.elapsed().as_secs_f32(),
            b_elapsed.as_secs_f32(),
            b_correct
        );

        // ---------- P8.2m 池级/分布判据（承接 §10.15 锁定项 1）----------
        // 双侧同口径采集：关联侧（30 条 P82H_QUERIES）+ 无关侧（22 条 P82H_UNRELATED），
        // 口径与生产旁路逐字一致（recall top_k=ASSOCIATION_ROOT_POOL_TOPK ＋ 去中心化余弦）。
        // 候选判据 z = (Top-1 − 池内均值) / 池内标准差（§10.8 已排除"池内单条余弦排序"族）。
        let mut related_stats: Vec<(&'static str, P82mPoolStats)> = Vec::new();
        for &(gold, query) in P82H_QUERIES {
            let dist = p82m_pool_dist(&mut store_b, query);
            let stats = p82m_pool_stats(&dist);
            eprintln!(
                "[P8.2m][池级·关联侧] 「{query}」gold={gold} n={} Top-1={:.4} Top-2={:.4} \
                 间隔={:.4} 均值={:.4} 标准差={:.4} z={:.3} Top-1内容={}",
                stats.count,
                stats.top1,
                stats.top2,
                stats.gap,
                stats.mean,
                stats.std,
                stats.z,
                stats.top1_content
            );
            related_stats.push((query, stats));
        }
        let mut unrelated_stats: Vec<(&'static str, P82mPoolStats)> = Vec::new();
        for &query in P82H_UNRELATED {
            let dist = p82m_pool_dist(&mut store_b, query);
            let stats = p82m_pool_stats(&dist);
            eprintln!(
                "[P8.2m][池级·无关侧] 「{query}」n={} Top-1={:.4} 间隔={:.4} 均值={:.4} \
                 标准差={:.4} z={:.3} Top-1内容={}",
                stats.count,
                stats.top1,
                stats.gap,
                stats.mean,
                stats.std,
                stats.z,
                stats.top1_content
            );
            unrelated_stats.push((query, stats));
        }
        // 阈值扫描：目标 = 无关侧放行 0 条且关联侧 MISS 救回最大。
        // 因判据为 z ≥ t，t 必须严格大于无关侧 z 上界；可行域内取"救回最多"的最小 t。
        let miss_set: HashSet<&str> = P82H_DOC_MISS.iter().copied().collect();
        let unrel_z_max = unrelated_stats
            .iter()
            .map(|(_, s)| s.z)
            .fold(f32::MIN, f32::max);
        let mut rel_miss_z: Vec<f32> = related_stats
            .iter()
            .filter(|(q, _)| miss_set.contains(*q))
            .map(|(_, s)| s.z)
            .collect();
        rel_miss_z.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut best_t = f32::INFINITY;
        let mut best_miss_pass = 0usize;
        for t in rel_miss_z.iter().copied() {
            if unrelated_stats.iter().filter(|(_, s)| s.z >= t).count() > 0 {
                continue;
            }
            let miss_pass = rel_miss_z.iter().filter(|z| **z >= t).count();
            if miss_pass > best_miss_pass {
                best_miss_pass = miss_pass;
                best_t = t;
            }
        }
        eprintln!(
            "[P8.2m][池级判据] 无关侧 z 上界={:.3}（22 条）；关联侧 MISS(16) z 最小={:.3} \
             中位={:.3} 最大={:.3}",
            unrel_z_max,
            rel_miss_z.first().copied().unwrap_or(0.0),
            rel_miss_z.get(rel_miss_z.len() / 2).copied().unwrap_or(0.0),
            rel_miss_z.last().copied().unwrap_or(0.0)
        );
        let unrel_top1_max = unrelated_stats
            .iter()
            .map(|(_, s)| s.top1)
            .fold(f32::MIN, f32::max);
        let miss_top1_min = related_stats
            .iter()
            .filter(|(q, _)| miss_set.contains(*q))
            .map(|(_, s)| s.top1)
            .fold(f32::MAX, f32::min);
        eprintln!(
            "[P8.2m][池级判据] 阈值扫描：可行最优 t={:.3}（须 > 无关上界 {:.3}）→ 无关放行 0，\
             MISS 救回 {}/16 → {}",
            best_t,
            unrel_z_max,
            best_miss_pass,
            if best_miss_pass >= 10 { "PASS" } else { "FAIL" }
        );
        eprintln!(
            "[P8.2m][池级判据·对照] 固定阈值族（Top-1 绝对值）：无关侧上界={:.4}，\
             关联 MISS 下界={:.4} → 可分离={}",
            unrel_top1_max,
            miss_top1_min,
            unrel_top1_max < miss_top1_min
        );

        // ---------- P8.2m 链级共识判据（锁定项 1 剩余候选形式）----------
        // 候选标量：Top-K 内"同一链"的最大票数（Top-3 / 全池 8 两条口径）。
        // 采样对象与生产旁路逐字同口径（含代码类候选），不额外过滤。
        let mut chain_all: Vec<P82mChainStats> = Vec::new();
        for &(gold, query) in P82H_QUERIES {
            let ranked = p82m_pool_ranked(&mut store_b, query);
            let top1_chain = ranked.first().map(|(c, _, _)| *c).unwrap_or("none");
            let top1_content: String = ranked
                .first()
                .map(|(_, _, c)| c.chars().take(24).collect())
                .unwrap_or_default();
            chain_all.push(P82mChainStats {
                gold,
                query,
                c3: p82m_best_chain_votes(&ranked, 3),
                c8: p82m_best_chain_votes(&ranked, 8),
                top1_chain,
                top1_content,
            });
        }
        for &query in P82H_UNRELATED {
            let ranked = p82m_pool_ranked(&mut store_b, query);
            let top1_chain = ranked.first().map(|(c, _, _)| *c).unwrap_or("none");
            let top1_content: String = ranked
                .first()
                .map(|(_, _, c)| c.chars().take(24).collect())
                .unwrap_or_default();
            chain_all.push(P82mChainStats {
                gold: "none",
                query,
                c3: p82m_best_chain_votes(&ranked, 3),
                c8: p82m_best_chain_votes(&ranked, 8),
                top1_chain,
                top1_content,
            });
        }
        for s in chain_all.iter() {
            eprintln!(
                "[P8.2m][链级共识] 「{}」gold={} Top-3 同链最大票={} 全池同链最大票={} Top-1链={}",
                s.query, s.gold, s.c3, s.c8, s.top1_chain
            );
        }
        // 阈值扫描（c3）：目标 = 无关侧放行 0 且关联 MISS 救回最大（判据 c3 ≥ t）。
        let related_chain: Vec<&P82mChainStats> =
            chain_all.iter().filter(|s| s.gold != "none").collect();
        let unrelated_chain: Vec<&P82mChainStats> =
            chain_all.iter().filter(|s| s.gold == "none").collect();
        for (label, pick) in [
            (
                "Top-3",
                (|s: &P82mChainStats| s.c3) as fn(&P82mChainStats) -> usize,
            ),
            (
                "全池8",
                (|s: &P82mChainStats| s.c8) as fn(&P82mChainStats) -> usize,
            ),
        ] {
            let unrel_max = unrelated_chain.iter().map(|s| pick(s)).max().unwrap_or(0);
            let mut miss_votes: Vec<usize> = related_chain
                .iter()
                .filter(|s| miss_set.contains(s.query))
                .map(|s| pick(s))
                .collect();
            miss_votes.sort_unstable();
            let mut best_t = usize::MAX;
            let mut best_pass = 0usize;
            for &t in miss_votes.iter().rev() {
                if t <= unrel_max || t == 0 {
                    continue;
                }
                let pass = miss_votes.iter().filter(|v| **v >= t).count();
                if pass > best_pass {
                    best_pass = pass;
                    best_t = t;
                }
            }
            eprintln!(
                "[P8.2m][链级共识判据·{label}] 无关侧上界={unrel_max}（22 条）；\
                 关联 MISS(16) 票分布={miss_votes:?} → 可行最优 t={} → 无关放行 0，\
                 MISS 救回 {best_pass}/16 → {}",
                if best_t == usize::MAX {
                    "不可行".to_string()
                } else {
                    best_t.to_string()
                },
                if best_pass >= 10 { "PASS" } else { "FAIL" }
            );
        }
        // 跨查询高频 Top-1 条目（承接锁定项 3 的"全局霸榜条目"观察）。
        let mut top1_freq: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for s in chain_all.iter() {
            if !s.top1_content.is_empty() {
                *top1_freq.entry(s.top1_content.clone()).or_insert(0) += 1;
            }
        }
        let mut freq_sorted: Vec<(String, usize)> = top1_freq.into_iter().collect();
        freq_sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        eprintln!("[P8.2m][霸榜条目·池内口径] 跨 52 查询（30 关联 + 22 无关）Top-1 频次 Top-5：");
        for (content, freq) in freq_sorted.iter().take(5) {
            eprintln!("  · {freq}/52 次  {content}");
        }

        // ---------- 判据计算（对齐 evaluate_h：H1-H4）----------
        let b_by_query: HashMap<&str, &P82hRecord> = arm_b.iter().map(|r| (r.query, r)).collect();

        // H1：P7.4 文档 16 个 MISS 查询中，B 臂救回（root 正确）且旁路真实 applied 的数量
        let mut miss_bypass: BTreeMap<String, usize> = BTreeMap::new();
        let mut saved: Vec<&str> = Vec::new();
        let mut saved_applied: Vec<&str> = Vec::new();
        eprintln!("[P8.2h] 16 个 MISS 查询在 B 臂的逐条结果：");
        for &query in P82H_DOC_MISS {
            let Some(record) = b_by_query.get(query) else {
                continue;
            };
            *miss_bypass
                .entry(record.semantic_bypass.clone())
                .or_insert(0) += 1;
            if record.root_correct {
                saved.push(query);
                if record.semantic_bypass == "applied" {
                    saved_applied.push(query);
                }
            }
            eprintln!(
                "  · {query:<16} root_chain={:<8} correct={:<5} weak={:<5} bypass={}",
                record.root_chain, record.root_correct, record.weak_match, record.semantic_bypass
            );
        }
        let h1 = saved_applied.len() >= 10;
        eprintln!(
            "[P8.2h] H1 救回率：{} / 16（其中旁路 applied {} / 16，阈值 ≥10）→ {}；\
             MISS 旁路分布 {miss_bypass:?}",
            saved.len(),
            saved_applied.len(),
            if h1 { "PASS" } else { "FAIL" }
        );

        // H2：A 臂 root 正确的查询在 B 臂不得退化；无关联查询两臂均须诚实空态
        let regressed: Vec<&str> = arm_a
            .iter()
            .filter(|r| r.root_correct && !b_by_query[r.query].root_correct)
            .map(|r| r.query)
            .collect();
        let b_unrelated_ok = unrelated_b.iter().all(|(_, honest, _, _)| *honest);
        let a_unrelated_ok = unrelated_a.iter().all(|(_, honest, _, _)| *honest);
        let h2 = regressed.is_empty() && b_unrelated_ok;
        eprintln!(
            "[P8.2h] H2 无新增噪声：退化查询 {regressed:?}；无关联空态 A 臂={a_unrelated_ok} \
             B 臂={b_unrelated_ok}（条目 {unrelated_b:?}）→ {}",
            if h2 { "PASS" } else { "FAIL" }
        );

        // H3（P8.2k 修正）：判据由"余弦阈值维持 0.55"改为
        // **"生效阈值须与所用向量空间一致"**——修正理由见文档 §10.4'（P8.2j 已识别
        // 的结构性冲突：去中心化改变余弦量纲，若仍锁死 0.55 则机制有效也永远不通过）。
        // 双条件：①历史安全下限常量必须恒为 0.55（不得因追求召回而下调）；
        //         ②门控开启时生效阈值 = 去中心化空间标定值 0.18；关闭时 = 0.55。
        let base_ok = (ASSOCIATION_ROOT_MIN_SEMANTIC_SIM - 0.55).abs() < 1e-6;
        let effective_ok = if assoc_debias_enabled() {
            (assoc_bypass_min_sim() - ASSOC_DEBIAS_MIN_SEMANTIC_SIM).abs() < 1e-6
        } else {
            (assoc_bypass_min_sim() - ASSOCIATION_ROOT_MIN_SEMANTIC_SIM).abs() < 1e-6
        };
        let h3 = base_ok && effective_ok;
        eprintln!(
            "[P8.2h] H3 门槛一致（P8.2k 修正）：历史下限 ASSOCIATION_ROOT_MIN_SEMANTIC_SIM \
             = {}（应 0.55，未下调={base_ok}）；生效阈值 = {}（去中心化={}，应 {}）→ {}",
            ASSOCIATION_ROOT_MIN_SEMANTIC_SIM,
            assoc_bypass_min_sim(),
            assoc_debias_enabled(),
            if assoc_debias_enabled() {
                ASSOC_DEBIAS_MIN_SEMANTIC_SIM
            } else {
                ASSOCIATION_ROOT_MIN_SEMANTIC_SIM
            },
            if h3 { "PASS" } else { "FAIL" }
        );

        let h4 = h1 && h2 && h3;
        eprintln!(
            "[P8.2h] H4 判定：H1={h1} H2={h2} H3={h3} → {}",
            if h4 {
                "PASS（旁路救回达标，可进入默认开启评审）"
            } else {
                "FAIL（如实记录救回率不足，维持诚实空态）"
            }
        );

        // ---------- 不变量断言（保留已打印的实测数据前提下，锁死可确证契约）----------
        assert_eq!(
            a_correct, 14,
            "A 臂（统计模式）应复现 P7.4 基线的 14 条 root 正确；实测 {a_correct}"
        );
        assert!(
            miss_bypass.contains_key("applied"),
            "B 臂（ml）在弱匹配查询上旁路必须 applied（bge 真实产出向量），实测分布 {miss_bypass:?}"
        );

        // ---------- H1–H4 判据硬断言（v0.9.7 门禁加固，M9「门禁必须能失败」）----------
        //
        // 背景（本轮修复）：此前 H1–H4 仅为 `eprintln!` 打印型诊断、**不含任何 assert**
        // （见文档 §10.20.4「判据性质声明」），故**判据退化时 CI 不会失败**——读者只能
        // 从测试日志正文人工核对，与项目第七轮确立的 M9「门禁必须能失败」相悖。
        // 该缺陷在联想能力上尤为危险：H1/H2 正是 `LRC_ASSOC_DEBIAS` 默认开启的**唯一
        // 依据**，若其静默退化（如模型版本更新、语料漂移），生产联想质量会在无告警的
        // 情况下下降。
        //
        // 加固方式：把已打印的四个布尔量直接断言。**不改变任何判据口径**（阈值 ≥10、
        // 零放行、H3 双条件均取自上方原计算），仅把"打印"升级为"可失败"。
        //
        // 前置条件：本段位于 `store_b` 夺取成功之后（ML 不可用时上方已 `return`），
        // 故断言不会在无模型环境下误报失败——这与"用例仅在 ml 环境执行"的既有约定一致。
        assert!(
            h1,
            "H1 救回率未达标：saved_applied={} / 16（阈值 ≥10）；\
             MISS 旁路分布 {miss_bypass:?}。若为真实退化，说明语义旁路救回能力下降，\
             `LRC_ASSOC_DEBIAS` 默认开启的依据已不成立，须复核阈值 0.18 的标定。",
            saved_applied.len()
        );
        assert!(
            h2,
            "H2 噪声放行失效：退化查询 {regressed:?}；无关联空态 A 臂={a_unrelated_ok} \
             B 臂={b_unrelated_ok}（被误召回条目 {unrelated_b:?}）。\
             即无关查询被旁路放行成起点，是生产联想质量最直接的风险信号。"
        );
        assert!(
            h3,
            "H3 门槛配对失效：历史下限 = {ASSOCIATION_ROOT_MIN_SEMANTIC_SIM}（应 0.55，\
             不得下调）；生效阈值 = {}（去中心化={}）。阈值必须与所用向量空间配对。",
            assoc_bypass_min_sim(),
            assoc_debias_enabled()
        );
        assert!(
            h4,
            "H4 综合判定 FAIL（H1={h1} H2={h2} H3={h3}）——联想质量门禁整体未通过"
        );
    }

    /// P8.2n：模长类代理（L2 范数）形式覆盖检查——关闭"无状态代理压制"路线的
    /// 最后一个理论候选。
    ///
    /// 背景（承接 §10.16.8 锁定项 1）：压制路线已试过两种"无状态中心度代理"，
    /// 均同族失效：
    ///
    /// 1. 去中心化空间 κ——受零和约束 `Σ_j c_j = 0` 支配，两两余弦均值退化为
    ///    去中心化模长的单调递减函数，即"模长最大者 κ 最负 ⇒ excess=0 ⇒
    ///    永不被压"，方向系统性反向（λ=10 档实测 H1 崩至 9/16 已证伪）；
    /// 2. 原始空间 τ——λ=10 修正版实测 H1 由 13/16 崩至 10/16，霸榜条目
    ///    （exam，52 查询 11 次池内 Top-1）反升至 12 次，仅台风 1 条误召回
    ///    被压过阈值 ⇒ 代理无判别力，放大只带来附带损伤。
    ///
    /// 最后一个候选是**模长类代理**：直接以 `‖v_i‖` 作为压制信号。因
    /// `encode_embedding` 返回**未归一化**的 CLS 向量，全库模长各异，故该代理
    /// 不被恒等退化排除，必须实测而非解析论证。判定标准：
    ///
    /// - 若霸榜条目即模长极值 ⇒ 模长代理与 κ 同型退化（"模长最大者永不被压"），
    ///   "无状态代理压制"路线**整体否证**，须转向口径重构或状态化方案；
    /// - 若霸榜条目并非模长极值 ⇒ 仍有新代理空间。
    ///
    /// 本探针为**纯观测**：只读语料常量、只调用 `encode_embedding`，
    /// 不进入任何判据路径（零回归）。
    #[test]
    #[cfg(feature = "ml")]
    fn test_assoc_p82n_l2_norm_probe() {
        let Ok(ml) = crate::engine::luoshu_encoder_ml::LuoShuMlEncoder::load() else {
            eprintln!("[P8.2n][模长代理] ML 编码器不可用，跳过模长探针");
            return;
        };
        let encoder = crate::engine::luoshu_encoder_ml::HybridLuoShuEncoder::new_with_ml(ml);
        let corpus = p82h_all_memories();
        // 并行编码（编码器 Sync）——串行 106 条会显著拉长用例时长。
        let vectors: Vec<Option<Vec<f32>>> = std::thread::scope(|scope| {
            let handles: Vec<_> = corpus
                .iter()
                .map(|(_, _, content)| {
                    let encoder = &encoder;
                    scope.spawn(move || encoder.encode_embedding(content))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or(None))
                .collect()
        });
        let norms: Vec<f32> = vectors
            .iter()
            .map(|v| {
                v.as_ref()
                    .map_or(0.0, |v| v.iter().map(|x| x * x).sum::<f32>().sqrt())
            })
            .collect();
        let mut sorted: Vec<f32> = norms.iter().copied().filter(|n| *n > 0.0).collect();
        if sorted.is_empty() {
            eprintln!("[P8.2n][模长代理] 全部编码失败，跳过模长探针");
            return;
        }
        sorted.sort_by(f32::total_cmp);
        let n = sorted.len();
        let pick = |q: f64| sorted[((q * n as f64).ceil() as usize).clamp(1, n) - 1];
        let mean = sorted.iter().sum::<f32>() / n as f32;
        eprintln!(
            "[P8.2n][模长代理] 语料 n={n}/{}（有效）L2 范数：min={:.3} p25={:.3} 中位={:.3} p75={:.3} max={:.3} 均值={:.3}",
            corpus.len(),
            sorted[0],
            pick(0.25),
            pick(0.5),
            pick(0.75),
            sorted[n - 1],
            mean
        );

        // 待检验条目：霸榜条目 + 3 条池内误召回 Top-1 的常客。
        const WATCH: &[&str] = &[
            "孩子下周三期末考，数学应用题是他的坎", // 霸榜（52 查询 11 次池内 Top-1）
            "预报周六多云十八度，傍晚起风",         // 台风查询误召回 Top-1（λ=10 被压至 0.1796）
            "交强险和车船税一起续，保单在抽屉",     // 合同法/跨境电商误召回 Top-1
            "部门五个人拼一辆七座商务车",           // 交响乐团误召回 Top-1
        ];
        let median = pick(0.5);
        for content in WATCH {
            let Some(pos) = corpus.iter().position(|(_, _, c)| *c == *content) else {
                continue;
            };
            let norm = norms[pos];
            let rank = sorted.iter().filter(|x| **x > norm).count() + 1;
            eprintln!(
                "[P8.2n][模长代理] 秩 {rank}/{n}（1=最大）模长={norm:.3} 相对中位={:.3}×  {content}",
                if median > 0.0 { norm / median } else { 0.0 }
            );
        }

        // 模长极值对照：判定"霸榜条目是否即模长最大者"。
        let mut idx: Vec<usize> = (0..corpus.len()).filter(|&i| norms[i] > 0.0).collect();
        idx.sort_by(|&a, &b| norms[b].total_cmp(&norms[a]));
        eprintln!("[P8.2n][模长代理] 模长 Top-5：");
        for &i in idx.iter().take(5) {
            eprintln!(
                "  · 模长={:.3} 链={} 内容={}",
                norms[i], corpus[i].0, corpus[i].2
            );
        }
        eprintln!("[P8.2n][模长代理] 模长 Bottom-5：");
        for &i in idx.iter().rev().take(5) {
            eprintln!(
                "  · 模长={:.3} 链={} 内容={}",
                norms[i], corpus[i].0, corpus[i].2
            );
        }
    }

    /// P8.2n+ 可分性统计：Mann-Whitney AUC（并列取平均秩）。
    ///
    /// 输入 `(标量, 是否正类)`；返回 `(AUC, 正类数, 负类数)`，任一类别为空返回
    /// `None`。AUC 等价于 `P(随机正类分数 > 随机负类分数)`——AUC=0.5 表示该标量
    /// **不携带任何类别信息**，任何基于它的阈值/排序判据都不可能工作。
    #[cfg(feature = "ml")]
    fn p82n_auc(samples: &[(f32, bool)]) -> Option<(f32, usize, usize)> {
        let n_pos = samples.iter().filter(|(_, p)| *p).count();
        let n_neg = samples.len().saturating_sub(n_pos);
        if n_pos == 0 || n_neg == 0 {
            return None;
        }
        let mut sorted: Vec<(f32, bool)> = samples.to_vec();
        sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
        // 并列组取平均秩（1-based）
        let mut ranks = vec![0.0f64; sorted.len()];
        let mut i = 0usize;
        while i < sorted.len() {
            let mut j = i + 1;
            while j < sorted.len()
                && sorted[j].0.total_cmp(&sorted[i].0) == std::cmp::Ordering::Equal
            {
                j += 1;
            }
            let avg = (i + 1 + j) as f64 / 2.0;
            for r in ranks[i..j].iter_mut() {
                *r = avg;
            }
            i = j;
        }
        let rank_sum_pos: f64 = ranks
            .iter()
            .zip(sorted.iter())
            .filter(|(_, entry)| entry.1)
            .map(|(r, _)| *r)
            .sum();
        let auc = (rank_sum_pos - (n_pos * (n_pos + 1)) as f64 / 2.0) / (n_pos * n_neg) as f64;
        Some((auc as f32, n_pos, n_neg))
    }

    /// P8.2n+ 可分性统计：10 箱重叠面积系数（OVL）。
    ///
    /// 两经验分布在同一箱区间上的 `Σ min(p_i, q_i)`；1.0 = 完全重叠（不可分），
    /// 0.0 = 完全不重叠。小样本下噪声较大，仅作 AUC 的辅助印证。
    #[cfg(feature = "ml")]
    fn p82n_ovl(pos: &[f32], neg: &[f32], bins: usize) -> f32 {
        if pos.is_empty() || neg.is_empty() || bins == 0 {
            return f32::NAN;
        }
        let lo = pos
            .iter()
            .chain(neg.iter())
            .copied()
            .fold(f32::INFINITY, f32::min);
        let hi = pos
            .iter()
            .chain(neg.iter())
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        if hi <= lo {
            return 1.0;
        }
        let w = (hi - lo) / bins as f32;
        let mut ovl = 0.0f32;
        for b in 0..bins {
            let l = lo + w * b as f32;
            // 末箱上界取闭区间，避免丢掉样本最大值
            let h = if b + 1 == bins {
                hi + 1e-6
            } else {
                lo + w * (b + 1) as f32
            };
            let p = pos.iter().filter(|s| **s >= l && **s < h).count() as f32 / pos.len() as f32;
            let q = neg.iter().filter(|s| **s >= l && **s < h).count() as f32 / neg.len() as f32;
            ovl += p.min(q);
        }
        ovl
    }

    /// P8.2n+ 候选池语义可分性确认探针（承接文档 §10.17.9 锁定项 1，纯观测零回归）。
    ///
    /// 以**不依赖具体判据形式**的秩统计（AUC / 秩分布 / 重叠面积）定量回答：
    /// 候选池内"关联候选（属查询 gold 链）vs 其余候选"是否可分。分两层定位根因：
    /// - **查询内**：同一查询池内 gold 与噪声在去中心化/原始余弦上是否分开
    ///   —— 决定"池内阈值/相对排序"族判据是否还有空间；
    /// - **查询间**：关联查询的池内 Top-1 与无关查询的池内 Top-1 是否分开
    ///   —— 决定 H2（无关侧零放行）能否靠绝对阈值满足。
    ///
    /// 结论分岔：查询内可分而查询间不可分 ⇒ 根因 = 跨查询尺度不可比（仍有新判据
    /// 空间）；两层皆不可分 ⇒ 根因 = 池组成语义混杂（只能走状态化或池重构）。
    ///
    /// 本探针只读语料常量、只调用生产相似度函数与既有候选池构造，不进入任何
    /// 判据路径（零回归）。
    #[test]
    #[cfg(feature = "ml")]
    fn test_assoc_p82n_pool_separability_probe() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_p82n_sep_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let Some(mut store) = new_ml_capable_store(dir.to_str().unwrap()) else {
            eprintln!(
                "[P8.2n+][可分性] ML 编码器不可用（bge 权重缺失或未设 LRC_LUOSHU_MODEL_ID），跳过可分性探针"
            );
            return;
        };
        p82h_seed(&mut store);

        // 预注册判定阈值（避免事后调参）：AUROC 不低于 0.75 判可分，不高于 0.60 判不可分
        const SEPARABLE_AUC: f32 = 0.75;
        const INSEPARABLE_AUC: f32 = 0.60;

        let mut within_auc_dec: Vec<f32> = Vec::new();
        let mut within_auc_raw: Vec<f32> = Vec::new();
        let mut within_ovl_dec: Vec<f32> = Vec::new();
        let mut gold_best_rank: Vec<f32> = Vec::new();
        let mut pool_sizes: Vec<usize> = Vec::new();
        let mut gold_present = 0usize;
        let mut merged_dec: Vec<(f32, bool)> = Vec::new();
        let mut merged_raw: Vec<(f32, bool)> = Vec::new();
        let mut top1_related: Vec<f32> = Vec::new();
        // §10.18 关键补测：H1 只关心 16 条 MISS，故单独采集其池内 Top-1，
        // 用于检验"是否存在同时满足 H1（≥10/16）与 H2（无关零放行）的绝对阈值"。
        let mut miss_top1: Vec<f32> = Vec::new();
        let miss_set: std::collections::HashSet<&str> = P82H_DOC_MISS.iter().copied().collect();

        for &(gold, query) in P82H_QUERIES {
            let pool = p82m_root_pool(&mut store, query);
            if pool.is_empty() {
                continue;
            }
            let refs: Vec<&crate::memory_types::Memory> = pool.iter().collect();
            let dec = store.semantic_similarities_debiased(query, &refs, assoc_suppress_lambda());
            let raw = store.semantic_similarities(query, &refs);
            let mut samples_dec: Vec<(f32, bool)> = Vec::new();
            let mut samples_raw: Vec<(f32, bool)> = Vec::new();
            let mut pos_dec: Vec<f32> = Vec::new();
            let mut neg_dec: Vec<f32> = Vec::new();
            let mut has_gold = false;
            for (i, memory) in pool.iter().enumerate() {
                let is_gold = p82h_classify(&memory.content).0 == gold;
                has_gold |= is_gold;
                if let Some(s) = dec[i] {
                    samples_dec.push((s, is_gold));
                    if is_gold {
                        pos_dec.push(s);
                    } else {
                        neg_dec.push(s);
                    }
                }
                if let Some(s) = raw[i] {
                    samples_raw.push((s, is_gold));
                }
            }
            pool_sizes.push(pool.len());
            if has_gold {
                gold_present += 1;
            }
            if let Some((auc, _, _)) = p82n_auc(&samples_dec) {
                within_auc_dec.push(auc);
                within_ovl_dec.push(p82n_ovl(&pos_dec, &neg_dec, 10));
            }
            if let Some((auc, _, _)) = p82n_auc(&samples_raw) {
                within_auc_raw.push(auc);
            }
            // gold 在池内去中心化降序中的最佳秩（纯秩方法，不依赖阈值）
            let mut ranked = samples_dec.clone();
            ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
            if let Some(pos) = ranked.iter().position(|(_, g)| *g) {
                gold_best_rank.push((pos + 1) as f32);
            }
            merged_dec.extend(samples_dec.iter().copied());
            merged_raw.extend(samples_raw.iter().copied());
            if let Some(max) = samples_dec.iter().map(|(s, _)| *s).reduce(f32::max) {
                top1_related.push(max);
            }
            // H1 口径：MISS 查询关心的是 **gold 候选本身**能否越过阈值（决定该查询能否
            // 被救回），故单独记录其池内最高分，而非池内全局 Top-1（可能仍被噪声霸占）。
            if miss_set.contains(query) {
                if let Some(gold_max) = pos_dec.iter().copied().reduce(f32::max) {
                    miss_top1.push(gold_max);
                }
            }
        }

        // 无关侧：池内 Top-1（同口径），用于查询间对照
        let mut top1_unrelated: Vec<f32> = Vec::new();
        for &query in P82H_UNRELATED {
            let pool = p82m_root_pool(&mut store, query);
            if pool.is_empty() {
                continue;
            }
            let refs: Vec<&crate::memory_types::Memory> = pool.iter().collect();
            let dec = store.semantic_similarities_debiased(query, &refs, assoc_suppress_lambda());
            if let Some(max) = dec.iter().flatten().copied().reduce(f32::max) {
                top1_unrelated.push(max);
            }
        }

        let mean = |v: &[f32]| -> f32 {
            if v.is_empty() {
                0.0
            } else {
                v.iter().sum::<f32>() / v.len() as f32
            }
        };
        let median = |v: &[f32]| -> f32 {
            if v.is_empty() {
                return 0.0;
            }
            let mut s = v.to_vec();
            s.sort_by(f32::total_cmp);
            s[s.len() / 2]
        };
        let over = |v: &[f32], t: f32| v.iter().filter(|x| **x >= t).count();
        let sizes_f32: Vec<f32> = pool_sizes.iter().map(|n| *n as f32).collect();

        eprintln!(
            "[P8.2n+][可分性] 关联查询 {} 条，池非空 {} 条，池内 gold 存在 {} 条；池规模均值={:.2}",
            P82H_QUERIES.len(),
            pool_sizes.len(),
            gold_present,
            mean(&sizes_f32)
        );
        eprintln!(
            "[P8.2n+][可分性][查询内] AUC(去中心化余弦) n={} 均值={:.3} 中位={:.3} 最小={:.3} 最大={:.3}；\
             >0.5 者 {}/{}；不低于 0.75 者 {}/{}",
            within_auc_dec.len(),
            mean(&within_auc_dec),
            median(&within_auc_dec),
            within_auc_dec.iter().copied().fold(f32::INFINITY, f32::min),
            within_auc_dec
                .iter()
                .copied()
                .fold(f32::NEG_INFINITY, f32::max),
            over(&within_auc_dec, 0.5001),
            within_auc_dec.len(),
            over(&within_auc_dec, SEPARABLE_AUC),
            within_auc_dec.len()
        );
        eprintln!(
            "[P8.2n+][可分性][查询内] AUC(原始余弦)   n={} 均值={:.3} 中位={:.3}；不低于 0.75 者 {}/{}",
            within_auc_raw.len(),
            mean(&within_auc_raw),
            median(&within_auc_raw),
            over(&within_auc_raw, SEPARABLE_AUC),
            within_auc_raw.len()
        );
        eprintln!(
            "[P8.2n+][可分性][查询内] OVL(去中心化余弦,10箱) 均值={:.3} 中位={:.3}（1.0=完全重叠）",
            mean(&within_ovl_dec),
            median(&within_ovl_dec)
        );
        eprintln!(
            "[P8.2n+][可分性][查询内] gold 最佳秩 n={} 均值={:.2} 中位={:.2}；秩=1 者 {}/{}（随机期望约为池半宽）",
            gold_best_rank.len(),
            mean(&gold_best_rank),
            median(&gold_best_rank),
            gold_best_rank
                .iter()
                .filter(|r| (**r - 1.0).abs() < 1e-6)
                .count(),
            gold_best_rank.len()
        );

        if let Some((auc, np, nn)) = p82n_auc(&merged_dec) {
            eprintln!("[P8.2n+][可分性][合并] AUC(去中心化余弦)={auc:.3}（正 {np} / 负 {nn}）");
        }
        if let Some((auc, np, nn)) = p82n_auc(&merged_raw) {
            eprintln!("[P8.2n+][可分性][合并] AUC(原始余弦)={auc:.3}（正 {np} / 负 {nn}）");
        }

        // ---------- 查询间对照：关联查询池内 Top-1 vs 无关查询池内 Top-1 ----------
        // 判据 H2（无关侧零放行）能否靠绝对阈值满足，取决于这两组 Top-1 是否可分。
        let mut cross: Vec<(f32, bool)> = Vec::new();
        cross.extend(top1_related.iter().map(|s| (*s, true)));
        cross.extend(top1_unrelated.iter().map(|s| (*s, false)));
        let ovl_cross = p82n_ovl(&top1_related, &top1_unrelated, 10);
        let rel_min = top1_related.iter().copied().fold(f32::INFINITY, f32::min);
        let unrel_max = top1_unrelated
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        eprintln!(
            "[P8.2n+][可分性][查询间] 关联 Top-1 n={} 均值={:.3} 中位={:.3} 最小={:.3}；无关 Top-1 n={} 均值={:.3} 中位={:.3} 最大={:.3}",
            top1_related.len(),
            mean(&top1_related),
            median(&top1_related),
            rel_min,
            top1_unrelated.len(),
            mean(&top1_unrelated),
            median(&top1_unrelated),
            unrel_max
        );
        eprintln!(
            "[P8.2n+][可分性][查询间] OVL(10箱)={ovl_cross:.3}；关联最小 Top-1={rel_min:.3} vs 无关最大 Top-1={unrel_max:.3}（若前者 <= 后者则不存在可行绝对阈值）"
        );
        if let Some((auc, np, nn)) = p82n_auc(&cross) {
            eprintln!(
                "[P8.2n+][可分性][查询间] AUC(关联 vs 无关 Top-1)={auc:.3}（正 {np} / 负 {nn}）"
            );
        }

        // ---------- 关键补测：H1 口径的可行绝对阈值区间 ----------
        // H1（16 条 MISS 救回 ≥10）要求 MISS 的 gold 分越过阈值 t；
        // H2（无关侧零放行）要求 t 严格大于无关侧池内 Top-1 上界。
        // 两者同时可满足 ⇔ 存在 t 使 ≥10 条 miss_top1 ≥ t > unrel_top1_max。
        let unrel_top1_max = top1_unrelated
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        let mut miss_sorted: Vec<f32> = miss_top1.clone();
        miss_sorted.sort_by(|a, b| b.total_cmp(a));
        let miss_10th = miss_sorted.get(9).copied();
        let feasible = miss_10th.is_some_and(|m| m > unrel_top1_max);
        eprintln!(
            "[P8.2n+][可分性][阈值可行性] MISS gold 分 n={} 降序前 12={:?}",
            miss_sorted.len(),
            miss_sorted.iter().take(12).copied().collect::<Vec<_>>()
        );
        eprintln!(
            "[P8.2n+][可分性][阈值可行性] MISS 第 10 高={:?} vs 无关池内 Top-1 上界={:.4} ⇒ 可行阈值区间{}",
            miss_10th,
            unrel_top1_max,
            if feasible {
                "非空（H1/H2 可同时满足）"
            } else {
                "为空（极值交叉，绝对阈值不可能同时满足 H1 与 H2）"
            }
        );

        // ---------- 预注册判读（不做事后调参）----------
        let within_mean = mean(&within_auc_dec);
        let within_sep = within_mean >= SEPARABLE_AUC;
        let within_insep = within_mean <= INSEPARABLE_AUC;
        let cross_auc = p82n_auc(&cross).map(|(a, _, _)| a).unwrap_or(f32::NAN);
        let cross_insep = cross_auc <= INSEPARABLE_AUC;
        // 判读改为"排序可分性（AUC）× 范围可分性（可行阈值区间）"二维：
        // 仅当两者皆成立，绝对阈值族判据才可能同时满足 H1 与 H2。
        let verdict = if within_sep && feasible {
            "两层排序可分且可行阈值区间非空 ⇒ 绝对阈值族判据在原理上可同时满足 H1/H2，需回到判据形式/阈值寻址"
        } else if within_sep && !feasible {
            "排序可分（AUC 高）但极值交叉 ⇒ 可行阈值区间为空：**排序可分不等于阈值可行**，\
             根因=跨查询尺度不可比（池内相对排序族无空间，须状态化或池组成重构）"
        } else if within_insep && cross_insep {
            "两层皆不可分 ⇒ 根因=池组成语义混杂，只能走状态化或池组成重构"
        } else {
            "混合信号 ⇒ 需结合逐查询池内细节定位（与 H1 否证方向一致）"
        };
        eprintln!(
            "[P8.2n+][可分性][判读] 查询内 AUC 均值={within_mean:.3}（阈值 >=0.75 可分 / <=0.60 不可分）；查询间 AUC={cross_auc:.3}；可行阈值区间非空={feasible}；结论：{verdict}"
        );
    }

    /// P8.2o 状态化压制方案可行性评估探针（承接文档 §10.17.9 锁定项 2，纯观测零回归）。
    ///
    /// **问题**：§10.17 已否证三种"无状态代理"（去中心化空间 κ、原始空间 τ、模长 L2）；
    /// §10.18 进一步证明绝对阈值族无空间（极值交叉）。路线仅剩"状态化方案"——
    /// 即为 `Memory` 增加**跨查询命中/访问统计**字段（真实跨会话频次），用频次折扣
    /// 压制"语义吸铁石"。本探针在**不改动任何数据结构**的前提下，以进程内 `BTreeMap`
    /// 等价模拟该持久化字段，先回答**算法价值问题**：频次信息能否同时改善 H1 与 H2？
    /// （工程代价评估为文档 §10.19 的静态部分，不含在代码内）
    ///
    /// **口径设计（避免循环论证）**：真实系统的压制只能用**历史**频次。故探针采用
    /// **留一法（leave-one-out）**：压制查询 q 时，频次取"其余 n−1 个查询"的累计
    /// 命中次数，排除 q 自身贡献。否则用全量频次会把当前查询的 Top-1 直接压掉，
    /// H2 改善沦为自证。
    ///
    /// **命中频次定义**：某条目在其余查询的**候选池内出现的次数**（跨查询文档频率 df），
    /// 与锁定项 2 措辞"跨查询命中统计"精确对应，且无需定义并列规则。折扣形式
    /// `s' = s − λ·留一命中率`（命中率量纲 0-1，与去中心化余弦同尺度；不额外 clamp，
    /// 以免截断行为对 H2 产生人为偏利）。
    ///
    /// **核心风险假设**：霸榜者（如「孩子下周三期末考…」）本身是某些查询的 gold
    /// （exam 链），压制它必然连带压制那些查询的 gold ⇒ H1 恶化。探针以 λ 扫描给出
    /// (H1, H2) 联合曲线，并附**频次判别力 AUC**（无关 Top-1 的留一频次 vs
    /// MISS gold 的留一频次，期望前者高、后者低）——AUC≈0.5 即频次无判别力。
    #[test]
    #[cfg(feature = "ml")]
    fn test_assoc_p82o_frequency_suppress_probe() {
        use std::collections::BTreeMap;
        use std::time::{SystemTime, UNIX_EPOCH};

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_p82o_freq_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let Some(mut store) = new_ml_capable_store(dir.to_str().unwrap()) else {
            eprintln!(
                "[P8.2o][频次压制] ML 编码器不可用（bge 权重缺失或未设 LRC_LUOSHU_MODEL_ID），跳过状态化可行性探针"
            );
            return;
        };
        p82h_seed(&mut store);

        /// 单查询池内缓存（第一遍采集；λ 扫描阶段纯数值重算，免重复编码）
        struct P82oQueryCache {
            is_related: bool,
            is_miss: bool,
            /// (候选内容, 去中心化余弦, 是否属查询 gold 链)
            hits: Vec<(String, f32, bool)>,
        }

        /// 池内 argmax（可选仅看 gold 候选）
        fn p82o_argmax(c: &P82oQueryCache, only_gold: bool) -> Option<&(String, f32, bool)> {
            c.hits
                .iter()
                .filter(|(_, _, g)| !only_gold || *g)
                .max_by(|a, b| a.1.total_cmp(&b.1))
        }

        let miss_set: std::collections::HashSet<&str> = P82H_DOC_MISS.iter().copied().collect();
        let mut caches: Vec<P82oQueryCache> = Vec::new();

        // ---------- 第一遍：逐查询采集池内 (内容, 去中心化余弦, 是否 gold) ----------
        // 口径与 §10.18 探针逐字一致（p82m_root_pool + 去中心化余弦），保证 λ=0 行
        // 可直接与 §10.18 的基线数值对照。压制阶段纯数值重算，不重复编码。
        for &(gold, query) in P82H_QUERIES {
            let pool = p82m_root_pool(&mut store, query);
            if pool.is_empty() {
                continue;
            }
            let refs: Vec<&crate::memory_types::Memory> = pool.iter().collect();
            let dec = store.semantic_similarities_debiased(query, &refs, assoc_suppress_lambda());
            let mut hits: Vec<(String, f32, bool)> = Vec::new();
            for (i, memory) in pool.iter().enumerate() {
                if let Some(s) = dec[i] {
                    let is_gold = p82h_classify(&memory.content).0 == gold;
                    hits.push((memory.content.clone(), s, is_gold));
                }
            }
            caches.push(P82oQueryCache {
                is_related: true,
                is_miss: miss_set.contains(query),
                hits,
            });
        }
        for &query in P82H_UNRELATED {
            let pool = p82m_root_pool(&mut store, query);
            if pool.is_empty() {
                continue;
            }
            let refs: Vec<&crate::memory_types::Memory> = pool.iter().collect();
            let dec = store.semantic_similarities_debiased(query, &refs, assoc_suppress_lambda());
            let mut hits: Vec<(String, f32, bool)> = Vec::new();
            for (i, memory) in pool.iter().enumerate() {
                if let Some(s) = dec[i] {
                    // 无关查询无 gold 链（H2 只看"是否有候选越过阈值"），故一律标 false
                    hits.push((memory.content.clone(), s, false));
                }
            }
            caches.push(P82oQueryCache {
                is_related: false,
                is_miss: false,
                hits,
            });
        }

        // ---------- 跨查询文档频率 df（按"出现的查询数"计数，池内不重复） ----------
        let mut df: BTreeMap<String, usize> = BTreeMap::new();
        for c in caches.iter() {
            for (content, _, _) in c.hits.iter() {
                *df.entry(content.clone()).or_insert(0) += 1;
            }
        }
        let n_queries = caches.len();
        let n_miss = caches.iter().filter(|c| c.is_miss).count();
        let n_unrel = caches.iter().filter(|c| !c.is_related).count();

        // 留一命中率：rate(c|q) = (df(c) − 1_{c ∈ q 池}) / (n_queries − 1)
        // 分母取全体查询数 −1，使"跨查询"语义完整（含无关查询的池），且排除 q 自身贡献。
        let others = n_queries.saturating_sub(1).max(1);
        let l1_rate = |q: &P82oQueryCache, content: &str| -> f32 {
            let total = *df.get(content).unwrap_or(&0);
            let self_hit = usize::from(q.hits.iter().any(|(c, _, _)| c == content));
            total.saturating_sub(self_hit) as f32 / others as f32
        };

        // 集中度诊断：df 最高的条目即"跨查询语义吸铁石"候选
        let mut df_sorted: Vec<(String, usize)> = df.iter().map(|(k, v)| (k.clone(), *v)).collect();
        df_sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let df_max = df_sorted.first().map(|(_, v)| *v).unwrap_or(0);
        let df_ge_mean: f32 = if df.is_empty() {
            0.0
        } else {
            df.values().sum::<usize>() as f32 / df.len() as f32
        };
        eprintln!(
            "[P8.2o][频次] 池内候选去重后 {} 条，覆盖查询 {}（关联 {} / 无关 {}，其中 MISS {}）；\
             池内 df 均值={df_ge_mean:.2} 最大={df_max}",
            df.len(),
            n_queries,
            n_queries - n_unrel,
            n_unrel,
            n_miss
        );
        eprintln!("[P8.2o][频次][吸铁石候选] 池内 df Top-5（跨查询出现次数）：");
        for (content, freq) in df_sorted.iter().take(5) {
            eprintln!("  · df={freq}/{n_queries}  {content}");
        }

        // ---------- 频次判别力 AUC（无状态代理 τ 的对照项） ----------
        // 正类 = 无关查询池内 Top-1 的留一命中率（期望高：吸铁石）；
        // 负类 = MISS 查询池内 gold 最高分条目的留一命中率（期望低：真 gold 应专指）。
        // AUC 由 p82n_auc 的 Mann-Whitney 口径给出；≈0.5 即频次不携带类别信息。
        let mut pos_freq: Vec<f32> = Vec::new();
        let mut neg_freq: Vec<f32> = Vec::new();
        for c in caches.iter() {
            if c.is_related {
                if !c.is_miss {
                    continue;
                }
                if let Some((content, _, _)) = p82o_argmax(c, true) {
                    neg_freq.push(l1_rate(c, content));
                }
            } else if let Some((content, _, _)) = p82o_argmax(c, false) {
                pos_freq.push(l1_rate(c, content));
            }
        }
        let mut freq_samples: Vec<(f32, bool)> = Vec::new();
        freq_samples.extend(pos_freq.iter().map(|v| (*v, true)));
        freq_samples.extend(neg_freq.iter().map(|v| (*v, false)));
        let freq_auc = p82n_auc(&freq_samples);
        let freq_ovl = p82n_ovl(&pos_freq, &neg_freq, 10);
        eprintln!(
            "[P8.2o][频次][判别力] 留一命中率：无关 Top-1 均值={:.4}（n={}） vs MISS gold 均值={:.4}（n={}）；\
             OVL(10箱)={freq_ovl:.3}",
            {
                let s: f32 = pos_freq.iter().sum();
                if pos_freq.is_empty() { 0.0 } else { s / pos_freq.len() as f32 }
            },
            pos_freq.len(),
            {
                let s: f32 = neg_freq.iter().sum();
                if neg_freq.is_empty() { 0.0 } else { s / neg_freq.len() as f32 }
            },
            neg_freq.len()
        );
        if let Some((auc, np, nn)) = freq_auc {
            eprintln!(
                "[P8.2o][频次][判别力] AUC(无关Top-1 高频 vs MISS-gold 低频)={auc:.3}（正 {np} / 负 {nn}；≈0.5 即无判别力）"
            );
        } else {
            eprintln!("[P8.2o][频次][判别力] AUC 不可计算（某一侧样本为空）");
        }

        // ---------- λ 扫描：(H1 救回, H2 误放行) 联合曲线 ----------
        // 阈值固定为去中心化空间标定值 0.18（§10.18/P8.2k 同源），避免"压制改变量纲后
        // 再动阈值"引入额外自由度；λ=0 行即当前 P8.2m 基线。
        const P82O_THRESHOLD: f32 = 0.18;
        let lambdas: [f32; 6] = [0.0, 0.25, 0.5, 1.0, 2.0, 4.0];
        let mut curve: Vec<(f32, usize, usize)> = Vec::new();
        for &lam in lambdas.iter() {
            let mut h1_saved = 0usize;
            let mut h2_pass = 0usize;
            for c in caches.iter() {
                if c.is_related {
                    if !c.is_miss {
                        continue;
                    }
                    // H1 口径同 §10.18：MISS 关心 gold 候选本身能否越过阈值
                    let best = c
                        .hits
                        .iter()
                        .filter(|(_, _, g)| *g)
                        .map(|(content, s, _)| *s - lam * l1_rate(c, content))
                        .reduce(f32::max);
                    if best.is_some_and(|v| v >= P82O_THRESHOLD) {
                        h1_saved += 1;
                    }
                } else {
                    let top1 = c
                        .hits
                        .iter()
                        .map(|(content, s, _)| *s - lam * l1_rate(c, content))
                        .reduce(f32::max);
                    if top1.is_some_and(|v| v >= P82O_THRESHOLD) {
                        h2_pass += 1;
                    }
                }
            }
            curve.push((lam, h1_saved, h2_pass));
        }
        eprintln!(
            "[P8.2o][压制][λ扫描] 阈值 t={P82O_THRESHOLD}（去中心化空间）；λ=0 行应与 §10.18 基线一致："
        );
        for (lam, h1, h2) in curve.iter() {
            eprintln!(
                "  · λ={lam:<4} H1 救回 {h1}/{n_miss}（阈值 ≥10）  H2 无关放行 {h2}/{n_unrel}（阈值 =0）"
            );
        }

        // ---------- 判读：算法价值是否存在 ----------
        // 有价值 ⇔ 存在 λ 使 H2 放行 0 且 H1 ≥10（两者同时成立，即无需任何结构变更
        // 也能扩大可行域）；若无 λ 能达到，则状态化路线在"算法层"即被否证。
        let feasible = curve
            .iter()
            .filter(|(_, _, h2)| *h2 == 0)
            .map(|(lam, h1, _)| (*lam, *h1))
            .max_by(|a, b| a.1.cmp(&b.1));
        let base_h1 = curve.first().map(|(_, h1, _)| *h1).unwrap_or(0);
        let last_h1 = curve.last().map(|(_, h1, _)| *h1).unwrap_or(0);
        let verdict = match feasible {
            Some((lam, h1)) if h1 >= 10 => format!(
                "λ={lam} 时 H2 放行 0 且 H1 救回 {h1}/{n_miss} ⇒ 频次压制可同时满足 H1/H2，\
                 状态化方案具算法价值，建议进入工程代价评审"
            ),
            Some((lam, h1)) => format!(
                "存在 λ={lam} 使 H2 放行 0，但 H1 仅 {h1}/{n_miss}（基线 {base_h1}）⇒ 频次压制\
                 能改善 H2 却无法使 H1 达标，状态化方案算法价值不足"
            ),
            None if last_h1 >= base_h1 => format!(
                "无任何 λ 能使无关侧零放行（H2 不可达）；但 H1 由 {base_h1} 增至 {last_h1} ⇒ \
                 压制方向与 gold 一致，瓶颈在频次不可分而非压制形式"
            ),
            None => format!(
                "无任何 λ 能使无关侧零放行（H2 不可达），且 H1 由 {base_h1} 恶化至 {last_h1} ⇒ \
                 压制连带杀伤 gold，状态化方案在算法层被否证"
            ),
        };
        let auc_txt = freq_auc
            .map(|(a, _, _)| format!("{a:.3}"))
            .unwrap_or_else(|| "N/A".to_string());
        eprintln!(
            "[P8.2o][压制][判读] 频次判别力 AUC={auc_txt}；H1 基线(λ=0)={base_h1}/{n_miss} → 最大 λ 时 {last_h1}/{n_miss}；结论：{verdict}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P8.2k 分位数（最近秩法：升序第 ceil(q×n) 个，取 1-based 秩）
    #[cfg(feature = "ml")]
    fn p82k_percentile(samples: &mut [f64], q: f64) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = samples.len();
        let rank = ((q * n as f64).ceil() as usize).clamp(1, n);
        samples[rank - 1]
    }

    /// /associations/explore 不触碰代码库，此桩仅满足 build_v1_router 的构造签名
    /// （刻意不 panic：HTTP 抽样链路里任何代码库调用都属意外，应显式暴露为 0 结果）。
    #[cfg(feature = "ml")]
    struct IdleCodebase;

    #[cfg(feature = "ml")]
    impl IndexedCodebase for IdleCodebase {
        fn search(&self, query: &str, _top_k: usize) -> crate::RetrievalResult {
            crate::RetrievalResult {
                query: query.to_string(),
                returned: 0,
                total_indexed: 0,
                results: Vec::new(),
            }
        }

        fn multi_keyword_search(
            &self,
            _keywords: &[String],
            _top_k: usize,
        ) -> crate::RetrievalResult {
            crate::RetrievalResult {
                query: String::new(),
                returned: 0,
                total_indexed: 0,
                results: Vec::new(),
            }
        }

        fn get_stats(&self) -> crate::ChunkStats {
            crate::ChunkStats {
                file_count: 0,
                total_chunks: 0,
                type_counts: std::collections::HashMap::new(),
                language_counts: std::collections::HashMap::new(),
                avg_lines: 0.0,
            }
        }

        fn recent_chunks(&self, _top_k: usize) -> crate::RetrievalResult {
            crate::RetrievalResult {
                query: String::new(),
                returned: 0,
                total_indexed: 0,
                results: Vec::new(),
            }
        }
    }

    /// P8.2k 在线链路时延实测（§10.13 锁定项 1）：把语义旁路放回完整 BFS 多跳
    /// 路径，测端到端分位数（P50/P95/max）而非单点均值。
    ///
    /// 立项口径（用户确认）：30 查询 × 5 轮 = 150 次调用/臂；A/B 双臂同测——
    /// - A 臂（统计编码器）：旁路必然 unavailable，即"词面通路"成本基线；
    /// - B 臂（ML 编码器 bge）：旁路真实参与，即"旁路净增量"。
    ///
    /// 为什么必须分位数 + 双峰分解：旁路只在 root 起点门禁零命中时惰性触发一次
    /// （BFS 扩散循环内不再触发），单查询时延因此呈双峰；把两峰混成一个均值会
    /// 重演 P7.4 的误读教训。故按响应 `semantic_bypass` 归组统计，并单列
    /// `unavailable` 计数以证明 A 臂旁路确实缺席（而非未触发）。
    #[test]
    #[cfg(feature = "ml")]
    fn test_assoc_p82k_explore_latency_p95() {
        use std::collections::BTreeMap;
        use std::time::{Instant, SystemTime, UNIX_EPOCH};

        /// 采样轮数（立项口径 5 轮）
        const ROUNDS: usize = 5;
        /// 单次探索的总预算（对齐 run_association_explore 内 ASSOCIATION_EXPLORE_TIME_BUDGET）
        const EXPLORE_BUDGET_SECS: f64 = 10.0;
        /// 外层 HTTP 超时（对齐 explore handler 的 Duration::from_secs(15)）
        const HTTP_TIMEOUT_SECS: f64 = 15.0;

        /// 单臂采样结果
        struct ArmSample {
            name: &'static str,
            /// 逐轮 P50（秒）——用于暴露首轮冷启动效应
            per_round_p50: Vec<f64>,
            /// 全部调用：(单次耗时秒, semantic_bypass)
            all: Vec<(f64, String)>,
        }

        /// 跑完一条臂的 ROUNDS 轮 × 30 查询，逐次计时
        fn sample_arm(
            name: &'static str,
            store: &mut MemoryStore<JsonPersistence>,
            rounds: usize,
        ) -> ArmSample {
            use std::sync::atomic::AtomicBool;
            let mut per_round_p50 = Vec::with_capacity(rounds);
            let mut all: Vec<(f64, String)> = Vec::with_capacity(P82H_QUERIES.len() * rounds);
            for _round in 0..rounds {
                let mut round_lat: Vec<f64> = Vec::with_capacity(P82H_QUERIES.len());
                for &(_gold, query) in P82H_QUERIES {
                    let cancel = AtomicBool::new(false);
                    let started = Instant::now();
                    let resp =
                        run_association_explore(store, Some(query), None, 4, 3, &cancel, None);
                    let elapsed = started.elapsed().as_secs_f64();
                    round_lat.push(elapsed);
                    all.push((elapsed, resp.semantic_bypass.clone()));
                }
                per_round_p50.push(p82k_percentile(&mut round_lat, 0.50));
            }
            ArmSample {
                name,
                per_round_p50,
                all,
            }
        }

        /// 打印一条臂的分位数报告（整体 + 双峰分解）
        fn report(arm: &ArmSample, budget: f64) {
            let mut lat: Vec<f64> = arm.all.iter().map(|(secs, _)| *secs).collect();
            let p50 = p82k_percentile(&mut lat, 0.50);
            let p95 = p82k_percentile(&mut lat, 0.95);
            let max = lat.last().copied().unwrap_or(0.0);
            let mut dist: BTreeMap<String, usize> = BTreeMap::new();
            for (_, bypass) in &arm.all {
                *dist.entry(bypass.clone()).or_insert(0) += 1;
            }
            eprintln!(
                "[P8.2k][时延·{}] n={} P50={:.3}s P95={:.3}s max={:.3}s（P95 占 {:.0}s 预算 {:.1}%）",
                arm.name,
                arm.all.len(),
                p50,
                p95,
                max,
                budget,
                p95 / budget * 100.0
            );
            eprintln!(
                "[P8.2k][时延·{}] 逐轮 P50（ms）：{:?}",
                arm.name,
                arm.per_round_p50
                    .iter()
                    .map(|secs| (secs * 1000.0).round() as u64)
                    .collect::<Vec<u64>>()
            );
            eprintln!("[P8.2k][时延·{}] 旁路分布：{:?}", arm.name, dist);

            let mut applied: Vec<f64> = arm
                .all
                .iter()
                .filter(|(_, bypass)| bypass == "applied")
                .map(|(secs, _)| *secs)
                .collect();
            let mut non_applied: Vec<f64> = arm
                .all
                .iter()
                .filter(|(_, bypass)| bypass != "applied")
                .map(|(secs, _)| *secs)
                .collect();
            if !applied.is_empty() {
                let a50 = p82k_percentile(&mut applied, 0.50);
                let a95 = p82k_percentile(&mut applied, 0.95);
                eprintln!(
                    "[P8.2k][时延·{}] applied 峰 n={} P50={:.3}s P95={:.3}s max={:.3}s",
                    arm.name,
                    applied.len(),
                    a50,
                    a95,
                    applied.last().copied().unwrap_or(0.0)
                );
            }
            if !non_applied.is_empty() {
                let n50 = p82k_percentile(&mut non_applied, 0.50);
                let n95 = p82k_percentile(&mut non_applied, 0.95);
                eprintln!(
                    "[P8.2k][时延·{}] 非 applied 峰 n={} P50={:.3}s P95={:.3}s max={:.3}s",
                    arm.name,
                    non_applied.len(),
                    n50,
                    n95,
                    non_applied.last().copied().unwrap_or(0.0)
                );
            }
        }

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();

        // ---------- A 臂：统计编码器（旁路缺席，代表词面通路基线）----------
        let dir_a = std::env::temp_dir().join(format!("lrc_p82k_lat_a_{ts}"));
        let _ = std::fs::remove_dir_all(&dir_a);
        std::fs::create_dir_all(&dir_a).unwrap();
        let dir_a_str = dir_a.to_str().unwrap().to_string();
        let mut store_a = new_statistical_store(&dir_a_str);
        p82h_seed(&mut store_a);
        let arm_a = sample_arm("A·统计", &mut store_a, ROUNDS);

        // ---------- B 臂：ML 编码器（bge 真实参与旁路）----------
        let dir_b = std::env::temp_dir().join(format!("lrc_p82k_lat_b_{ts}"));
        let _ = std::fs::remove_dir_all(&dir_b);
        std::fs::create_dir_all(&dir_b).unwrap();
        let dir_b_str = dir_b.to_str().unwrap().to_string();
        let Some(mut store_b) = new_ml_capable_store(&dir_b_str) else {
            eprintln!(
                "[P8.2k][时延] ML 编码器不可用（bge 权重缺失或环境未设 LRC_LUOSHU_MODEL_ID）\
                 ——本用例仅在 ml 环境执行；请设置 LRC_LUOSHU_MODEL_ID=BAAI/bge-base-zh"
            );
            return;
        };
        p82h_seed(&mut store_b);
        let arm_b = sample_arm("B·ml  ", &mut store_b, ROUNDS);

        // ---------- 分位数报告 ----------
        report(&arm_a, EXPLORE_BUDGET_SECS);
        report(&arm_b, EXPLORE_BUDGET_SECS);

        // ---------- 旁路净增量：两臂"旁路触发峰"的 P50 之差 ----------
        let mut a_trig: Vec<f64> = arm_a
            .all
            .iter()
            .filter(|(_, bypass)| bypass != "unused")
            .map(|(secs, _)| *secs)
            .collect();
        let mut b_trig: Vec<f64> = arm_b
            .all
            .iter()
            .filter(|(_, bypass)| bypass != "unused")
            .map(|(secs, _)| *secs)
            .collect();
        if !a_trig.is_empty() && !b_trig.is_empty() {
            let a_p50 = p82k_percentile(&mut a_trig, 0.50);
            let b_p50 = p82k_percentile(&mut b_trig, 0.50);
            eprintln!(
                "[P8.2k][净增量] 旁路触发峰 P50：A(统计·unavailable)={:.3}s（n={}）→ B(ml·applied)={:.3}s（n={}）；\
                 单查询净增量 {:.3}s",
                a_p50,
                a_trig.len(),
                b_p50,
                b_trig.len(),
                b_p50 - a_p50
            );
        }

        // ---------- 不变量断言（与机器绝对性能无关的可确证契约）----------
        assert_eq!(
            arm_a.all.len(),
            P82H_QUERIES.len() * ROUNDS,
            "A 臂采样数应为 30×5"
        );
        assert_eq!(
            arm_b.all.len(),
            P82H_QUERIES.len() * ROUNDS,
            "B 臂采样数应为 30×5"
        );
        let stat_applied = arm_a
            .all
            .iter()
            .filter(|(_, bypass)| bypass == "applied")
            .count();
        let stat_unavailable = arm_a
            .all
            .iter()
            .filter(|(_, bypass)| bypass == "unavailable")
            .count();
        assert_eq!(
            stat_applied, 0,
            "A 臂为统计编码器，语义旁路不可能 applied（实测 {stat_applied}）"
        );
        assert!(
            stat_unavailable > 0,
            "A 臂应出现 unavailable——证明旁路被触发但编码器缺席（≠未触发）；实测 {stat_unavailable}"
        );
        let ml_applied = arm_b
            .all
            .iter()
            .filter(|(_, bypass)| bypass == "applied")
            .count();
        assert!(
            ml_applied > 0,
            "B 臂 ml 编码器必须在弱匹配查询上真实参与旁路（applied）；实测 {ml_applied}"
        );
        // 在线链路可用性底线：任何单次探索都必须在外层 HTTP 15s 超时内返回
        let max_a = arm_a
            .all
            .iter()
            .map(|(secs, _)| *secs)
            .fold(0.0f64, f64::max);
        let max_b = arm_b
            .all
            .iter()
            .map(|(secs, _)| *secs)
            .fold(0.0f64, f64::max);
        assert!(
            max_a < HTTP_TIMEOUT_SECS && max_b < HTTP_TIMEOUT_SECS,
            "单次探索必须在外层 {HTTP_TIMEOUT_SECS}s HTTP 超时内返回：A max={max_a:.3}s B max={max_b:.3}s"
        );
    }

    /// P8.2k HTTP 层端到端抽样对照（§10.13 锁定项 1 的 B 抽样）：同一 ML 记忆库、
    /// 同一批查询，比较「函数直接调用」与「经 build_v1_router 的完整 HTTP 链路
    /// （axum 路由 + spawn_blocking + JSON 编解码）」的 P50，量化框架开销占比。
    ///
    /// 抽样量刻意远小于函数层（5 查询 × 2 轮 = 10 次/层）：HTTP 层只为给函数层
    /// 主力数据做"框架开销上限"的旁证，不重复 BFS 全量成本。
    #[tokio::test]
    #[cfg(feature = "ml")]
    async fn test_assoc_p82k_http_latency_overhead() {
        use axum::body::{to_bytes, Body};
        use axum::http::{header, Request};
        use std::sync::atomic::AtomicBool as StdAtomicBool;
        use std::time::{Instant, SystemTime, UNIX_EPOCH};
        use tower::ServiceExt;

        /// 抽样查询数（取 P82H_QUERIES 前 N 条）
        const SAMPLE_QUERIES: usize = 5;
        /// 抽样轮数
        const SAMPLE_ROUNDS: usize = 2;

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_p82k_http_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();

        let mut store = match new_ml_capable_store(&dir_str) {
            Some(store) => store,
            None => {
                eprintln!(
                    "[P8.2k][HTTP] ML 编码器不可用（bge 权重缺失或环境未设 LRC_LUOSHU_MODEL_ID）\
                     ——本用例仅在 ml 环境执行；请设置 LRC_LUOSHU_MODEL_ID=BAAI/bge-base-zh"
                );
                return;
            }
        };
        p82h_seed(&mut store);

        let queries: Vec<&'static str> = P82H_QUERIES
            .iter()
            .take(SAMPLE_QUERIES)
            .map(|(_gold, query)| *query)
            .collect();

        // ---------- 函数层基准 ----------
        let mut fn_lat: Vec<f64> = Vec::with_capacity(SAMPLE_QUERIES * SAMPLE_ROUNDS);
        let mut fn_root: Vec<String> = Vec::with_capacity(SAMPLE_QUERIES * SAMPLE_ROUNDS);
        for _round in 0..SAMPLE_ROUNDS {
            for &query in &queries {
                let cancel = StdAtomicBool::new(false);
                let started = Instant::now();
                let resp =
                    run_association_explore(&mut store, Some(query), None, 4, 3, &cancel, None);
                fn_lat.push(started.elapsed().as_secs_f64());
                // P8.2o：两臂各自独立建库（防跨查询状态串扰），记忆 UUID
                // 必然不同，故以**起点内容**为对照口径而非 root 记忆 ID。
                fn_root.push(
                    resp.nodes
                        .iter()
                        .find(|node| node.depth == 0)
                        .map(|node| node.content.clone())
                        .unwrap_or_default(),
                );
            }
        }

        // ---------- HTTP 层：与生产链路同构（build_v1_router → Service → oneshot）----------
        // P8.2o：压制判定依赖**跨查询累计状态**（`assoc_frequency` 随每次探索
        // 递增），因此两臂必须在**各自全新的 store** 上跑同样的查询序列，
        // 才能保证"第 i 次抽样的 root 起点一致"这一不变量。若复用同一 store
        // 续跑，HTTP 臂会继承函数臂已累计的命中频次，同一查询重复执行会压制
        // 其自身此前的 root，导致第 0 次抽样即出现差异（实测 loss）。
        let http_dir = std::env::temp_dir().join(format!("lrc_p82k_http_arm_{ts}"));
        let _ = std::fs::remove_dir_all(&http_dir);
        std::fs::create_dir_all(&http_dir).unwrap();
        // v0.9.7 审查修复（消除同族隐患）：与 test_assoc_p82p_concurrency_pressure
        // 同字面文案的 expect。本处当前**不可达**（上方 L3451 已在模型缺失时 return），
        // 但属同类隐患——若将来上方守卫被移动，此处会重新变成 CI 硬失败点。
        // 统一改为同族跳过守卫，保证"ml 用例在无模型环境一律优雅跳过"这一契约
        // 不依赖"守卫恰好在其上方"这一脆弱假设。
        let Some(mut store_http) = new_ml_capable_store(http_dir.to_str().unwrap()) else {
            eprintln!("[P8.2k][HTTP] HTTP 臂 ML 编码器二次加载失败——本用例仅在 ml 环境执行，跳过");
            return;
        };
        p82h_seed(&mut store_http);
        let shared = Arc::new(Mutex::new(store_http));
        let manager: Arc<Mutex<Box<dyn IndexedCodebase>>> =
            Arc::new(Mutex::new(Box::new(IdleCodebase)));
        let llm_api = Arc::new(RwLock::new(crate::LlmApiConfig::default()));
        let llm_ready = Arc::new(AtomicBool::new(false));
        // 路由内注册路径为 /associations/explore（/v1 前缀由生产侧 nest_service 追加）
        let app = build_v1_router(shared, manager, llm_api, llm_ready, false);

        let mut http_lat: Vec<f64> = Vec::with_capacity(SAMPLE_QUERIES * SAMPLE_ROUNDS);
        let mut http_root: Vec<String> = Vec::with_capacity(SAMPLE_QUERIES * SAMPLE_ROUNDS);
        for _round in 0..SAMPLE_ROUNDS {
            for &query in &queries {
                let payload = serde_json::json!({
                    "query": query,
                    "depth": 4,
                    "width": 3,
                })
                .to_string();
                let request = Request::builder()
                    .method("POST")
                    .uri("/associations/explore")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(payload))
                    .unwrap();
                let started = Instant::now();
                let response = app
                    .clone()
                    .into_service()
                    .oneshot(request)
                    .await
                    .expect("HTTP 层调用失败");
                let status = response.status();
                let bytes = to_bytes(response.into_body(), usize::MAX)
                    .await
                    .expect("读取响应体失败");
                http_lat.push(started.elapsed().as_secs_f64());
                assert_eq!(status, StatusCode::OK, "HTTP 层应返回 200，实测 {status}");
                let body: serde_json::Value =
                    serde_json::from_slice(&bytes).expect("响应体应为合法 JSON");
                assert!(
                    body.get("semantic_bypass").is_some(),
                    "响应必须携带 semantic_bypass 活性字段"
                );
                http_root.push(
                    body.get("nodes")
                        .and_then(|value| value.as_array())
                        .and_then(|nodes| {
                            nodes
                                .iter()
                                .find(|node| node.get("depth").and_then(|d| d.as_u64()) == Some(0))
                        })
                        .and_then(|node| node.get("content"))
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string(),
                );
            }
        }

        // ---------- 对照报告 ----------
        let mut fn_sorted = fn_lat.clone();
        let mut http_sorted = http_lat.clone();
        let fn_p50 = p82k_percentile(&mut fn_sorted, 0.50);
        let http_p50 = p82k_percentile(&mut http_sorted, 0.50);
        let overhead_percent = if fn_p50 > 0.0 {
            (http_p50 / fn_p50 - 1.0) * 100.0
        } else {
            0.0
        };
        eprintln!(
            "[P8.2k][HTTP] 函数层 n={} P50={:.3}s max={:.3}s；HTTP 层 n={} P50={:.3}s max={:.3}s；\
             框架开销 {:.1}%（绝对值 {:.3}s）",
            fn_lat.len(),
            fn_p50,
            fn_sorted.last().copied().unwrap_or(0.0),
            http_lat.len(),
            http_p50,
            http_sorted.last().copied().unwrap_or(0.0),
            overhead_percent,
            http_p50 - fn_p50
        );

        // ---------- 不变量断言 ----------
        assert_eq!(
            fn_lat.len(),
            SAMPLE_QUERIES * SAMPLE_ROUNDS,
            "函数层抽样数应为 5×2"
        );
        assert_eq!(
            http_lat.len(),
            SAMPLE_QUERIES * SAMPLE_ROUNDS,
            "HTTP 层抽样数应为 5×2"
        );
        for (index, (fn_value, http_value)) in fn_root.iter().zip(http_root.iter()).enumerate() {
            assert_eq!(
                fn_value, http_value,
                "第 {index} 次抽样：HTTP 层与函数层 root 起点必须一致（{fn_value} vs {http_value}）"
            );
        }
    }

    /// P8.2p 池规模扩展语料的无关填充条（与六链 30 条查询保持零词面重叠）。
    ///
    /// 仅用于抬高"全库词面扫描"成本，不参与命中关系判定：话题词与考试/宠物/
    /// 旅行/车辆/健康/家居六链完全无关，确保扩展后 root 起点仍由原 106 条语料
    /// 决定——即"唯一变量 = 候选池规模"。
    #[cfg(feature = "ml")]
    fn p82p_filler_memories(scale: usize) -> Vec<crate::memory_types::Memory> {
        use crate::memory_types::{Importance, Memory, MemoryType};
        const TOPICS: [&str; 12] = [
            "木工榫卯",
            "矿石化验",
            "陶笛吹奏",
            "版画拓印",
            "茶道点茶",
            "集邮册整理",
            "帆船索具",
            "天文观测",
            "篆刻边款",
            "织染扎染",
            "皮划艇划桨",
            "气象云图",
        ];
        let base = p82h_all_memories().len();
        let extra = base.saturating_mul(scale.saturating_sub(1));
        let mut out = Vec::with_capacity(extra);
        for index in 0..extra {
            let topic = TOPICS[index % TOPICS.len()];
            let content = format!(
                "{topic}札记第 {} 则：本条为语料规模填充样本（编号 p82p-filler-{index}），\
                 内容与联想精度评测的六条语义链均无关联。",
                index + 1
            );
            out.push(Memory::new(
                content,
                MemoryType::Fact,
                Some("p82p-pool-scale-filler".to_string()),
                vec!["chain:none".to_string(), "hop:9".to_string()],
                Importance::new(3),
                None,
            ));
        }
        out
    }

    /// P8.2p 候选池规模扩展复测（计划文档 §10.17.9 锁定项 4 前半，承接未决风险 6+7）。
    ///
    /// 立项口径原文（§10.14 锁定项 1）：
    /// "将『触发查询 P50 0.723s』置于 N 条并发 × 更大候选池下复测，确认是否仍落在 10s 预算内。"
    ///
    /// 本用例只取"更大候选池"这一半（并发的另一半见
    /// `test_assoc_p82p_concurrency_pressure`）：同一批 30 条触发查询，分别在原池
    /// 106 条与扩展池 ≈1060 条上跑同一口径，测 P50/P95/max 与净增量。
    /// 两臂各自独立建库，防止 `assoc_frequency` 跨查询状态串扰对照。
    ///
    /// 判据性质声明：本用例为**时延实测报表 + 预算契约断言**，不对精度判据
    /// （H1–H4）重复断言——池规模扩展意在压低精度基线，故只逐字复述既有
    /// root 起点口径用于观测，不新增精度判据。
    #[test]
    #[cfg(feature = "ml")]
    fn test_assoc_p82p_pool_scale_latency() {
        use std::time::{Instant, SystemTime, UNIX_EPOCH};

        /// 池规模扩展倍率（106 → 106×10 ≈ 1060 条）
        const POOL_SCALE: usize = 10;
        /// 单次探索内部预算（对齐 run_association_explore 的 ASSOCIATION_EXPLORE_TIME_BUDGET）
        const EXPLORE_BUDGET_SECS: f64 = 10.0;
        /// 外层 HTTP 超时（对齐 explore handler 的 Duration::from_secs(15)）
        const HTTP_TIMEOUT_SECS: f64 = 15.0;

        /// 跑完一条臂的 30 条触发查询，逐次计时（返回 (耗时秒, semantic_bypass)）
        fn sample_arm(store: &mut MemoryStore<JsonPersistence>) -> Vec<(f64, String)> {
            use std::sync::atomic::AtomicBool;
            let mut all: Vec<(f64, String)> = Vec::with_capacity(P82H_QUERIES.len());
            for &(_gold, query) in P82H_QUERIES {
                let cancel = AtomicBool::new(false);
                let started = Instant::now();
                let resp = run_association_explore(store, Some(query), None, 4, 3, &cancel, None);
                all.push((
                    started.elapsed().as_secs_f64(),
                    resp.semantic_bypass.clone(),
                ));
            }
            all
        }

        /// 打印一条臂的分位数报告，返回 (P50, P95, max)
        fn report(name: &str, samples: &[(f64, String)], budget: f64) -> (f64, f64, f64) {
            let mut lat: Vec<f64> = samples.iter().map(|(secs, _)| *secs).collect();
            let p50 = p82k_percentile(&mut lat, 0.50);
            let p95 = p82k_percentile(&mut lat, 0.95);
            let max = lat.last().copied().unwrap_or(0.0);
            let applied = samples
                .iter()
                .filter(|(_, bypass)| bypass == "applied")
                .count();
            let n = samples.len();
            let pct = p95 / budget * 100.0;
            eprintln!(
                "[P8.2p][池规模·{name}] n={n} P50={p50:.3}s P95={p95:.3}s max={max:.3}s\
                 （P95 占 {budget:.0}s 预算 {pct:.1}%）；旁路 applied={applied}"
            );
            (p50, p95, max)
        }

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();

        // ---------- 基准臂：原池 106 条 ----------
        let base_dir = std::env::temp_dir().join(format!("lrc_p82p_pool_base_{ts}"));
        let _ = std::fs::remove_dir_all(&base_dir);
        std::fs::create_dir_all(&base_dir).unwrap();
        let Some(mut store_base) = new_ml_capable_store(base_dir.to_str().unwrap()) else {
            eprintln!(
                "[P8.2p][池规模] ML 编码器不可用（bge 权重缺失或环境未设 LRC_LUOSHU_MODEL_ID）\
                 ——本用例仅在 ml 环境执行；请设置 LRC_LUOSHU_MODEL_ID=BAAI/bge-base-zh"
            );
            return;
        };
        p82h_seed(&mut store_base);

        // ---------- 扩展臂：原池 + 填充语料 ≈ ×POOL_SCALE ----------
        let scaled_dir = std::env::temp_dir().join(format!("lrc_p82p_pool_scaled_{ts}"));
        let _ = std::fs::remove_dir_all(&scaled_dir);
        std::fs::create_dir_all(&scaled_dir).unwrap();
        // v0.9.7 审查修复（消除同族隐患，同 L3501）：当前**不可达**
        // （上方 L3724 已在模型缺失时 return），但属同类隐患——统一改为跳过守卫，
        // 使"ml 用例在无模型环境一律优雅跳过"契约不依赖"守卫恰好在其上方"。
        let Some(mut store_scaled) = new_ml_capable_store(scaled_dir.to_str().unwrap()) else {
            eprintln!("[P8.2p][池规模] 扩展臂 ML 编码器二次加载失败——本用例仅在 ml 环境执行，跳过");
            return;
        };
        p82h_seed(&mut store_scaled);
        let fillers = p82p_filler_memories(POOL_SCALE);
        let filler_count = fillers.len();
        store_scaled
            .remember_batch(fillers)
            .expect("扩展语料批量注入失败");

        let base_samples = sample_arm(&mut store_base);
        let scaled_samples = sample_arm(&mut store_scaled);

        let pool_base = p82h_all_memories().len();
        let pool_scaled = pool_base + filler_count;
        eprintln!(
            "[P8.2p][池规模] 基准池 {pool_base} 条 → 扩展池 {pool_scaled} 条（+{filler_count} 条无关填充）"
        );
        let (b50, b95, bmax) = report("基准池", &base_samples, EXPLORE_BUDGET_SECS);
        let (s50, s95, smax) = report("扩展池", &scaled_samples, EXPLORE_BUDGET_SECS);
        eprintln!(
            "[P8.2p][池规模] 规模 ×{POOL_SCALE} 净增量：P50 {:+.3}s P95 {:+.3}s max {:+.3}s",
            s50 - b50,
            s95 - b95,
            smax - bmax
        );

        // ---------- 预算契约断言 ----------
        assert_eq!(
            base_samples.len(),
            P82H_QUERIES.len(),
            "基准臂采样数应为 30"
        );
        assert_eq!(
            scaled_samples.len(),
            P82H_QUERIES.len(),
            "扩展臂采样数应为 30"
        );
        assert!(
            s95 < EXPLORE_BUDGET_SECS,
            "扩展池下 P95 必须仍落 {EXPLORE_BUDGET_SECS}s 内部预算内：实测 {s95:.3}s"
        );
        assert!(
            smax < HTTP_TIMEOUT_SECS,
            "扩展池下 max 必须仍落 {HTTP_TIMEOUT_SECS}s 外层 HTTP 超时内：实测 {smax:.3}s"
        );

        let _ = std::fs::remove_dir_all(&base_dir);
        let _ = std::fs::remove_dir_all(&scaled_dir);
    }

    /// P8.2p 并发压力复测（计划文档 §10.17.9 锁定项 4 后半，承接未决风险 6）。
    ///
    /// 与 `test_assoc_p82p_pool_scale_latency` 配对：那条量化"更大池"，这条量化
    /// "N 条并发"。以生产同构链路（`build_v1_router` → `oneshot`）发起 N 条并发
    /// POST /associations/explore，逐请求记录状态码与墙钟。
    ///
    /// 为什么预期会看到兜底：`MemoryStore` 含 `RefCell`（持久化缓存）而为 `!Sync`，
    /// 生产 handler 用 `Arc<Mutex<..>>` + `try_lock` 50ms 轮询取锁，故并发探索被
    /// **全局串行化**——后到者在锁外排队，一旦排队超过外层 15s，`timeout` 会置取消
    /// 标志并返回 503 `explore_timeout`。本用例的契约不是"必须全部 200"，而是
    /// **每个请求都必须在有限时间内拿到 200 或 503 的明确应答（无挂死）**。
    ///
    /// 用例分两段：
    /// 1. **并发批**（N=8 真实并发）——量化排队现象，但 200/503 的分布依赖机器性能，
    ///    本机实测 200=8/503=0（未触发兜底）；
    /// 2. **确定性兜底段**——主动在外部独占 `shared` 的 tokio Mutex 直到底层 15s
    ///    `timeout` 触发，**不依赖机器性能**地硬验证 503 `explore_timeout` 兜底路径
    ///    （覆盖 HCSE"异常路径必须覆盖"要求）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    #[cfg(feature = "ml")]
    async fn test_assoc_p82p_concurrency_pressure() {
        use axum::body::{to_bytes, Body};
        use axum::http::{header, Request};
        use std::sync::atomic::AtomicBool as StdAtomicBool;
        use std::time::{Instant, SystemTime, UNIX_EPOCH};
        use tokio::task::JoinSet;
        use tower::ServiceExt;

        /// 单批并发请求数（每请求对应一条互不相同的触发查询）
        const CONCURRENT_REQUESTS: usize = 8;
        /// 池规模扩展倍率（与池规模用例同口径）
        const POOL_SCALE: usize = 10;
        /// 外层 HTTP 超时（对齐 explore handler 的 Duration::from_secs(15)）
        const HTTP_TIMEOUT_SECS: f64 = 15.0;
        /// 超时兜底的调度容差：timeout 触发到状态码返回之间的实测延迟
        const TIMEOUT_TOLERANCE_SECS: f64 = 3.0;

        /// 单请求采样
        struct Sample {
            status: u16,
            secs: f64,
            bypass: Option<String>,
        }

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_p82p_conc_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // v0.9.7 审查修复（P0 CI 平台/环境缺陷）：此前此处为
        //   `.expect("并发臂 ML 编码器加载失败——若失败则并发结论不成立，不得静默跳过")`
        // 其意图是"防止静默跳过掩盖结论"，但实现后果与 CI 契约直接冲突：
        //   CI 的 `cargo test --features server,ml` 运行在**无 bge 模型权重**的环境，
        //   该步骤的既有约定是"ml 用例均带模型缺失即跳过守卫"（同文件其余 7 处
        //   均用 `let Some(..) else { eprintln!; return }`）。本处用 expect 硬失败，
        //   使 CI 必然红——实测 ubuntu 报 `panicked at src/v1_api_tests.rs:3833`
        //   （`option::expect_failed`，前一行日志 `[LRC·洛书ML] 3s 快速检测超时`）。
        // 正确做法：**跳过时必须显式留痕**（eprintln 说明原因与环境要求），
        // 既满足 CI 契约，又不掩盖"本机未验证"这一事实——这与既有守卫一致。
        let Some(mut store) = new_ml_capable_store(dir.to_str().unwrap()) else {
            eprintln!(
                "[P8.2p][并发] ML 编码器不可用（bge 权重缺失或未设 LRC_LUOSHU_MODEL_ID）\
                 ——本用例仅在 ml 环境执行；请设置 LRC_LUOSHU_MODEL_ID=BAAI/bge-base-zh。\
                 注意：本用例结论（并发无挂死）在无模型环境下**未获验证**，须在本地 ml 环境复跑。"
            );
            return;
        };
        p82h_seed(&mut store);
        store
            .remember_batch(p82p_filler_memories(POOL_SCALE))
            .expect("并发臂扩展语料注入失败");

        let shared = Arc::new(Mutex::new(store));
        // 保留一份锁句柄用于第 2 段的确定性兜底验证（build_v1_router 会 move 走 shared）
        let probe_store = shared.clone();
        let manager: Arc<Mutex<Box<dyn IndexedCodebase>>> =
            Arc::new(Mutex::new(Box::new(IdleCodebase)));
        let llm_api = Arc::new(RwLock::new(crate::LlmApiConfig::default()));
        let llm_ready = Arc::new(StdAtomicBool::new(false));
        // 路由内注册路径为 /associations/explore（/v1 前缀由生产侧 nest_service 追加）
        let app = build_v1_router(shared, manager, llm_api, llm_ready, false);

        let queries: Vec<&'static str> = P82H_QUERIES
            .iter()
            .take(CONCURRENT_REQUESTS)
            .map(|(_gold, query)| *query)
            .collect();
        assert_eq!(
            queries.len(),
            CONCURRENT_REQUESTS,
            "触发查询池不足 {CONCURRENT_REQUESTS} 条"
        );

        let wall_started = Instant::now();
        let mut tasks: JoinSet<Sample> = JoinSet::new();
        for &query in &queries {
            let app = app.clone();
            tasks.spawn(async move {
                let payload = serde_json::json!({
                    "query": query,
                    "depth": 4,
                    "width": 3,
                })
                .to_string();
                let request = Request::builder()
                    .method("POST")
                    .uri("/associations/explore")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(payload))
                    .expect("请求构造失败");
                let started = Instant::now();
                let response = app
                    .into_service()
                    .oneshot(request)
                    .await
                    .expect("HTTP 层调用失败");
                let status = response.status().as_u16();
                let bytes = to_bytes(response.into_body(), usize::MAX)
                    .await
                    .expect("读取响应体失败");
                let bypass = serde_json::from_slice::<serde_json::Value>(&bytes)
                    .ok()
                    .and_then(|body| {
                        body.get("semantic_bypass")
                            .and_then(|value| value.as_str())
                            .map(|value| value.to_string())
                    });
                Sample {
                    status,
                    secs: started.elapsed().as_secs_f64(),
                    bypass,
                }
            });
        }

        let mut samples: Vec<Sample> = Vec::with_capacity(CONCURRENT_REQUESTS);
        while let Some(joined) = tasks.join_next().await {
            samples.push(joined.expect("并发任务 panic"));
        }
        let wall = wall_started.elapsed().as_secs_f64();

        // ---------- 并发报表 ----------
        let mut ok: Vec<f64> = samples
            .iter()
            .filter(|sample| sample.status == 200)
            .map(|sample| sample.secs)
            .collect();
        let busy = samples.iter().filter(|sample| sample.status == 503).count();
        let ok_count = ok.len();
        let ok_p50 = p82k_percentile(&mut ok, 0.50);
        let ok_max = ok.last().copied().unwrap_or(0.0);
        let applied = samples
            .iter()
            .filter(|sample| sample.bypass.as_deref() == Some("applied"))
            .count();
        eprintln!(
            "[P8.2p][并发] N={CONCURRENT_REQUESTS} 并发（池 ×{POOL_SCALE}）：200={ok_count} 503={busy}；\
             墙钟 {wall:.3}s；成功请求 P50={ok_p50:.3}s max={ok_max:.3}s；applied={applied}"
        );
        let serial_sum: f64 = samples
            .iter()
            .filter(|sample| sample.status == 200)
            .map(|sample| sample.secs)
            .sum();
        eprintln!(
            "[P8.2p][并发] 串行化证据：成功请求耗时之和 {serial_sum:.3}s vs 墙钟 {wall:.3}s\
             （MemoryStore 为 !Sync，探索被全局 Mutex 串行化；判据：比值 ≈N（={CONCURRENT_REQUESTS}）\
             为真并行，≈(N+1)/2（={:.1}）即锁排队主导）",
            (CONCURRENT_REQUESTS as f64 + 1.0) / 2.0
        );

        // ---------- 韧性契约断言 ----------
        assert_eq!(
            samples.len(),
            CONCURRENT_REQUESTS,
            "并发采样数应为 {CONCURRENT_REQUESTS}"
        );
        for (index, sample) in samples.iter().enumerate() {
            assert!(
                sample.status == 200 || sample.status == 503,
                "第 {index} 个请求状态码须为 200 或 503（忙/超时兜底），实测 {}",
                sample.status
            );
            assert!(
                sample.secs <= HTTP_TIMEOUT_SECS + TIMEOUT_TOLERANCE_SECS,
                "第 {index} 个请求须在 {HTTP_TIMEOUT_SECS}s 外层超时（+{TIMEOUT_TOLERANCE_SECS}s 容差）内返回，\
                 实测 {:.3}s——超出即无兜底（挂死）",
                sample.secs
            );
        }
        assert!(
            ok_count > 0,
            "并发下至少一个请求须成功：全 503 意味着兜底把正常负载也拒了"
        );

        // ---------- 确定性兜底段：外部独占锁 → 硬验证 503 explore_timeout ----------
        // 第 1 段的 200/503 分布依赖机器性能（本机 503=0，兜底路径未被走过）。
        // 这里主动在外部持锁：handler 的 try_lock 50ms 轮询会一直失败，
        // 直到外层 15s timeout 触发并置取消标志 → 必然返回 503 explore_timeout。
        // 该段不依赖机器性能，确定性覆盖 HCSE 要求的"超时/卡死异常路径"。
        let guard = probe_store.lock().await;
        let request = Request::builder()
            .method("POST")
            .uri("/associations/explore")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "query": queries[0],
                    "depth": 4,
                    "width": 3,
                })
                .to_string(),
            ))
            .expect("兜底段请求构造失败");
        let started = Instant::now();
        let response = app
            .clone()
            .into_service()
            .oneshot(request)
            .await
            .expect("兜底段 HTTP 层调用失败");
        let fallback_secs = started.elapsed().as_secs_f64();
        let fallback_status = response.status().as_u16();
        let fallback_body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("兜底段读取响应体失败");
        let fallback_error = serde_json::from_slice::<serde_json::Value>(&fallback_body)
            .ok()
            .and_then(|body| {
                body.get("error")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string())
            })
            .unwrap_or_default();
        drop(guard);

        eprintln!(
            "[P8.2p][并发][兜底] 外部独占锁下：status={fallback_status} error={fallback_error} \
             耗时 {fallback_secs:.3}s（预期 503 explore_timeout，耗时贴近 15s 外层超时）"
        );
        assert_eq!(
            fallback_status, 503,
            "外部独占锁时须返回 503（探索超时兜底），实测 {fallback_status}"
        );
        assert_eq!(
            fallback_error, "explore_timeout",
            "外部独占锁须走 15s 外层超时分支返回 explore_timeout，实测 {fallback_error}"
        );
        assert!(
            (HTTP_TIMEOUT_SECS - 1.0..=HTTP_TIMEOUT_SECS + TIMEOUT_TOLERANCE_SECS)
                .contains(&fallback_secs),
            "兜底段耗时须贴近 {HTTP_TIMEOUT_SECS}s 外层超时（容差 -1.0/+{TIMEOUT_TOLERANCE_SECS}s），\
             实测 {fallback_secs:.3}s"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P7.2：联想边反馈闭环——用户确认 (root→child_b) 边后，门控开启时
    /// explore BFS 使 child_b 前置（稳定排序按边净调整翻转）；门控关闭时
    /// 顺序与无反馈基线逐字节一致（零影响承诺消融对照）。
    ///
    /// 语料设计：root 与 child_a/child_b 词面共享完全相同（平分秋色），
    /// 基线稳定排序按 id 字典序 child_a 在前；确认边 +0.30×0.5 分打破平局，
    /// 使 child_b 前置——证明差异来自边反馈而非词面。
    #[test]
    fn test_assoc_edge_feedback_rerank_in_explore() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_edge_rerank_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        let mut store = MemoryStore::new(JsonPersistence::new(&dir_str).unwrap());

        // root：查询起点记忆（查询与自身完全匹配 → 自身成为起点）
        let root = Memory::new(
            "今晚吃什么好呢".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        store.remember(root.clone()).unwrap();
        // child_a / child_b：与 root 词面共享完全相同（各共享「今晚」「晚吃」
        // 两个 bigram → BM25 同分），但彼此差异大（Jaccard≈0.15 < 合并阈值 0.5，
        // 避免 remember 相似合并吃掉独立 id）；基线同分按写入序 a 在前。
        let child_a = Memory::new(
            "今晚吃啥 家常小炒".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        store.remember(child_a.clone()).unwrap();
        let child_b = Memory::new(
            "今晚吃水饺 北方风味".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        store.remember(child_b.clone()).unwrap();

        let cancel = AtomicBool::new(false);
        let depth1_ids = |resp: &AssociationExploreResponse| -> Vec<String> {
            resp.nodes
                .iter()
                .filter(|n| n.depth == 1)
                .map(|n| n.id.clone())
                .collect()
        };

        // 基线（门控默认关）：child_a 在前
        let base = run_association_explore(
            &mut store,
            Some("今晚吃什么好呢"),
            None,
            2,
            1,
            &cancel,
            None,
        );
        assert!(base.root.is_some(), "root 记忆应成为起点");
        let base_children = depth1_ids(&base);
        assert_eq!(
            base_children,
            vec![child_a.id.clone()],
            "基线 width=1 时应取同分稳定的 child_a: {:?}",
            base_children
        );

        // 记录确认边：root → child_b（用户确认"从今晚吃什么好呢联想到今晚吃面"）
        store.user_feedback.record_association_edge_feedback(
            FeedbackType::Positive,
            &root.id,
            &child_b.id,
            Some("今晚吃什么好呢"),
            None,
        );

        // 门控开启：确认边打破平局，child_b 前置
        std::env::set_var("LRC_ASSOC_EDGE_FEEDBACK", "1");
        let rerank = run_association_explore(
            &mut store,
            Some("今晚吃什么好呢"),
            None,
            2,
            1,
            &cancel,
            None,
        );
        std::env::remove_var("LRC_ASSOC_EDGE_FEEDBACK");
        let rerank_children = depth1_ids(&rerank);
        assert_eq!(
            rerank_children,
            vec![child_b.id.clone()],
            "确认边应使 child_b 前置: baseline={:?} rerank={:?}",
            base_children,
            rerank_children
        );

        // 门控恢复关闭：回到基线顺序（零影响承诺）
        let restore = run_association_explore(
            &mut store,
            Some("今晚吃什么好呢"),
            None,
            2,
            1,
            &cancel,
            None,
        );
        assert_eq!(
            depth1_ids(&restore),
            vec![child_a.id.clone()],
            "门控关闭后应回到基线顺序（零影响）"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P7.3：路径级评分——门控开启时，候选节点分数相对基线发生三档偏移：
    /// 与 root 主题实义词共享 ≥2 的 +TOPIC_BONUS、=1 的无偏移、=0 的
    /// −DRIFT_PENALTY（差分断言，容差 1e-3）；门控关闭/还原后与基线
    /// 逐字节一致（F1 生效 + F2 消融对照）。
    #[test]
    fn test_assoc_path_score_topic_rerank_in_explore() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_path_score_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        let mut store = MemoryStore::new(JsonPersistence::new(&dir_str).unwrap());

        // root：查询起点记忆（查询与自身完全匹配 → 自身成为起点）
        let root = Memory::new(
            "今晚吃什么好呢".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        store.remember(root.clone()).unwrap();
        // mem_a：与 root 共享 2 个实义 bigram（今晚/晚吃）→ 主题强，+BONUS
        let mem_a = Memory::new(
            "今晚吃啥 家常小炒".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        store.remember(mem_a.clone()).unwrap();
        // mem_b：与 root 共享 1 个实义 bigram（今晚）→ 主题中，无偏移
        let mem_b = Memory::new(
            "今晚家常面".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        store.remember(mem_b.clone()).unwrap();
        // mem_c：与 root 内容零共享，但带标签"今晚"通过回归校验（边缘样本）
        // → 主题漂移，−DRIFT_PENALTY
        let mem_c = Memory::new(
            "周末爬山 户外装备清单".to_string(),
            MemoryType::Fact,
            None,
            vec!["今晚".to_string()],
            Importance::new(5),
            None,
        );
        store.remember(mem_c.clone()).unwrap();

        let cancel = AtomicBool::new(false);
        let depth1_score = |resp: &AssociationExploreResponse, id: &str| -> f32 {
            resp.nodes
                .iter()
                .find(|n| n.depth == 1 && n.id == id)
                .map(|n| n.score)
                .unwrap_or(f32::NAN)
        };

        // 基线（门控默认关）：三候选均进入联想链（回归校验后仍保留，width=3）
        let base = run_association_explore(
            &mut store,
            Some("今晚吃什么好呢"),
            None,
            2,
            3,
            &cancel,
            None,
        );
        assert!(base.root.is_some(), "root 记忆应成为起点");
        let sa0 = depth1_score(&base, &mem_a.id);
        let sb0 = depth1_score(&base, &mem_b.id);
        let sc0 = depth1_score(&base, &mem_c.id);
        assert!(
            !sa0.is_nan() && !sb0.is_nan() && !sc0.is_nan(),
            "三候选都应通过回归校验进入联想链: a={sa0:.3} b={sb0:.3} c={sc0:.3}"
        );

        // 门控开启：三档偏移
        std::env::set_var("LRC_ASSOC_PATH_SCORE", "1");
        let on = run_association_explore(
            &mut store,
            Some("今晚吃什么好呢"),
            None,
            2,
            3,
            &cancel,
            None,
        );
        std::env::remove_var("LRC_ASSOC_PATH_SCORE");
        let sa1 = depth1_score(&on, &mem_a.id);
        let sb1 = depth1_score(&on, &mem_b.id);
        let sc1 = depth1_score(&on, &mem_c.id);
        assert!(
            (sa1 - sa0 - ASSOC_PATH_TOPIC_BONUS).abs() < 1e-3,
            "a 偏移应 +{}，实际 {:.4}（{:.4} → {:.4}）",
            ASSOC_PATH_TOPIC_BONUS,
            sa1 - sa0,
            sa0,
            sa1
        );
        assert!(
            (sb1 - sb0).abs() < 1e-3,
            "b 偏移应为 0，实际 {:.4}（{:.4} → {:.4}）",
            sb1 - sb0,
            sb0,
            sb1
        );
        assert!(
            (sc1 - sc0 + ASSOC_PATH_DRIFT_PENALTY).abs() < 1e-3,
            "c 偏移应 −{}，实际 {:.4}（{:.4} → {:.4}）",
            ASSOC_PATH_DRIFT_PENALTY,
            sc1 - sc0,
            sc0,
            sc1
        );

        // 门控还原：回到基线（零影响承诺）
        let restore = run_association_explore(
            &mut store,
            Some("今晚吃什么好呢"),
            None,
            2,
            3,
            &cancel,
            None,
        );
        assert!(
            (depth1_score(&restore, &mem_a.id) - sa0).abs() < 1e-3,
            "门控还原后 a 应回到基线"
        );
        assert!(
            (depth1_score(&restore, &mem_b.id) - sb0).abs() < 1e-3,
            "门控还原后 b 应回到基线"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 开发上下文噪声过滤：非代码起点的联想链不得混入 code_context
    /// 代码块（用户视角的"src/benchmark"噪声）。词面构造保证起点
    /// 稳定通过门禁，两种 feature 配置下行为一致。
    #[test]
    fn test_run_association_explore_skips_code_children_for_life_root() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_code_noise_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        let mut store = new_statistical_store(&dir_str);

        // 起点：与查询共享 周末/去哪/哪儿 多个 token，稳定通过词面门禁
        let root = Memory::new(
            "周末去哪儿玩都可以，我去白云山徒步看风景。".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(6),
            None,
        );
        // 真实发散：与起点共享 白云山/徒步 token
        let real_child = Memory::new(
            "白云山徒步记得带水，南门上山比较轻松。".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        // 代码噪声：内容刻意包含"白云山"以挤进扩散候选，但类型是代码块
        let code_noise = Memory::new(
            "来源：src/benchmark.rs；主题：rs。白云山景区人流量数据统计实现。".to_string(),
            MemoryType::CodeContext,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        for memory in [root, real_child, code_noise] {
            store.remember(memory).unwrap();
        }

        let cancel = AtomicBool::new(false);
        let resp =
            run_association_explore(&mut store, Some("周末去哪儿玩"), None, 2, 2, &cancel, None);

        assert!(resp.root.is_some(), "起点应通过词面实质共鸣门禁");
        assert!(
            resp.nodes
                .iter()
                .all(|node| !node.content.contains("src/benchmark")),
            "代码块不得混入生活起点的联想链: {:?}",
            resp.nodes.iter().map(|n| &n.content).collect::<Vec<_>>()
        );
        assert!(
            resp.nodes
                .iter()
                .any(|node| node.content.contains("白云山徒步记得带水")),
            "真实发散记忆应正常出现"
        );
    }

    /// P3.2-1：导航信号在场时，根候选经 navigated_deep_recall 多视图召回，
    /// 且词面实质共鸣门禁 + CodeContext 过滤仍生效（导航不豁免防线）。
    #[test]
    fn test_run_association_explore_with_navigation_signal() {
        use crate::engine::navigation::NavigationSignal;
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_nav_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        let mut store = new_statistical_store(&dir_str);

        // 生活记忆：词面与"今晚吃什么"实质共鸣（今晚/吃 均重叠）
        let life = Memory::new(
            "今晚吃番茄炒蛋，记得先买番茄".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(7),
            None,
        );
        store.remember(life).unwrap();
        // 代码噪声：含"今晚"但为代码块，任何路径都不应混入
        let code = Memory::new(
            "fn 今晚执行() { run_test_suite(); } // 编译入口".to_string(),
            MemoryType::CodeContext,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        store.remember(code).unwrap();

        // 构造导航信号：艮宫 + 与生活记忆词面一致的探测词"番茄"
        // （探测词是导航"值得探测的方向"，本用例验证导航候选进入门禁）
        let nav = NavigationSignal {
            palaces: vec!["艮".to_string(), "坤".to_string()],
            probes: Some(vec![
                vec!["番茄".to_string(), "吃".to_string()],
                vec!["家".to_string(), "厨房".to_string()],
            ]),
            source_version: Some("daoti-lexicon-v1".to_string()),
        };

        let cancel = AtomicBool::new(false);
        let resp = run_association_explore(
            &mut store,
            Some("今晚吃什么"),
            None,
            2,
            2,
            &cancel,
            Some(&nav),
        );

        // 导航候选 + 词面门禁 → 生活记忆上位为起点
        // 先验证 navigated_deep_recall 本身是否召回了生活记忆（探针，测试失败时可见）
        {
            use crate::memory_store::RecallFilter;
            let nav_filter = RecallFilter::new().with_top_k(ASSOCIATION_ROOT_POOL_TOPK);
            let nav_result = crate::engine::navigation::navigated_deep_recall(
                &mut store,
                "今晚吃什么",
                &nav_filter,
                1,
                &nav,
            );
            eprintln!(
                "[P3.2探针] navigated_deep_recall = {}",
                match &nav_result {
                    Some(r) => format!(
                        "Some({} memories: {:?})",
                        r.memories.len(),
                        r.memories.iter().map(|m| &m.content).collect::<Vec<_>>()
                    ),
                    None => "None".to_string(),
                }
            );
        }
        assert!(resp.root.is_some(), "导航信号应帮助生活记忆成为起点");
        let root_node = resp
            .nodes
            .iter()
            .find(|n| resp.root.as_deref() == Some(n.id.as_str()));
        assert!(
            root_node.is_some_and(|n| n.content.contains("番茄炒蛋")),
            "起点应为生活记忆而非代码: {:?}",
            root_node.map(|n| &n.content)
        );
        // CodeContext 过滤仍生效：代码块不进入联想链
        assert!(
            resp.nodes
                .iter()
                .all(|node| !node.content.contains("run_test_suite")),
            "导航不得豁免 CodeContext 过滤: {:?}",
            resp.nodes.iter().map(|n| &n.content).collect::<Vec<_>>()
        );
        assert!(!resp.weak_match, "存在实质起点时不应标记弱匹配");
    }

    /// P3.2-2：导航信号全无效（palaces 空 → from_json 返回 None）时，
    /// 行为与无导航基线逐字节一致（诚实降级）。
    #[test]
    fn test_run_association_explore_navigation_invalid_degrades() {
        use crate::engine::navigation::NavigationSignal;
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_nav_invalid_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        let mut store = new_statistical_store(&dir_str);

        let life = Memory::new(
            "今晚吃番茄炒蛋，记得先买番茄".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(7),
            None,
        );
        store.remember(life).unwrap();

        // palaces 为空 → navigated_deep_recall 返回 None → 回退单查询 recall
        let nav = NavigationSignal {
            palaces: vec![],
            probes: None,
            source_version: Some("daoti-lexicon-v1".to_string()),
        };

        let cancel = AtomicBool::new(false);
        let resp = run_association_explore(
            &mut store,
            Some("今晚吃什么"),
            None,
            2,
            2,
            &cancel,
            Some(&nav),
        );

        // 无有效导航方向时仍能正常找到生活起点（与无导航基线一致）
        assert!(resp.root.is_some(), "空导航应回退基线召回");
        let root_node = resp
            .nodes
            .iter()
            .find(|n| resp.root.as_deref() == Some(n.id.as_str()));
        assert!(
            root_node.is_some_and(|n| n.content.contains("番茄炒蛋")),
            "回退基线仍应由词面门禁选出生活记忆"
        );
    }

    // ============================================================
    // 记录层（event_id / entities / Experience）端到端验证
    //
    // 背景（daoti/PREREG_ACTIVE_DISCOVERY.md §3.42 / §3.43）：
    //   用户裁定「先改记录层」。此前验证只到 store 层单元测试与 remember
    //   工具层，**未验证真实 HTTP → 落盘 → 重载**这一完整链路。
    //   本组用例补上该缺口：走生产同构链路（build_v1_router → oneshot），
    //   并**从磁盘重新加载**（而非复用内存缓存），以捕获
    //   "写入成功、重载后字段消失"这一类最隐蔽的失败（§3.42.5 缺陷二）。
    // ============================================================

    /// 记录层用例专用代码库桩：本组端点（remember / associations）不触碰代码库，
    /// 该桩仅满足 build_v1_router 的构造签名。**刻意不 panic**——任何代码库调用
    /// 都属意外，应表现为 0 结果而非掩盖真实失败。
    struct NoopCodebase;

    impl IndexedCodebase for NoopCodebase {
        fn search(&self, query: &str, _top_k: usize) -> crate::RetrievalResult {
            crate::RetrievalResult {
                query: query.to_string(),
                returned: 0,
                total_indexed: 0,
                results: Vec::new(),
            }
        }

        fn multi_keyword_search(
            &self,
            _keywords: &[String],
            _top_k: usize,
        ) -> crate::RetrievalResult {
            crate::RetrievalResult {
                query: String::new(),
                returned: 0,
                total_indexed: 0,
                results: Vec::new(),
            }
        }

        fn get_stats(&self) -> crate::ChunkStats {
            crate::ChunkStats {
                file_count: 0,
                total_chunks: 0,
                type_counts: std::collections::HashMap::new(),
                language_counts: std::collections::HashMap::new(),
                avg_lines: 0.0,
            }
        }

        fn recent_chunks(&self, _top_k: usize) -> crate::RetrievalResult {
            crate::RetrievalResult {
                query: String::new(),
                returned: 0,
                total_indexed: 0,
                results: Vec::new(),
            }
        }
    }

    /// 记录层端到端：HTTP 写入带 event_id/entities 的记忆 → 落盘 → 重载读回，
    /// 并用 `/memories/associations` 验证三类关联可被真实查询。
    #[tokio::test]
    async fn test_record_layer_end_to_end_persist_and_associate() {
        use axum::body::{to_bytes, Body};
        use axum::http::{header, Request};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        use tower::ServiceExt;

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_record_layer_e2e_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();

        let shared = Arc::new(Mutex::new(new_statistical_store(&dir_str)));
        let manager: Arc<Mutex<Box<dyn IndexedCodebase>>> =
            Arc::new(Mutex::new(Box::new(NoopCodebase)));
        let llm_api = Arc::new(RwLock::new(crate::LlmApiConfig::default()));
        let llm_ready = Arc::new(AtomicBool::new(false));
        let app = build_v1_router(shared.clone(), manager, llm_api, llm_ready, false);

        // 两次经历，各自产生"语义不同侧面"的记忆：
        // - e-trip（出行）：西湖徒步 / 楼外楼吃饭 —— 无共享专名，语义相距远
        // - e-bday（生日）：买钓鱼竿 / 订蛋糕 —— 共享实体「爸爸」
        let payloads = [
            serde_json::json!({
                "content": "和爸妈去杭州西湖，在苏堤上走了整整一下午",
                "memory_type": "experience",
                "event_id": "e-trip",
                "entities": [{"name": "爸妈", "kind": "person"},
                             {"name": "西湖", "kind": "place"}],
                "importance": 7,
            }),
            serde_json::json!({
                "content": "中午在楼外楼吃了西湖醋鱼，味道一般但环境好",
                "memory_type": "experience",
                "event_id": "e-trip",
                "entities": [{"name": "楼外楼", "kind": "place"}],
                "importance": 5,
            }),
            serde_json::json!({
                "content": "爸爸下个月生日，想送他一套钓鱼竿",
                "memory_type": "experience",
                "event_id": "e-bday",
                "entities": [{"name": "爸妈", "kind": "person"},
                             {"name": "钓鱼竿", "kind": "thing"}],
                "importance": 8,
            }),
            // ★v0.9.8 联想用例的**关键一条**：与 e-trip 同一次经历，
            // 但内容在词面与语义上都**远离**「苏堤」（讲的是回程与充电宝）——
            // 因此它不会被检索召回，却应当被"共同经历"联想补出来。
            // 这正是"相似度给不出、只能靠记录"的连接（本用例的验证靶心）。
            serde_json::json!({
                "content": "回程的高铁上把充电宝忘在了座位底下",
                "memory_type": "experience",
                "event_id": "e-trip",
                "entities": [{"name": "充电宝", "kind": "thing"}],
                "importance": 3,
            }),
        ];

        let mut ids = Vec::new();
        for payload in &payloads {
            let request = Request::builder()
                .method("POST")
                .uri("/memories/remember")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap();
            let response = app
                .clone()
                .into_service()
                .oneshot(request)
                .await
                .expect("HTTP 层调用失败");
            let status = response.status();
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert_eq!(
                status,
                StatusCode::OK,
                "记忆写入应返回 200，实测 {status}，body={}",
                String::from_utf8_lossy(&bytes)
            );
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["success"], true, "写入应成功: {body}");
            ids.push(body["memory_id"].as_str().unwrap().to_string());
        }

        // ---------- 关键：从磁盘重新加载（而非复用内存缓存）----------
        // 这一步是"持久层字段白名单漏字段"的唯一检出手段（§3.42.5）。
        // 用 persistence().load_all_memories() 直读磁盘，绕开 store 内存缓存。
        let reloaded = MemoryStore::new(JsonPersistence::new(&dir_str).unwrap());
        let all = reloaded
            .persistence()
            .load_all_memories()
            .expect("重载失败");
        assert_eq!(all.len(), 4, "重载后应有 4 条记忆");

        // 负向对照（方法论 74）：断言字段**确实出现在磁盘文本中**。
        // 若仅断言"内存读回正确"，当 JSON 序列化漏掉 skip_serializing 字段时
        // 测试仍可能因缓存而通过；直接查磁盘文本可堵住该假绿。
        let raw = std::fs::read_to_string(dir.join("memories.json")).expect("读取落盘 JSON 失败");
        assert!(
            raw.contains("\"event_id\""),
            "落盘 JSON 文本必须含 event_id 键——不在则字段在序列化环节被丢弃"
        );
        assert!(raw.contains("e-trip"), "落盘 JSON 必须含事件 ID 值 e-trip");
        assert!(
            raw.contains("钓鱼竿"),
            "落盘 JSON 必须含实体名——不在则 entities 在序列化环节被丢弃"
        );

        let by_content = |key: &str| {
            all.iter()
                .find(|m| m.content.contains(key))
                .unwrap_or_else(|| panic!("重载后未找到含『{key}』的记忆——字段可能在落盘环节丢失"))
        };

        let trip = by_content("苏堤");
        assert_eq!(
            trip.event_id.as_deref(),
            Some("e-trip"),
            "重载后 event_id 必须保留（否则『共同经历』信息在落盘环节被静默丢弃）"
        );
        assert_eq!(
            trip.memory_type,
            crate::memory_types::MemoryType::Experience
        );
        assert!(
            trip.entities.iter().any(|e| e.name == "爸妈"),
            "重载后 entities 必须保留，实测 {:?}",
            trip.entities
        );

        let meal = by_content("楼外楼");
        assert_eq!(meal.event_id.as_deref(), Some("e-trip"));

        let gift = by_content("钓鱼竿");
        assert_eq!(gift.event_id.as_deref(), Some("e-bday"));

        // ---------- 关联推导：同经历 + 共享实体两类并存 ----------
        let request = Request::builder()
            .method("POST")
            .uri("/memories/associations")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "memory_id": gift.id }).to_string(),
            ))
            .unwrap();
        let response = app
            .clone()
            .into_service()
            .oneshot(request)
            .await
            .expect("associations HTTP 调用失败");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(status, StatusCode::OK, "关联查询应返回 200");
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let assoc = body["associations"]
            .as_array()
            .expect("associations 应为数组");

        // 「钓鱼竿」这条：与「西湖」共享实体爸妈（shared_entity）
        assert!(
            assoc.iter().any(|a| {
                a["relation"] == "shared_entity"
                    && a["content_preview"].as_str().unwrap_or("").contains("苏堤")
            }),
            "应给出与西湖记忆的『共享实体』关联（依据实体而非语义相似）: {body}"
        );
        // 每条关联必须带人类可读依据（对应判据「人类可解释」）
        for a in assoc {
            assert!(
                a["why"].as_str().is_some_and(|w| !w.is_empty()),
                "每条关联必须携带 why 依据: {a}"
            );
        }

        // ---------- 关联类型过滤 ----------
        let request = Request::builder()
            .method("POST")
            .uri("/memories/associations")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "memory_id": trip.id, "relation": "same_event" }).to_string(),
            ))
            .unwrap();
        let response = app
            .clone()
            .into_service()
            .oneshot(request)
            .await
            .expect("associations 过滤调用失败");
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let filtered = body["associations"].as_array().unwrap();
        assert!(
            filtered.iter().all(|a| a["relation"] == "same_event"),
            "relation 过滤应只返回同经历关联: {body}"
        );
        assert!(
            filtered.iter().any(|a| a["content_preview"]
                .as_str()
                .unwrap_or("")
                .contains("楼外楼")),
            "『西湖徒步』的同经历关联应包含『楼外楼吃饭』（语义不相似但同源）: {body}"
        );

        // ---------- 关联图端到端：HTTP → 多跳结构推理 ----------
        // 以「钓鱼竿」（e-bday，含实体 爸妈/钓鱼竿）为根：
        //   直接：同经历「蛋糕」…（本数据集无第二条 e-bday，故直接边为 0）
        //   间接：根 —共享实体(爸妈)— 「西湖」 —同经历— 「楼外楼」 ⇒ 两层
        // 关键：根与「楼外楼」之间**无任何直接记录**，该关联只能由结构推出。
        let request = Request::builder()
            .method("POST")
            .uri("/memories/association-graph")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "memory_id": gift.id }).to_string(),
            ))
            .unwrap();
        let response = app
            .clone()
            .into_service()
            .oneshot(request)
            .await
            .expect("association-graph HTTP 调用失败");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "关联图应返回 200，实测 {status}，body={}",
            String::from_utf8_lossy(&bytes)
        );
        let g: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(g["root"].as_str(), Some(gift.id.as_str()));
        assert_eq!(g["truncated"], false, "默认上限 50 不应截断: {g}");

        let edges = g["edges"].as_array().expect("edges 应为数组");
        // 直接边：与「西湖」共享 person 实体「爸妈」
        assert!(
            edges.iter().any(|e| {
                e["hops"] == 1
                    && e["relation"] == "shared_entity"
                    && e["to"].as_str() == Some(trip.id.as_str())
            }),
            "应有直接边：与西湖记忆共享实体「爸妈」，实际: {edges:?}"
        );
        // ★ 间接边：根 →(共享实体)→ 西湖 →(同一次经历)→ 楼外楼
        let ind = edges
            .iter()
            .find(|e| e["hops"] == 2 && e["to"].as_str() == Some(meal.id.as_str()))
            .unwrap_or_else(|| {
                panic!("应推出指向『楼外楼』的间接关联（二者无任何直接记录）: {edges:?}")
            });
        assert_eq!(ind["relation"], "indirect");
        assert_eq!(
            ind["path"].as_array().map(|p| p.len()),
            Some(3),
            "间接边的 path 必须含 [根, 中间, 目标] 三个节点，供人工核验: {ind}"
        );
        assert!(
            ind["why"]
                .as_str()
                .is_some_and(|w| w.contains("共享实体") && w.contains("同一次经历")),
            "间接边的解释须写出两段关系类型，实际: {}",
            ind["why"]
        );

        // 节点自洽：edges 中出现的所有节点 ID 都必须在 nodes 中
        // （否则前端渲染悬空边——图数据不自洽）
        let node_ids: Vec<&str> = g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["memory_id"].as_str().unwrap())
            .collect();
        for e in edges {
            for k in ["from", "to"] {
                let id = e[k].as_str().unwrap();
                assert!(
                    node_ids.contains(&id),
                    "边引用了不存在的节点 {id}（图不自洽）: nodes={node_ids:?}"
                );
            }
        }

        // ---------- ★v0.9.8：联想接入**检索出口**（真正的记忆联想）----------
        //
        // 此前关联推导只挂在详情页接口：用户必须先点开某条记忆才看得到关联，
        // **检索结果本身从不带联想**。本段验证 enrich 检索出口真的带出联想，
        // 且是"语义不相似、但由记录必然关联"的那一类。
        //
        // 检索「苏堤」→ 主结果应含西湖记忆 → 联想分区应补出同一次经历的
        // 「楼外楼」（两句无共同词、语义不相似，靠 event_id 连上）。
        //
        // ★`top_k` 必须 **小于语料条数（4）**，否则主检索会把全部记忆都返回
        // ⇒「充电宝不在主结果」这一前提**无法由构造保证**，整个"补全"断言退化。
        // （v0.9.8 门控翻转实测暴露：此前该前提是**旧道体回归校验层**偶然
        // 过滤掉「充电宝」才成立的——见 daoti/PREREG §3.55。
        // 依赖一个已被否证的通道来满足测试前提属设计缺陷，故改为构造保证。）
        // 取 3：主结果为 [苏堤, 钓鱼竿, 楼外楼]，「充电宝」被 top_k 截断——
        // 与被过滤时得到的主结果集**完全相同**，但依据由"噪声过滤"换成"截断"。
        let request = Request::builder()
            .method("POST")
            .uri("/memories/enrich")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "query": "苏堤", "top_k": 3 }).to_string(),
            ))
            .unwrap();
        let response = app
            .clone()
            .into_service()
            .oneshot(request)
            .await
            .expect("enrich HTTP 调用失败");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "enrich 应返回 200，实测 {status}，body={}",
            String::from_utf8_lossy(&bytes)
        );
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        // 联想必须**单独分区**（不混进 memories），否则两类不同证据性质的结果
        // 会被误当作可按分数排序的同一列表
        let associated = body["associated"]
            .as_array()
            .unwrap_or_else(|| panic!("enrich 响应必须含 associated 分区: {body}"));

        // 负向对照（防退化为恒真）：先确认「充电宝」那条**确实没被主检索召回**
        // ——若它本就在主结果里，那这段断言就无法证明"联想补全"起了作用。
        let main_ids: Vec<&str> = body["memories"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["id"].as_str())
            .collect();
        assert!(
            !body["memories"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["content"].as_str().unwrap_or("").contains("充电宝")),
            "前提不成立：『充电宝』那条若已被主检索召回，则本用例无法证明联想补全的价值。\
             主检索实际返回内容：{:?}",
            body["memories"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m["content"].as_str().unwrap_or(""))
                .collect::<Vec<_>>()
        );

        let meal_assoc = associated
            .iter()
            .find(|a| {
                a["content_preview"]
                    .as_str()
                    .unwrap_or("")
                    .contains("充电宝")
            })
            .unwrap_or_else(|| {
                panic!(
                    "检索「苏堤」时应联想起同一次经历的「充电宝」\
（二者词面与语义都远离，唯一依据是 event_id）: {associated:?}"
                )
            });
        assert_eq!(
            meal_assoc["relation"], "same_event",
            "该联想必须标注为『共同经历』: {meal_assoc}"
        );
        assert!(
            meal_assoc["why"]
                .as_str()
                .is_some_and(|w| w.contains("e-trip")),
            "依据须写出具体 event_id（可解释到具体对象）: {meal_assoc}"
        );
        // 可追溯：必须说明从哪条记忆联想过来
        assert!(
            meal_assoc["via_memory_id"].as_str().is_some(),
            "必须标注联想起点 via_memory_id: {meal_assoc}"
        );

        // 联想项不得同时出现在主结果里（否则是重复，不是联想）
        for a in associated {
            let aid = a["memory_id"].as_str().unwrap();
            assert!(
                !main_ids.contains(&aid),
                "联想项 {aid} 不应同时出现在主结果中（应去重）"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 记录层异常路径：memory_id 不存在时返回空关联而非报错
    /// （HCSE 要求：异常输入必须有明确、可预期的应答，不得挂死或 500）。
    #[tokio::test]
    async fn test_associations_unknown_memory_returns_empty_not_error() {
        use axum::body::{to_bytes, Body};
        use axum::http::{header, Request};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        use tower::ServiceExt;

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_record_layer_unknown_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();

        let shared = Arc::new(Mutex::new(new_statistical_store(&dir_str)));
        let manager: Arc<Mutex<Box<dyn IndexedCodebase>>> =
            Arc::new(Mutex::new(Box::new(NoopCodebase)));
        let llm_api = Arc::new(RwLock::new(crate::LlmApiConfig::default()));
        let llm_ready = Arc::new(AtomicBool::new(false));
        let app = build_v1_router(shared, manager, llm_api, llm_ready, false);

        let request = Request::builder()
            .method("POST")
            .uri("/memories/associations")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "memory_id": "not-exist-id" }).to_string(),
            ))
            .unwrap();
        let response = app
            .into_service()
            .oneshot(request)
            .await
            .expect("HTTP 层调用失败");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "不存在的 memory_id 应返回 200 + 空列表，而非 5xx"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["total"], 0, "应返回空关联: {body}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ============================================================
    // v0.9.8：联想中心接入「记录层联想」+ 示例问题数据驱动
    //
    // 背景：联想中心此前走的是**相似度 BFS 逐跳扩散**——一个专门叫
    // 「联想中心」的页面，用的却是相似度，记录层关联（同一次经历/共享实体）
    // **从未被它消费**。本组用例固化"它真的用上了记录层"。
    // ============================================================

    /// ★核心：联想中心探索结果中必须出现「记录层」节点
    ///
    /// 构造：起点与另一条记忆同属一次经历，但内容**语义毫不相似**——
    /// 相似度 BFS 不可能把它带出来，只有记录层关联能。若该节点出现
    /// （source="record" 且带 relation/why），说明联想中心真的用上了记录层。
    #[tokio::test]
    async fn test_explore_surfaces_record_layer_node() {
        use axum::body::{to_bytes, Body};
        use axum::http::{header, Request};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        use tower::ServiceExt;

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_record_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();

        let shared = Arc::new(Mutex::new(new_statistical_store(&dir_str)));
        let manager: Arc<Mutex<Box<dyn IndexedCodebase>>> =
            Arc::new(Mutex::new(Box::new(NoopCodebase)));
        let llm_api = Arc::new(RwLock::new(crate::LlmApiConfig::default()));
        let llm_ready = Arc::new(AtomicBool::new(false));
        let app = build_v1_router(shared, manager, llm_api, llm_ready, false);

        // 两条同一次经历、但语义毫不相似的记忆
        for payload in [
            serde_json::json!({
                "content": "和爸妈去杭州西湖，在苏堤上走了整整一下午",
                "memory_type": "experience",
                "event_id": "e-trip-x",
                "importance": 7,
            }),
            serde_json::json!({
                "content": "回程的高铁上把充电宝忘在了座位底下",
                "memory_type": "experience",
                "event_id": "e-trip-x",
                "importance": 3,
            }),
        ] {
            let request = Request::builder()
                .method("POST")
                .uri("/memories/remember")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap();
            let response = app.clone().into_service().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }

        // 从「苏堤」出发探索
        let request = Request::builder()
            .method("POST")
            .uri("/associations/explore")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "query": "苏堤", "depth": 2, "width": 3 }).to_string(),
            ))
            .unwrap();
        let response = app.clone().into_service().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "探索应返回 200，实测 {status}，body={}",
            String::from_utf8_lossy(&bytes)
        );
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let nodes = body["nodes"].as_array().expect("nodes 应为数组");

        // ★记录层节点必须出现（这是本用例的靶心）
        let rec = nodes
            .iter()
            .find(|n| {
                n["source"] == "record" && n["content"].as_str().unwrap_or("").contains("充电宝")
            })
            .unwrap_or_else(|| {
                panic!(
                    "联想中心应通过**记录层**带回同一次经历的「充电宝」记忆\
（相似度 BFS 给不出它，只能靠 event_id）: {nodes:?}"
                )
            });
        assert_eq!(
            rec["relation"], "same_event",
            "记录层节点必须标注关联类型: {rec}"
        );
        assert!(
            rec["why"].as_str().is_some_and(|w| w.contains("e-trip-x")),
            "记录层节点必须带人类可读依据（含具体 event_id）: {rec}"
        );

        // 边也必须标注关系类型，且自洽（from/to 均在 nodes 中）
        let edges = body["edges"].as_array().expect("edges 应为数组");
        let node_ids: Vec<&str> = nodes.iter().filter_map(|n| n["id"].as_str()).collect();
        let rec_edge = edges
            .iter()
            .find(|e| e["to"] == rec["id"] && e["relation"] == "same_event");
        assert!(
            rec_edge.is_some(),
            "应有标注 same_event 的记录层边指向该节点: {edges:?}"
        );
        for e in edges {
            for k in ["from", "to"] {
                let id = e[k].as_str().unwrap();
                assert!(
                    node_ids.contains(&id),
                    "边引用了不存在的节点 {id}（图不自洽）"
                );
            }
        }

        // 负向对照：相似度扩散的节点**不应**带 relation（两类性质必须可区分）
        for n in nodes {
            if n["source"] == "expanded" {
                assert!(
                    n["relation"].is_null(),
                    "相似度扩散节点不应带记录层 relation（否则两类无法区分）: {n}"
                );
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 示例问题必须来自用户真实记忆，且空库时返回空（不得编造）
    #[tokio::test]
    async fn test_association_suggestions_are_data_driven() {
        use axum::body::{to_bytes, Body};
        use axum::http::Request;
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        use tower::ServiceExt;

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_suggest_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();

        let shared = Arc::new(Mutex::new(new_statistical_store(&dir_str)));
        let manager: Arc<Mutex<Box<dyn IndexedCodebase>>> =
            Arc::new(Mutex::new(Box::new(NoopCodebase)));
        let llm_api = Arc::new(RwLock::new(crate::LlmApiConfig::default()));
        let llm_ready = Arc::new(AtomicBool::new(false));
        let app = build_v1_router(shared, manager, llm_api, llm_ready, false);

        // ① 空库：必须返回空数组（诚实空态，不得编造示例）
        let request = Request::builder()
            .method("GET")
            .uri("/associations/suggestions")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().into_service().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            body["count"], 0,
            "空库必须返回 0 条示例（不得硬编码任何语料）: {body}"
        );

        // ② 写入技术类记忆（**非生活场景**）：示例必须来自这些内容
        for payload in [
            serde_json::json!({
                "content": "v0.9.8 修复了 event_id 填写率 0% 的分发缺口问题",
                "memory_type": "decision",
                "entities": [{"name": "event_id", "kind": "thing"}],
                "importance": 8,
            }),
            serde_json::json!({
                "content": "排查了 Nginx proxy_pass 前缀替换的陷阱",
                "memory_type": "decision",
                "entities": [{"name": "Nginx", "kind": "thing"}],
                "importance": 7,
            }),
        ] {
            let request = Request::builder()
                .method("POST")
                .uri("/memories/remember")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap();
            let response = app.clone().into_service().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }

        let request = Request::builder()
            .method("GET")
            .uri("/associations/suggestions")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().into_service().oneshot(request).await.unwrap();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let items = body["suggestions"]
            .as_array()
            .expect("suggestions 应为数组");

        // 每一条的文本必须**能在该用户记忆中找到来源**（不出现库外语料）
        let all_text = body.to_string();
        assert!(
            !all_text.contains("今晚吃什么") && !all_text.contains("周末去哪儿玩"),
            "★不得出现硬编码的生活场景示例（这正是本次修复的缺陷）: {body}"
        );
        for s in items {
            let text = s["text"].as_str().unwrap_or("");
            let from_memory = text.contains("event_id")
                || text.contains("Nginx")
                || text.contains("填写率")
                || text.contains("proxy_pass");
            assert!(
                from_memory || text.is_empty(),
                "示例文本必须来自用户记忆内容/实体，实测: {s}"
            );
            assert!(
                s["origin"].as_str().is_some(),
                "示例必须标注来源类型（可核验非凑数）: {s}"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
