// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件实现复杂度预算追踪，属于守护层 (Layer 2)。
// ============================================================
//
// 复杂度预算 (ComplexityBudget)
//
// 质疑五·终极：系统是否已经达到了"人类无法驾驭的复杂度"？
//
// 当系统拥有 297 个测试、15+ 个核心模块、自愈降级链路、
// 动态参数调节、审计防篡改、隐式反馈等功能时，其内部状态
// 之多、交互链条之长，可能已超出单个开发者能完全理解并
// 安全变更的阈值。
//
// 本模块提供三层防护：
//
// 1. 复杂度预算追踪：监控模块数量、公开 API 表面、跨模块
//    依赖数量，设定预算上限，防止系统无限膨胀。
//
// 2. 因果链映射：为每个关键参数建立"影响图谱"，展示修改
//    该参数会如何经由 DaoRegulator、SynthesisJournal、
//    UserFeedback 等子系统产生涟漪效应。
//
// 3. 可维护性评分：基于上述指标生成综合评分，作为 CI/CD
//    的门禁条件——评分低于阈值时警告或阻止合并。
//
// 道枢映射：乾卦·天 (☰) — 天行健，君子以自强不息。
//   复杂度预算不是限制，而是自知——如同天道运行，了解
//   自己的边界才能持续演化而不崩溃。

use serde::{Deserialize, Serialize};

/// 红线检查结果（质疑二·防退化：从"事后统计"升级为"CI 守门人"）
///
/// 当红线违规时，对应的测试会失败，强制阻止 CI/CD 流程。
/// 这确保复杂度预算不会退化为无人理睬的背景噪音。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedLineResult {
    /// 是否通过检查
    pub passed: bool,
    /// 违规列表
    pub violations: Vec<RedLineViolation>,
    /// 检查时间戳（毫秒）
    pub checked_at_ms: u64,
    /// 检查摘要
    pub summary: String,
}

/// 红线违规详情
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedLineViolation {
    /// 违规规则名称
    pub rule: String,
    /// 当前值
    pub current: String,
    /// 阈值
    pub threshold: String,
    /// 严重程度
    pub severity: RedLineSeverity,
    /// 修复建议
    pub suggestion: String,
}

/// 红线严重程度
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, PartialOrd)]
pub enum RedLineSeverity {
    /// 警告（不阻止 CI，但提示风险）
    Warning,
    /// 严重（阻止 CI）
    Severe,
    /// 紧急（阻止 CI，需要立即处理）
    Critical,
}

/// 复杂度预算（质疑五·终极：防止系统超出人类可理解范围）
///
/// 追踪系统的复杂度指标，提供因果链映射和可维护性评分。
/// v2.0 新增 complexity_honesty 诚实度评分，防止"指标游戏"。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityBudget {
    /// 模块数量
    pub module_count: usize,
    /// 公开 API 表面（pub fn 总数）
    pub public_api_surface: usize,
    /// 跨模块依赖数量
    pub cross_module_dependencies: usize,
    /// 最大依赖深度（最长因果链）
    pub max_dependency_depth: usize,
    /// 因果链映射（参数 → 影响子系统）
    pub causal_chains: Vec<CausalChain>,
    /// 复杂度预算上限
    pub budget_limit: ComplexityLimit,
    /// 可维护性评分 (0.0 ~ 1.0，越高越好)
    pub maintainability_score: f32,
    /// 预算消耗率 (0.0 ~ 1.0)
    pub budget_consumed: f32,
    /// 复杂度诚实度（质疑二·防博弈：检测隐性复杂度反模式）
    pub complexity_honesty: ComplexityHonesty,
    /// 生成时间戳（毫秒）
    pub generated_at_ms: u64,
}

/// 复杂度预算上限
///
/// 当系统的复杂度指标超过这些上限时，意味着系统已经超出了
/// 单个开发者或小团队能安全理解和变更的阈值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityLimit {
    /// 最大模块数
    pub max_modules: usize,
    /// 最大公开 API 数
    pub max_public_api: usize,
    /// 最大跨模块依赖数
    pub max_cross_deps: usize,
    /// 最大依赖深度
    pub max_dependency_depth: usize,
    /// 最低可维护性评分
    pub min_maintainability_score: f32,
}

impl Default for ComplexityLimit {
    fn default() -> Self {
        Self {
            // 当前 ~20 个核心模块，预算上限设为 35
            max_modules: 35,
            // 当前 ~200 个 pub fn，预算上限设为 350
            max_public_api: 350,
            // 预算上限：模块数 × 2
            max_cross_deps: 70,
            // 最深因果链：当前约 5 层，上限 8 层
            max_dependency_depth: 8,
            // 最低可维护性：0.4 以下触发警报
            min_maintainability_score: 0.4,
        }
    }
}

/// 复杂度诚实度（质疑二·防博弈：防止"指标游戏"）
///
/// 开发者可能会为了降低依赖深度指标，而采用更隐晦的通信方式
/// （如全局状态、深层 trait 继承），这实际上增加了隐含的、
/// 更危险的复杂性。诚实度评分检测这些反模式，确保复杂度预算
/// 不会退化为"看起来很美"的数字游戏。
///
/// 道枢映射：兑卦·兑 (☱) — 说也，刚中而柔外。
///   诚实度是系统"说"出自己真实复杂度的能力——不粉饰，
///   不隐藏，直面自身的混乱与秩序。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityHonesty {
    /// 诚实度评分 (0.0 ~ 1.0，越高越诚实)
    pub score: f32,
    /// 检测到的诚实度违规
    pub violations: Vec<HonestyViolation>,
    /// 隐性复杂度估算（未被正式指标捕获的复杂度）
    pub hidden_complexity_estimate: f32,
}

/// 诚实度违规：检测到的隐性复杂度反模式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HonestyViolation {
    /// 违规类型
    pub violation_type: HonestyViolationType,
    /// 违规描述
    pub description: String,
    /// 严重程度
    pub severity: HonestySeverity,
}

/// 诚实度违规类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HonestyViolationType {
    /// 全局状态/静态变量（绕过依赖追踪的通信方式）
    GlobalState,
    /// 深层 trait 继承链（>3 层的 trait 嵌套）
    DeepTraitChain,
    /// 回调地狱（多层异步嵌套）
    CallbackHell,
    /// 隐式类型转换（通过 From/Into 的隐蔽耦合）
    ImplicitCoupling,
    /// 未文档化的跨模块依赖
    UndocumentedDependency,
    /// 过度泛型化（泛型参数 > 3 个）
    OverlyGeneric,
}

impl HonestyViolationType {
    pub fn as_str(&self) -> &str {
        match self {
            HonestyViolationType::GlobalState => "global_state",
            HonestyViolationType::DeepTraitChain => "deep_trait_chain",
            HonestyViolationType::CallbackHell => "callback_hell",
            HonestyViolationType::ImplicitCoupling => "implicit_coupling",
            HonestyViolationType::UndocumentedDependency => "undocumented_dependency",
            HonestyViolationType::OverlyGeneric => "overly_generic",
        }
    }
}

/// 诚实度严重程度
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, PartialOrd)]
pub enum HonestySeverity {
    /// 提示：可优化但不紧急
    Notice,
    /// 警告：建议在下次迭代中修复
    Warning,
    /// 严重：影响可维护性，需优先处理
    Severe,
}

impl Default for ComplexityHonesty {
    fn default() -> Self {
        Self {
            score: 1.0,
            violations: Vec::new(),
            hidden_complexity_estimate: 0.0,
        }
    }
}

impl ComplexityHonesty {
    /// 重置诚实度状态
    pub fn reset(&mut self) {
        self.score = 1.0;
        self.violations.clear();
        self.hidden_complexity_estimate = 0.0;
    }

    /// 记录一条诚实度违规并重新计算评分
    pub fn record_violation(&mut self, violation: HonestyViolation) {
        self.violations.push(violation);
        self.recalculate();
    }

    /// 根据违规列表重新计算诚实度评分
    ///
    /// 评分规则：
    /// - 每个 Notice 扣 0.05
    /// - 每个 Warning 扣 0.1
    /// - 每个 Severe 扣 0.2
    /// - 最低为 0.1（永远不为零，保留一丝诚实）
    fn recalculate(&mut self) {
        let penalty: f32 = self
            .violations
            .iter()
            .map(|v| match v.severity {
                HonestySeverity::Notice => 0.05,
                HonestySeverity::Warning => 0.1,
                HonestySeverity::Severe => 0.2,
            })
            .sum();

        self.score = (1.0 - penalty).max(0.1);
        self.hidden_complexity_estimate = 1.0 - self.score;
    }

    /// 生成面向开发者的提示
    pub fn summary(&self) -> String {
        if self.violations.is_empty() {
            return "复杂度诚实度: 优秀 — 未检测到隐性复杂度反模式".to_string();
        }

        let severe = self
            .violations
            .iter()
            .filter(|v| v.severity == HonestySeverity::Severe)
            .count();
        let warnings = self
            .violations
            .iter()
            .filter(|v| v.severity == HonestySeverity::Warning)
            .count();
        let notices = self
            .violations
            .iter()
            .filter(|v| v.severity == HonestySeverity::Notice)
            .count();

        format!(
            "复杂度诚实度: {:.0}% — {} 严重, {} 警告, {} 提示. 隐性复杂度估算 {:.0}%",
            self.score * 100.0,
            severe,
            warnings,
            notices,
            self.hidden_complexity_estimate * 100.0,
        )
    }
}

/// 因果链：追踪一个参数变更如何影响整个系统
///
/// 当未来的维护者试图修改一个看似简单的参数（如 GC 的触发间隔），
/// 他需要知道这会如何经由 DaoRegulator、SynthesisJournal 和
/// UserFeedback 的隐式校准，最终影响整个记忆生态的健康。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalChain {
    /// 参数名称
    pub parameter: String,
    /// 参数所在模块
    pub source_module: String,
    /// 影响链条（按顺序）
    pub chain: Vec<CausalLink>,
    /// 影响深度（链条长度）
    pub depth: usize,
    /// 影响广度（受影响的子系统数量）
    pub breadth: usize,
    /// 风险等级
    pub risk_level: RiskLevel,
}

/// 因果链中的一环
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalLink {
    /// 受影响的模块
    pub module: String,
    /// 受影响的函数/组件
    pub function: String,
    /// 影响类型
    pub impact_type: ImpactType,
    /// 影响描述
    pub description: String,
}

/// 影响类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ImpactType {
    /// 直接传递（参数值直接使用）
    DirectPass,
    /// 条件分支（参数影响决策逻辑）
    ConditionalBranch,
    /// 校准/调节（参数影响调节器的行为）
    Calibration,
    /// 级联（参数影响下游输出质量）
    Cascade,
    /// 反馈回路（影响会循环回参数本身）
    FeedbackLoop,
}

impl ImpactType {
    pub fn as_str(&self) -> &str {
        match self {
            ImpactType::DirectPass => "direct",
            ImpactType::ConditionalBranch => "conditional",
            ImpactType::Calibration => "calibration",
            ImpactType::Cascade => "cascade",
            ImpactType::FeedbackLoop => "feedback_loop",
        }
    }
}

/// 风险等级
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, PartialOrd)]
pub enum RiskLevel {
    /// 低风险：影响范围小，变更安全
    Low,
    /// 中风险：影响多个子系统，需要谨慎
    Medium,
    /// 高风险：影响核心子系统，需要全面测试
    High,
    /// 极高风险：影响整个记忆生态，修改前必须深度审查
    Critical,
}

impl RiskLevel {
    pub fn as_str(&self) -> &str {
        match self {
            RiskLevel::Low => "low",
            RiskLevel::Medium => "medium",
            RiskLevel::High => "high",
            RiskLevel::Critical => "critical",
        }
    }
}

impl ComplexityBudget {
    /// 创建新的复杂度预算追踪器
    pub fn new() -> Self {
        Self {
            module_count: 0,
            public_api_surface: 0,
            cross_module_dependencies: 0,
            max_dependency_depth: 0,
            causal_chains: Self::build_default_causal_chains(),
            budget_limit: ComplexityLimit::default(),
            maintainability_score: 1.0,
            budget_consumed: 0.0,
            complexity_honesty: ComplexityHonesty::default(),
            generated_at_ms: 0,
        }
    }

    /// 使用当前系统指标更新复杂度预算
    pub fn update(
        &mut self,
        module_count: usize,
        public_api_surface: usize,
        cross_module_dependencies: usize,
        max_dependency_depth: usize,
    ) {
        self.module_count = module_count;
        self.public_api_surface = public_api_surface;
        self.cross_module_dependencies = cross_module_dependencies;
        self.max_dependency_depth = max_dependency_depth;

        // 计算预算消耗率
        let module_ratio = module_count as f32 / self.budget_limit.max_modules as f32;
        let api_ratio = public_api_surface as f32 / self.budget_limit.max_public_api as f32;
        let dep_ratio = cross_module_dependencies as f32 / self.budget_limit.max_cross_deps as f32;
        let depth_ratio =
            max_dependency_depth as f32 / self.budget_limit.max_dependency_depth as f32;

        // 取最大消耗率作为预算消耗
        self.budget_consumed = module_ratio
            .max(api_ratio)
            .max(dep_ratio)
            .max(depth_ratio)
            .clamp(0.0, 1.0);

        // 计算可维护性评分
        // 评分 = 1.0 - (加权平均消耗率)
        // 权重：API 表面 40%、依赖 30%、模块 20%、深度 10%
        let weighted = api_ratio * 0.4 + dep_ratio * 0.3 + module_ratio * 0.2 + depth_ratio * 0.1;
        self.maintainability_score = (1.0 - weighted).clamp(0.0, 1.0);

        self.generated_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
    }

    /// 生成复杂度概览（人类可读）
    pub fn summary(&self) -> String {
        let status = if self.budget_consumed > 0.8 {
            "⚠ 预算告急"
        } else if self.budget_consumed > 0.6 {
            "● 预算紧张"
        } else {
            "○ 预算充裕"
        };

        format!(
            "复杂度预算: {} | 模块 {}/{}, API {}/{}, 依赖 {}/{}, 深度 {}/{} | \
             可维护性 {:.1}% | 消耗 {:.0}%",
            status,
            self.module_count,
            self.budget_limit.max_modules,
            self.public_api_surface,
            self.budget_limit.max_public_api,
            self.cross_module_dependencies,
            self.budget_limit.max_cross_deps,
            self.max_dependency_depth,
            self.budget_limit.max_dependency_depth,
            self.maintainability_score * 100.0,
            self.budget_consumed * 100.0,
        )
    }

    /// 查询参数变更的影响范围
    ///
    /// 当开发者计划修改某个参数时，调用此方法可以预览影响范围。
    pub fn preview_impact(&self, parameter: &str) -> Option<&CausalChain> {
        self.causal_chains.iter().find(|c| c.parameter == parameter)
    }

    /// 检查是否超出预算（任一维度超限）
    pub fn is_over_budget(&self) -> bool {
        self.module_count > self.budget_limit.max_modules
            || self.public_api_surface > self.budget_limit.max_public_api
            || self.cross_module_dependencies > self.budget_limit.max_cross_deps
            || self.max_dependency_depth > self.budget_limit.max_dependency_depth
    }

    /// 检查可维护性是否低于阈值
    pub fn is_maintainability_critical(&self) -> bool {
        self.maintainability_score < self.budget_limit.min_maintainability_score
    }

    /// 构建默认因果链映射
    ///
    /// 这是 LRC 系统的"影响图谱"——为每个关键参数记录其修改
    /// 会如何经由各子系统产生涟漪效应。
    ///
    /// 道枢映射：兑卦·兑 (☱) — 说也，刚中而柔外。
    ///   因果链是系统"说"出自己内部关系的方式——透明化
    ///   复杂的交互，让维护者能"看见"参数变更的后果。
    fn build_default_causal_chains() -> Vec<CausalChain> {
        vec![
            // 因果链 1: GC 触发间隔
            CausalChain {
                parameter: "gc_interval_ms".to_string(),
                source_module: "memory_gc".to_string(),
                depth: 4,
                breadth: 4,
                risk_level: RiskLevel::High,
                chain: vec![
                    CausalLink {
                        module: "memory_gc".to_string(),
                        function: "should_run".to_string(),
                        impact_type: ImpactType::DirectPass,
                        description: "直接决定 GC 是否触发".to_string(),
                    },
                    CausalLink {
                        module: "memory_gc".to_string(),
                        function: "collect_garbage".to_string(),
                        impact_type: ImpactType::DirectPass,
                        description: "影响回收频率，进而影响过期记忆的清理速度".to_string(),
                    },
                    CausalLink {
                        module: "dao_regulator".to_string(),
                        function: "regulate".to_string(),
                        impact_type: ImpactType::Calibration,
                        description: "GC 清理速度影响活跃记忆数量，进而影响道同构度指标，最终触发调节器调整合成阈值".to_string(),
                    },
                    CausalLink {
                        module: "synthesis_engine".to_string(),
                        function: "try_synthesize".to_string(),
                        impact_type: ImpactType::Cascade,
                        description: "合成阈值变化影响合成频率，进而影响记忆库的整体质量和多样性".to_string(),
                    },
                ],
            },
            // 因果链 2: 信息增量阈值
            CausalChain {
                parameter: "information_gain_threshold".to_string(),
                source_module: "synthesis_engine".to_string(),
                depth: 5,
                breadth: 5,
                risk_level: RiskLevel::Critical,
                chain: vec![
                    CausalLink {
                        module: "synthesis_engine".to_string(),
                        function: "try_synthesize".to_string(),
                        impact_type: ImpactType::DirectPass,
                        description: "直接决定合成是否执行——阈值越高，合成越少".to_string(),
                    },
                    CausalLink {
                        module: "synthesis_journal".to_string(),
                        function: "record_synthesis".to_string(),
                        impact_type: ImpactType::DirectPass,
                        description: "合成频率影响合成日志的统计分布".to_string(),
                    },
                    CausalLink {
                        module: "dao_regulator".to_string(),
                        function: "regulate".to_string(),
                        impact_type: ImpactType::Calibration,
                        description: "合成日志质量反馈到调节器，影响阈值自身的动态调整".to_string(),
                    },
                    CausalLink {
                        module: "dao_regulator".to_string(),
                        function: "apply_threshold_adjustment".to_string(),
                        impact_type: ImpactType::FeedbackLoop,
                        description: "阈值调整形成反馈回路——调节器修改阈值，阈值影响合成质量，合成质量再反馈给调节器".to_string(),
                    },
                    CausalLink {
                        module: "user_feedback".to_string(),
                        function: "get_implicit_quality_adjustments".to_string(),
                        impact_type: ImpactType::Cascade,
                        description: "合成质量影响用户隐式反馈，进而影响下一次检索的排序".to_string(),
                    },
                ],
            },
            // 因果链 3: 编码器质量评分
            CausalChain {
                parameter: "encoder_quality_score".to_string(),
                source_module: "luoshu_encoder".to_string(),
                depth: 4,
                breadth: 5,
                risk_level: RiskLevel::Critical,
                chain: vec![
                    CausalLink {
                        module: "luoshu_encoder".to_string(),
                        function: "encode_text".to_string(),
                        impact_type: ImpactType::DirectPass,
                        description: "编码器质量直接影响所有语义向量的准确性".to_string(),
                    },
                    CausalLink {
                        module: "mirror_trapezoid".to_string(),
                        function: "mirror_project".to_string(),
                        impact_type: ImpactType::DirectPass,
                        description: "向量质量影响镜像投影的精度，进而影响检索结果的排序".to_string(),
                    },
                    CausalLink {
                        module: "synthesis_engine".to_string(),
                        function: "find_synthesis_clusters".to_string(),
                        impact_type: ImpactType::Cascade,
                        description: "向量质量影响 Jaccard 相似度计算，进而影响合成簇的发现".to_string(),
                    },
                    CausalLink {
                        module: "dao_metrics".to_string(),
                        function: "snapshot".to_string(),
                        impact_type: ImpactType::Cascade,
                        description: "编码质量影响道同构度指标，进而影响系统模式判断".to_string(),
                    },
                ],
            },
            // 因果链 4: 调节器步长乘数
            CausalChain {
                parameter: "step_multiplier".to_string(),
                source_module: "dao_regulator".to_string(),
                depth: 3,
                breadth: 3,
                risk_level: RiskLevel::Medium,
                chain: vec![
                    CausalLink {
                        module: "dao_regulator".to_string(),
                        function: "regulate".to_string(),
                        impact_type: ImpactType::DirectPass,
                        description: "步长乘数决定每次调节的幅度——过大导致振荡，过小导致响应迟缓".to_string(),
                    },
                    CausalLink {
                        module: "dao_regulator".to_string(),
                        function: "detect_oscillation".to_string(),
                        impact_type: ImpactType::ConditionalBranch,
                        description: "步长过大触发振荡检测，可能导致调节器暂停".to_string(),
                    },
                    CausalLink {
                        module: "synthesis_engine".to_string(),
                        function: "try_synthesize".to_string(),
                        impact_type: ImpactType::Cascade,
                        description: "调节器暂停导致合成阈值不再更新，可能影响合成质量".to_string(),
                    },
                ],
            },
            // 因果链 5: 隐式反馈权重
            CausalChain {
                parameter: "implicit_feedback_weight".to_string(),
                source_module: "user_feedback".to_string(),
                depth: 3,
                breadth: 3,
                risk_level: RiskLevel::Medium,
                chain: vec![
                    CausalLink {
                        module: "user_feedback".to_string(),
                        function: "get_implicit_quality_adjustments".to_string(),
                        impact_type: ImpactType::DirectPass,
                        description: "权重决定隐式反馈对检索排序的影响程度".to_string(),
                    },
                    CausalLink {
                        module: "synthesis_engine".to_string(),
                        function: "try_synthesize".to_string(),
                        impact_type: ImpactType::Calibration,
                        description: "隐式反馈权重影响合成质量评估，进而影响合成决策".to_string(),
                    },
                    CausalLink {
                        module: "dao_regulator".to_string(),
                        function: "regulate".to_string(),
                        impact_type: ImpactType::Cascade,
                        description: "合成决策变化影响调节器的输入，可能改变调节策略".to_string(),
                    },
                ],
            },
            // 因果链 6: 阈值基线
            CausalChain {
                parameter: "threshold_baseline".to_string(),
                source_module: "dao_regulator".to_string(),
                depth: 3,
                breadth: 2,
                risk_level: RiskLevel::High,
                chain: vec![
                    CausalLink {
                        module: "dao_regulator".to_string(),
                        function: "apply_threshold_adjustment".to_string(),
                        impact_type: ImpactType::DirectPass,
                        description: "基线是动态阈值的锚点——阈值不能偏离基线超过 max_threshold_deviation".to_string(),
                    },
                    CausalLink {
                        module: "synthesis_engine".to_string(),
                        function: "try_synthesize".to_string(),
                        impact_type: ImpactType::Calibration,
                        description: "基线变化直接影响合成频率的上限和下限".to_string(),
                    },
                    CausalLink {
                        module: "dao_regulator".to_string(),
                        function: "maybe_revert_threshold".to_string(),
                        impact_type: ImpactType::FeedbackLoop,
                        description: "当阈值长期偏离基线时，均值回归机制会将其拉回".to_string(),
                    },
                ],
            },
        ]
    }

    /// 计算可维护性评分（0.0 ~ 1.0，越高越好）
    pub fn maintainability_score(&self) -> f64 {
        self.maintainability_score as f64
    }

    /// 红线检查：当关键指标越过红线时返回 false，阻止 CI/CD 流程
    ///
    /// 质疑二解决方案：ComplexityBudget 从"事后统计员"升级为"守门人"。
    /// 当可维护性评分低于硬性红线时，此方法返回 false，
    /// 对应的测试会失败，从而强制性地阻止 CI/CD 流程。
    ///
    /// 红线规则：
    /// - 可维护性评分 < 0.3 → 不合格（红线触发）
    /// - 诚实度评分 < 0.5 → 不合格（隐性复杂度过高）
    /// - 模块数 > 上限的 150% → 不合格（严重超限）
    /// - 跨模块依赖 > 上限的 200% → 不合格（耦合危机）
    ///
    /// 道枢映射：坎卦·水 (☵) — "习坎，重险也。"
    ///   红线如同坎卦中隐藏的深渊——表面平静，但一旦越过，
    ///   就必须正视危险，而非继续麻木前行。
    pub fn red_line_check(&self) -> RedLineResult {
        let mut violations = Vec::new();
        let mut passed = true;

        // 红线一：可维护性评分
        let maintainability = self.maintainability_score();
        if maintainability < 0.3 {
            passed = false;
            violations.push(RedLineViolation {
                rule: "可维护性评分低于红线".to_string(),
                current: format!("{:.2}", maintainability),
                threshold: "0.30".to_string(),
                severity: RedLineSeverity::Critical,
                suggestion: "立即进行代码重构：拆分大模块、减少跨模块依赖、消除重复代码"
                    .to_string(),
            });
        }

        // 红线二：诚实度评分
        let honesty = &self.complexity_honesty;
        if honesty.score < 0.5 {
            passed = false;
            violations.push(RedLineViolation {
                rule: "诚实度评分低于红线".to_string(),
                current: format!("{:.2}", honesty.score),
                threshold: "0.50".to_string(),
                severity: RedLineSeverity::Critical,
                suggestion: format!(
                    "检测到 {} 个隐性复杂度违规，请逐一修复：{}",
                    honesty.violations.len(),
                    honesty
                        .violations
                        .iter()
                        .map(|v| format!("{:?}", v.violation_type))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }

        // 红线三：模块数超限
        if self.module_count as f64 > self.budget_limit.max_modules as f64 * 1.5 {
            passed = false;
            violations.push(RedLineViolation {
                rule: "模块数量严重超限".to_string(),
                current: format!("{}", self.module_count),
                threshold: format!("{}", (self.budget_limit.max_modules as f64 * 1.5) as u32),
                severity: RedLineSeverity::Severe,
                suggestion: "考虑将模块拆分为独立的 crate 或子目录".to_string(),
            });
        }

        // 红线四：跨模块依赖超限
        if self.cross_module_dependencies as f64 > self.budget_limit.max_cross_deps as f64 * 2.0 {
            passed = false;
            violations.push(RedLineViolation {
                rule: "跨模块依赖严重超限".to_string(),
                current: format!("{}", self.cross_module_dependencies),
                threshold: format!("{}", (self.budget_limit.max_cross_deps as f64 * 2.0) as u32),
                severity: RedLineSeverity::Severe,
                suggestion: "引入接口层（trait）解耦模块，或重新划分模块边界".to_string(),
            });
        }

        let violation_count = violations.len();

        RedLineResult {
            passed,
            violations,
            checked_at_ms: chrono::Utc::now().timestamp_millis() as u64,
            summary: if passed {
                "所有红线检查通过，系统健康".to_string()
            } else {
                format!("{} 项红线违规，CI/CD 将被阻止", violation_count)
            },
        }
    }
}

impl Default for ComplexityBudget {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_budget_is_healthy() {
        let budget = ComplexityBudget::new();
        assert!(!budget.is_over_budget());
        assert!(!budget.is_maintainability_critical());
        assert!(budget.maintainability_score > 0.9);
        assert_eq!(budget.causal_chains.len(), 6);
    }

    #[test]
    fn test_update_with_normal_load() {
        let mut budget = ComplexityBudget::new();
        budget.update(20, 200, 40, 4);

        assert!(!budget.is_over_budget());
        assert!(!budget.is_maintainability_critical());
        // 20/35=0.57, 200/350=0.57, 40/70=0.57, 4/8=0.5
        // 加权: 0.57*0.4 + 0.57*0.3 + 0.57*0.2 + 0.5*0.1 = 0.228+0.171+0.114+0.05 = 0.563
        // 评分: 1.0 - 0.563 = 0.437
        assert!(budget.maintainability_score > 0.4);
        assert!(budget.budget_consumed < 0.7);
    }

    #[test]
    fn test_update_over_budget() {
        let mut budget = ComplexityBudget::new();
        budget.update(40, 400, 80, 10);

        assert!(budget.is_over_budget());
        assert!(budget.maintainability_score < 0.3);
        assert!(budget.budget_consumed > 0.9);
    }

    #[test]
    fn test_preview_impact() {
        let budget = ComplexityBudget::new();

        let impact = budget.preview_impact("gc_interval_ms");
        assert!(impact.is_some());
        let chain = impact.unwrap();
        assert_eq!(chain.risk_level, RiskLevel::High);
        assert_eq!(chain.depth, 4);
        assert_eq!(chain.breadth, 4);

        let impact = budget.preview_impact("information_gain_threshold");
        assert!(impact.is_some());
        let chain = impact.unwrap();
        assert_eq!(chain.risk_level, RiskLevel::Critical);
        assert_eq!(chain.depth, 5);
        assert!(chain
            .chain
            .iter()
            .any(|link| link.impact_type == ImpactType::FeedbackLoop));
    }

    #[test]
    fn test_preview_impact_unknown_param() {
        let budget = ComplexityBudget::new();
        let impact = budget.preview_impact("nonexistent_param");
        assert!(impact.is_none());
    }

    #[test]
    fn test_summary_format() {
        let mut budget = ComplexityBudget::new();
        budget.update(20, 200, 40, 4);
        let summary = budget.summary();
        assert!(summary.contains("复杂度预算"));
        assert!(summary.contains("可维护性"));
    }

    #[test]
    fn test_maintainability_critical() {
        let mut budget = ComplexityBudget::new();
        // 将最低可维护性设为很高的值来触发临界
        budget.budget_limit.min_maintainability_score = 0.9;
        budget.update(30, 300, 60, 7);
        assert!(budget.is_maintainability_critical());
    }

    #[test]
    fn test_default_limit_reasonable() {
        let limit = ComplexityLimit::default();
        assert!(limit.max_modules > 20);
        assert!(limit.max_public_api > 200);
        assert!(limit.max_cross_deps > 40);
        assert!(limit.max_dependency_depth >= 5);
        assert!(limit.min_maintainability_score > 0.0);
    }

    #[test]
    fn test_causal_chain_serialization() {
        let budget = ComplexityBudget::new();
        let json = serde_json::to_string_pretty(&budget).unwrap();
        assert!(json.contains("causal_chains"));
        assert!(json.contains("information_gain_threshold"));
        assert!(json.contains("Critical"));
        assert!(json.contains("FeedbackLoop"));
    }

    #[test]
    fn test_impact_type_as_str() {
        assert_eq!(ImpactType::DirectPass.as_str(), "direct");
        assert_eq!(ImpactType::ConditionalBranch.as_str(), "conditional");
        assert_eq!(ImpactType::Calibration.as_str(), "calibration");
        assert_eq!(ImpactType::Cascade.as_str(), "cascade");
        assert_eq!(ImpactType::FeedbackLoop.as_str(), "feedback_loop");
    }

    #[test]
    fn test_risk_level_ordering() {
        assert!(RiskLevel::Critical > RiskLevel::High);
        assert!(RiskLevel::High > RiskLevel::Medium);
        assert!(RiskLevel::Medium > RiskLevel::Low);
    }

    #[test]
    fn test_red_line_check_passes_for_healthy_budget() {
        let budget = ComplexityBudget::new();
        let result = budget.red_line_check();
        assert!(result.passed, "健康预算应通过红线检查");
        assert!(result.violations.is_empty());
    }

    #[test]
    fn test_red_line_check_fails_on_low_maintainability() {
        let mut budget = ComplexityBudget::new();
        // 模拟极端低可维护性：将所有参数设为 0
        budget.maintainability_score = 0.0;
        budget.module_count = 999;
        budget.cross_module_dependencies = 999;
        let result = budget.red_line_check();
        // 红线检查应该失败
        assert!(!result.passed, "极端低可维护性应触发红线");
        assert!(!result.violations.is_empty());
    }
}
