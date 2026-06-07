// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件实现道枢演化协议，属于守护层 (Layer 2)。
// ============================================================
//
// 道枢演化协议 (DaoEvolutionProtocol) — 质疑五：道的演化与系统价值归宿
//
// 核心问题：系统已完美编码了"道"，但最终目的是什么？
// 是服务于用户的工具，还是承载哲学的庙宇？
//
// 道枢映射：中宫（五）— 统摄八方，调和阴阳
//
// 洛书九宫中，中宫（五）是统摄八方、调和阴阳的枢纽。
// 双模价值宣言（DualModeValueDeclaration）是系统的"中宫"——
// 它不偏向实用主义或哲学主义的任何一方，而是在两者之间
// 建立桥梁，让追求效率的用户和追求思想深度的人都能找到归属。
//
// 中宫之数五，是八卦的"道枢"（Dao Pivot）：
//   - 实用模式（PragmaticValue）：面向用户，提供准确、快速、隐私安全的检索服务
//   - 哲学模式（PhilosophicalValue）：面向思想同道，承载"道"的演化与知识传承
//   - 综合声明（SynthesisStatement）：两者并非对立，而是阴阳互济——实用是哲学的
//     落地验证，哲学是实用的方向指引
//
// 核心功能：
//   - 社区提案系统：允许用户和开发者提交道的演化提案
//   - 演化代际追踪：每次重大演化增加代际计数，记录系统的成长轨迹
//   - 双模价值宣言：明确系统在实用与哲学两个维度的价值承诺
//   - 演化历史审计：所有已接受的演化可追溯，形成知识传承链

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// 获取当前时间戳（毫秒）
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ============================================================
// 提案状态枚举
// ============================================================

/// 提案状态
///
/// 道的演化不是随心所欲的变更，而是经过社区的充分讨论和论证。
/// 提案从"提出"到"被接受"需要经历严格的审查流程。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProposalStatus {
    /// 已提出：提案已提交，等待社区关注
    Proposed,
    /// 审查中：提案正在接受社区成员的审查和讨论
    UnderReview,
    /// 已接受：提案通过审查，哲学基础和实际收益均得到认可
    Accepted,
    /// 已拒绝：提案被驳回，可能是哲学基础不扎实或实际收益不足
    Rejected,
}

impl std::fmt::Display for ProposalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProposalStatus::Proposed => write!(f, "已提出"),
            ProposalStatus::UnderReview => write!(f, "审查中"),
            ProposalStatus::Accepted => write!(f, "已接受"),
            ProposalStatus::Rejected => write!(f, "已拒绝"),
        }
    }
}

// ============================================================
// 社区提案结构体
// ============================================================

/// 社区提案 — 道演化的基本单元
///
/// 每一份提案都必须包含：
/// 1. 哲学基础（philosophical_basis）：提案如何从道/洛书/八卦体系中衍生
/// 2. 实际收益（practical_benefit）：对用户/开发者的具体好处
///
/// 只有同时满足这两个维度的提案，才能通过审查。
/// 这体现了"中宫调和"——不偏废哲学也不偏废实用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaoProposal {
    /// 提案 ID
    pub proposal_id: String,
    /// 提案标题
    pub title: String,
    /// 提案描述
    pub description: String,
    /// 提案者
    pub proposed_by: String,
    /// 提案时间戳（毫秒）
    pub proposed_at_ms: u64,
    /// 哲学基础：提案如何从道/洛书/八卦体系中衍生
    pub philosophical_basis: String,
    /// 实际收益：对用户/开发者的具体好处
    pub practical_benefit: String,
    /// 提案状态
    pub status: ProposalStatus,
    /// 赞成票数
    pub votes_for: u64,
    /// 反对票数
    pub votes_against: u64,
    /// 讨论链接（可为空）
    pub discussion_url: Option<String>,
}

impl DaoProposal {
    /// 创建新提案
    pub fn new(
        proposal_id: String,
        title: String,
        description: String,
        proposed_by: String,
        philosophical_basis: String,
        practical_benefit: String,
    ) -> Self {
        Self {
            proposal_id,
            title,
            description,
            proposed_by,
            proposed_at_ms: now_ms(),
            philosophical_basis,
            practical_benefit,
            status: ProposalStatus::Proposed,
            votes_for: 0,
            votes_against: 0,
            discussion_url: None,
        }
    }

    /// 将提案状态推进到审查中
    pub fn start_review(&mut self) {
        self.status = ProposalStatus::UnderReview;
    }

    /// 投赞成票
    pub fn vote_for(&mut self) {
        self.votes_for += 1;
    }

    /// 投反对票
    pub fn vote_against(&mut self) {
        self.votes_against += 1;
    }

    /// 设置讨论链接
    pub fn set_discussion_url(&mut self, url: String) {
        self.discussion_url = Some(url);
    }
}

// ============================================================
// 已接受的演化结构体
// ============================================================

/// 已接受的演化 — 道的演化历史记录
///
/// 当一份提案被接受后，它被记录为一次"演化"。
/// 每次演化都包含新的哲学映射、代码变更摘要和迁移指南，
/// 形成完整的知识传承链。
///
/// 道枢映射：演化记录是"道"的时间维度——道不是静止的，
/// 而是在时间中展开的。每一次演化都是"道"在新的历史条件
/// 下的显现。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptedEvolution {
    /// 演化 ID
    pub evolution_id: String,
    /// 来源提案 ID
    pub from_proposal: String,
    /// 接受时间戳（毫秒）
    pub accepted_at_ms: u64,
    /// 新的哲学映射：如 @涌现: 新模式名称
    pub new_philosophical_mapping: String,
    /// 代码变更摘要
    pub code_changes_summary: String,
    /// 迁移指南
    pub migration_guide: String,
}

impl AcceptedEvolution {
    /// 从已接受的提案创建演化记录
    pub fn from_proposal(
        evolution_id: String,
        proposal: &DaoProposal,
        new_philosophical_mapping: String,
        code_changes_summary: String,
        migration_guide: String,
    ) -> Self {
        Self {
            evolution_id,
            from_proposal: proposal.proposal_id.clone(),
            accepted_at_ms: now_ms(),
            new_philosophical_mapping,
            code_changes_summary,
            migration_guide,
        }
    }
}

// ============================================================
// 道枢演化协议结构体
// ============================================================

/// 道枢演化协议 — 质疑五的核心实现
///
/// 道枢演化协议是系统的"演化管理中枢"，负责：
/// 1. 管理社区提案的生命周期
/// 2. 记录已接受的演化，追踪代际递增
/// 3. 维护双模价值宣言，确保系统不偏离"中宫"
///
/// 道枢映射：中宫（五）— 统摄八方，调和阴阳
///
/// 该协议是系统在时间维度上的"中宫"——它协调着实用主义
/// 和哲学主义两个方向的演化诉求，确保系统在演化过程中
/// 始终保持平衡，不偏向任何一方。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaoEvolutionProtocol {
    /// 协议版本
    pub protocol_version: String,
    /// 社区提案列表
    pub proposals: Vec<DaoProposal>,
    /// 已接受的演化记录
    pub accepted_evolutions: Vec<AcceptedEvolution>,
    /// 演化代际（从 0 开始，每接受一次重大演化 +1）
    pub evolution_generation: u64,
    /// 上次演化时间戳（毫秒）
    pub last_evolution_ms: u64,
}

impl DaoEvolutionProtocol {
    /// 创建新的道枢演化协议实例
    pub fn new(protocol_version: String) -> Self {
        Self {
            protocol_version,
            proposals: Vec::new(),
            accepted_evolutions: Vec::new(),
            evolution_generation: 0,
            last_evolution_ms: 0,
        }
    }

    /// 提交新提案
    pub fn submit_proposal(&mut self, proposal: DaoProposal) {
        self.proposals.push(proposal);
    }

    /// 接受提案并将其转化为一次演化
    ///
    /// 接受提案时：
    /// 1. 将提案状态标记为 Accepted
    /// 2. 创建 AcceptedEvolution 记录
    /// 3. 演化代际 +1
    /// 4. 更新上次演化时间戳
    pub fn accept_proposal(
        &mut self,
        proposal_id: &str,
        evolution_id: String,
        new_philosophical_mapping: String,
        code_changes_summary: String,
        migration_guide: String,
    ) -> Result<&AcceptedEvolution, String> {
        // 查找提案
        let proposal = self
            .proposals
            .iter_mut()
            .find(|p| p.proposal_id == proposal_id)
            .ok_or_else(|| format!("未找到提案: {}", proposal_id))?;

        // 检查提案状态
        if proposal.status == ProposalStatus::Accepted {
            return Err(format!("提案 {} 已被接受，不可重复接受", proposal_id));
        }
        if proposal.status == ProposalStatus::Rejected {
            return Err(format!("提案 {} 已被拒绝，不可接受", proposal_id));
        }

        // 标记提案为已接受
        proposal.status = ProposalStatus::Accepted;

        // 创建演化记录
        let evolution = AcceptedEvolution::from_proposal(
            evolution_id,
            proposal,
            new_philosophical_mapping,
            code_changes_summary,
            migration_guide,
        );

        let now = now_ms();

        // 演化代际递增
        self.evolution_generation += 1;
        self.last_evolution_ms = now;

        self.accepted_evolutions.push(evolution);

        self.accepted_evolutions
            .last()
            .ok_or_else(|| "Failed to retrieve the accepted evolution".to_string())
    }

    /// 拒绝提案
    pub fn reject_proposal(&mut self, proposal_id: &str) -> Result<(), String> {
        let proposal = self
            .proposals
            .iter_mut()
            .find(|p| p.proposal_id == proposal_id)
            .ok_or_else(|| format!("未找到提案: {}", proposal_id))?;

        if proposal.status == ProposalStatus::Accepted {
            return Err(format!("提案 {} 已被接受，不可拒绝", proposal_id));
        }

        proposal.status = ProposalStatus::Rejected;
        Ok(())
    }

    /// 获取指定提案
    pub fn get_proposal(&self, proposal_id: &str) -> Option<&DaoProposal> {
        self.proposals.iter().find(|p| p.proposal_id == proposal_id)
    }

    /// 获取所有待审查的提案
    pub fn get_pending_proposals(&self) -> Vec<&DaoProposal> {
        self.proposals
            .iter()
            .filter(|p| {
                p.status == ProposalStatus::Proposed || p.status == ProposalStatus::UnderReview
            })
            .collect()
    }

    /// 获取演化历史
    pub fn get_evolution_history(&self) -> &[AcceptedEvolution] {
        &self.accepted_evolutions
    }

    /// 生成双模价值宣言
    pub fn generate_dual_mode_declaration(&self) -> DualModeValueDeclaration {
        DualModeValueDeclaration::new(self.evolution_generation)
    }
}

// ============================================================
// 实用价值结构体
// ============================================================

/// 实用价值 — 面向用户的价值承诺
///
/// 这些承诺是面向"工具使用者"的——用户不需要理解内部的
/// 洛书八卦、道枢演化，他们只需要知道系统能提供什么。
///
/// 道枢映射：这是"中宫"面向"阳"的一面——外向、实用、可见。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PragmaticValue {
    /// 检索准确率承诺
    pub retrieval_accuracy: String,
    /// 响应延迟承诺
    pub response_latency: String,
    /// 数据隐私承诺
    pub data_privacy: String,
    /// 简洁原则：用户不需要理解内部哲学即可使用
    pub simplicity_principle: String,
}

impl Default for PragmaticValue {
    fn default() -> Self {
        Self {
            retrieval_accuracy: "始终返回最相关的结果，检索准确率不低于基线".to_string(),
            response_latency: "在保证质量的前提下，优化响应延迟至可交互阈值".to_string(),
            data_privacy: "用户数据本地处理，不上传至云端，不用于训练外部模型".to_string(),
            simplicity_principle: "用户无需理解洛书八卦或道枢演化即可使用——工具的本分是服务"
                .to_string(),
        }
    }
}

// ============================================================
// 哲学价值结构体
// ============================================================

/// 哲学价值 — 面向思想同道的价值承诺
///
/// 这些承诺是面向"道"的追寻者——他们关心系统如何在工程中
/// 承载哲学思想，如何让"道"在代码中传承。
///
/// 道枢映射：这是"中宫"面向"阴"的一面——内向、深邃、不可见。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhilosophicalValue {
    /// 道的承诺：系统如何承载"道"的思想
    pub dao_commitment: String,
    /// 演化原则：道如何随时间演化
    pub evolution_principle: String,
    /// 社区治理模式
    pub community_governance: String,
    /// 知识传承：哲学思想如何通过代码传承
    pub knowledge_heritage: String,
}

impl Default for PhilosophicalValue {
    fn default() -> Self {
        Self {
            dao_commitment: "系统的每一个工程特性都映射到八卦体系中的某一卦或其交互关系，\
                 确保代码不是哲学的附庸，而是哲学在工程领域的自然涌现。\
                 道枢演化协议确保这种映射关系随时间保持鲜活。"
                .to_string(),
            evolution_principle: "道法自然——演化不是外部赋加的规划，而是从系统内在需求中\
                 自然生发。每一次演化都必须同时具备哲学基础（从洛书八卦中推导）\
                 和实际收益（对系统有可验证的改进）。"
                .to_string(),
            community_governance: "社区提案制——任何对'道'有理解的人都可以提交演化提案，\
                 经过社区审查和投票后决定是否接受。这确保了'道'不是少数人的\
                 垄断，而是集体智慧的结晶。"
                .to_string(),
            knowledge_heritage: "代码即经典——每一行代码都是'道'的一次书写，每一次演化都被\
                 记录在演化历史中，形成可追溯的知识传承链。新加入的开发者\
                 可以通过阅读演化历史来理解系统的哲学根源。"
                .to_string(),
        }
    }
}

// ============================================================
// 双模价值宣言
// ============================================================

/// 双模价值宣言 — 系统的"中宫"声明
///
/// 双模价值宣言是系统的身份宣言，它明确回答"我们是谁"的问题：
/// - 我们是一个工具，服务于用户的实际需求
/// - 我们是一座庙宇，承载着"道"的思想传承
/// - 我们同时是两者，因为实用与哲学并非对立，而是阴阳互济
///
/// 道枢映射：中宫（五）— 统摄八方，调和阴阳
///
/// 中宫之数五，是洛书九宫的中心。它不偏向任何一方的极端，
/// 而是调和阴阳，让实用主义者和哲学主义者都能在系统中
/// 找到归属。双模价值宣言就是这份"调和"的书面表达。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DualModeValueDeclaration {
    /// 实用模式：面向用户的价值
    pub pragmatic_mode: PragmaticValue,
    /// 哲学模式：面向思想同道
    pub philosophical_mode: PhilosophicalValue,
    /// 综合声明：调和两者的桥梁
    pub synthesis_statement: String,
    /// 生成时的演化代际
    pub generation_at_creation: u64,
}

impl DualModeValueDeclaration {
    /// 创建新的双模价值宣言
    pub fn new(generation: u64) -> Self {
        Self {
            pragmatic_mode: PragmaticValue::default(),
            philosophical_mode: PhilosophicalValue::default(),
            synthesis_statement: Self::default_synthesis_statement(),
            generation_at_creation: generation,
        }
    }

    /// 默认的综合声明
    fn default_synthesis_statement() -> String {
        concat!(
            "道枢演化协议（DaoEvolutionProtocol）是 LRC 系统的'中宫'——\n",
            "它不偏向实用主义或哲学主义的任何一方，而是在两者之间建立桥梁。\n\n",
            "实用模式是'阳'：面向用户，提供准确、快速、隐私安全的检索服务。\n",
            "用户无需理解洛书八卦或道枢演化即可使用——工具的本分是服务。\n\n",
            "哲学模式是'阴'：面向思想同道，承载'道'的思想传承与演化。\n",
            "每一个工程特性都有其道枢映射，每一次演化都被记录在案。\n\n",
            "两者并非对立，而是阴阳互济——\n",
            "实用是哲学的落地验证，哲学是实用的方向指引。\n",
            "没有实用的哲学是空中楼阁，没有哲学的实用是盲人摸象。\n\n",
            "这就是 LRC 对质疑五的回答：\n",
            "系统既是服务用户的工具，也是承载哲学的庙宇——\n",
            "正如中宫之数五，既是数，也是道。"
        )
        .to_string()
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试：创建提案
    #[test]
    fn test_create_proposal() {
        let proposal = DaoProposal::new(
            "prop_001".to_string(),
            "引入八卦权重自适应调节".to_string(),
            "当检索结果中某一卦的比例过高时，自动降低该卦的权重，鼓励探索其他卦象。".to_string(),
            "loong_ps".to_string(),
            "艮卦·山 (☶) — 时止则止，时行则行。当某一卦象权重过高时，系统应如艮卦所示，'止'于这个方向，转而行于其他方向。".to_string(),
            "用户将获得更多样化的检索结果，避免信息茧房。开发者将获得一个更均衡的记忆分布。".to_string(),
        );

        assert_eq!(proposal.proposal_id, "prop_001");
        assert_eq!(proposal.title, "引入八卦权重自适应调节");
        assert_eq!(proposal.proposed_by, "loong_ps");
        assert_eq!(proposal.status, ProposalStatus::Proposed);
        assert_eq!(proposal.votes_for, 0);
        assert_eq!(proposal.votes_against, 0);
        assert!(proposal.proposed_at_ms > 0);
        assert!(!proposal.philosophical_basis.is_empty());
        assert!(!proposal.practical_benefit.is_empty());
        assert!(proposal.discussion_url.is_none());
    }

    /// 测试：提案投票和状态流转
    #[test]
    fn test_proposal_voting_and_review() {
        let mut proposal = DaoProposal::new(
            "prop_002".to_string(),
            "测试提案".to_string(),
            "测试描述".to_string(),
            "tester".to_string(),
            "哲学基础".to_string(),
            "实际收益".to_string(),
        );

        // 初始状态应为 Proposed
        assert_eq!(proposal.status, ProposalStatus::Proposed);

        // 进入审查
        proposal.start_review();
        assert_eq!(proposal.status, ProposalStatus::UnderReview);

        // 投票
        proposal.vote_for();
        proposal.vote_for();
        proposal.vote_for();
        proposal.vote_against();

        assert_eq!(proposal.votes_for, 3);
        assert_eq!(proposal.votes_against, 1);
    }

    /// 测试：演化接受流程
    #[test]
    fn test_accept_evolution() {
        let mut protocol = DaoEvolutionProtocol::new("1.0.0".to_string());

        // 提交提案
        let proposal = DaoProposal::new(
            "prop_003".to_string(),
            "引入五行相生相克记忆淘汰机制".to_string(),
            "基于五行相生相克原理，当某一类记忆过多时，通过'相克'关系自动淘汰旧记忆。".to_string(),
            "loong_ps".to_string(),
            "五行相生相克——《黄帝内经》中的五行理论可映射到记忆管理：木生火（新记忆衍生关联），火生土（关联记忆沉淀为知识），土生金（知识结晶化为核心记忆），金生水（核心记忆引导新学习），水生木（新学习产生新记忆）。相克则用于淘汰：木克土（新记忆淘汰过时知识），水克火（新学习淘汰旧关联）。".to_string(),
            "用户将获得更智能的记忆淘汰机制，自动清理过时信息，释放存储空间。开发者将获得一个基于中国古典哲学的记忆管理框架。".to_string(),
        );

        protocol.submit_proposal(proposal);

        assert_eq!(protocol.proposals.len(), 1);
        assert_eq!(protocol.evolution_generation, 0);
        assert_eq!(protocol.last_evolution_ms, 0);

        // 接受提案
        let result = protocol.accept_proposal(
            "prop_003",
            "evo_001".to_string(),
            "@五行相克: 记忆淘汰机制".to_string(),
            "新增 memory_wuxing.rs 模块，实现五行相生相克的记忆生命周期管理。核心变更：在 memory_gc.rs 中新增 wuxing_gc 方法，淘汰策略从纯时间衰减升级为五行相克驱动。".to_string(),
            "1. 更新配置：在 config.toml 中添加 [wuxing] 配置段\n2. 运行迁移脚本：cargo run --bin migrate-wuxing\n3. 验证：运行 cargo test -- wuxing 确认所有测试通过".to_string(),
        );

        assert!(result.is_ok());
        let evolution = result.unwrap();

        assert_eq!(evolution.evolution_id, "evo_001");
        assert_eq!(evolution.from_proposal, "prop_003");
        assert!(evolution.accepted_at_ms > 0);
        assert_eq!(protocol.evolution_generation, 1);
        assert!(protocol.last_evolution_ms > 0);
        assert_eq!(protocol.accepted_evolutions.len(), 1);

        // 验证提案状态已更新
        let proposal = protocol.get_proposal("prop_003").unwrap();
        assert_eq!(proposal.status, ProposalStatus::Accepted);
    }

    /// 测试：重复接受同一提案应报错
    #[test]
    fn test_cannot_accept_twice() {
        let mut protocol = DaoEvolutionProtocol::new("1.0.0".to_string());

        let proposal = DaoProposal::new(
            "prop_004".to_string(),
            "测试".to_string(),
            "测试".to_string(),
            "tester".to_string(),
            "哲学".to_string(),
            "收益".to_string(),
        );

        protocol.submit_proposal(proposal);

        // 第一次接受
        let result1 = protocol.accept_proposal(
            "prop_004",
            "evo_002".to_string(),
            "@test: 测试映射".to_string(),
            "测试变更".to_string(),
            "测试指南".to_string(),
        );
        assert!(result1.is_ok());

        // 第二次接受应报错
        let result2 = protocol.accept_proposal(
            "prop_004",
            "evo_003".to_string(),
            "@test: 重复映射".to_string(),
            "重复变更".to_string(),
            "重复指南".to_string(),
        );
        assert!(result2.is_err());
        assert!(result2.unwrap_err().contains("已被接受"));
    }

    /// 测试：拒绝已接受的提案应报错
    #[test]
    fn test_cannot_reject_accepted() {
        let mut protocol = DaoEvolutionProtocol::new("1.0.0".to_string());

        let proposal = DaoProposal::new(
            "prop_005".to_string(),
            "测试".to_string(),
            "测试".to_string(),
            "tester".to_string(),
            "哲学".to_string(),
            "收益".to_string(),
        );

        protocol.submit_proposal(proposal);

        // 先接受
        let _ = protocol.accept_proposal(
            "prop_005",
            "evo_004".to_string(),
            "@test: 测试".to_string(),
            "变更".to_string(),
            "指南".to_string(),
        );

        // 再拒绝应报错
        let result = protocol.reject_proposal("prop_005");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("已被接受"));
    }

    /// 测试：拒绝提案
    #[test]
    fn test_reject_proposal() {
        let mut protocol = DaoEvolutionProtocol::new("1.0.0".to_string());

        let proposal = DaoProposal::new(
            "prop_006".to_string(),
            "测试".to_string(),
            "测试".to_string(),
            "tester".to_string(),
            "哲学".to_string(),
            "收益".to_string(),
        );

        protocol.submit_proposal(proposal);

        let result = protocol.reject_proposal("prop_006");
        assert!(result.is_ok());

        let proposal = protocol.get_proposal("prop_006").unwrap();
        assert_eq!(proposal.status, ProposalStatus::Rejected);
    }

    /// 测试：双模价值宣言生成
    #[test]
    fn test_dual_mode_declaration() {
        let protocol = DaoEvolutionProtocol::new("1.0.0".to_string());

        let declaration = protocol.generate_dual_mode_declaration();

        // 验证实用模式
        assert!(!declaration.pragmatic_mode.retrieval_accuracy.is_empty());
        assert!(!declaration.pragmatic_mode.response_latency.is_empty());
        assert!(!declaration.pragmatic_mode.data_privacy.is_empty());
        assert!(!declaration.pragmatic_mode.simplicity_principle.is_empty());

        // 验证哲学模式
        assert!(!declaration.philosophical_mode.dao_commitment.is_empty());
        assert!(!declaration
            .philosophical_mode
            .evolution_principle
            .is_empty());
        assert!(!declaration
            .philosophical_mode
            .community_governance
            .is_empty());
        assert!(!declaration.philosophical_mode.knowledge_heritage.is_empty());

        // 验证综合声明
        assert!(!declaration.synthesis_statement.is_empty());
        // 综合声明应包含关键概念
        assert!(declaration.synthesis_statement.contains("中宫"));
        assert!(declaration.synthesis_statement.contains("阳"));
        assert!(declaration.synthesis_statement.contains("阴"));
        assert!(declaration.synthesis_statement.contains("工具"));
        assert!(declaration.synthesis_statement.contains("庙宇"));

        // 验证代际
        assert_eq!(declaration.generation_at_creation, 0);
    }

    /// 测试：演化代际递增
    #[test]
    fn test_evolution_generation_increment() {
        let mut protocol = DaoEvolutionProtocol::new("1.0.0".to_string());

        // 初始代际为 0
        assert_eq!(protocol.evolution_generation, 0);

        // 提交并接受多个提案，验证代际递增
        for i in 1..=5 {
            let proposal = DaoProposal::new(
                format!("prop_{:03}", i),
                format!("演化提案 {}", i),
                format!("描述 {}", i),
                "loong_ps".to_string(),
                format!("哲学基础 {}", i),
                format!("实际收益 {}", i),
            );

            protocol.submit_proposal(proposal);

            let result = protocol.accept_proposal(
                &format!("prop_{:03}", i),
                format!("evo_{:03}", i),
                format!("@演化{}: 新映射", i),
                format!("变更 {}", i),
                format!("指南 {}", i),
            );

            assert!(result.is_ok());
            assert_eq!(protocol.evolution_generation, i as u64);
        }

        // 验证演化历史记录数量
        assert_eq!(protocol.accepted_evolutions.len(), 5);
        assert_eq!(protocol.get_evolution_history().len(), 5);

        // 验证代际递增后的宣言
        let declaration = protocol.generate_dual_mode_declaration();
        assert_eq!(declaration.generation_at_creation, 5);
    }

    /// 测试：待审查提案过滤
    #[test]
    fn test_get_pending_proposals() {
        let mut protocol = DaoEvolutionProtocol::new("1.0.0".to_string());

        // 创建三个提案
        for i in 1..=3 {
            let proposal = DaoProposal::new(
                format!("prop_{:03}", i),
                format!("提案 {}", i),
                format!("描述 {}", i),
                "tester".to_string(),
                format!("哲学 {}", i),
                format!("收益 {}", i),
            );
            protocol.submit_proposal(proposal);
        }

        // 接受一个，拒绝一个，保留一个
        let _ = protocol.accept_proposal(
            "prop_001",
            "evo_001".to_string(),
            "@test: 1".to_string(),
            "变更".to_string(),
            "指南".to_string(),
        );

        let _ = protocol.reject_proposal("prop_002");

        // 待审查的只有 prop_003
        let pending = protocol.get_pending_proposals();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].proposal_id, "prop_003");
    }

    /// 测试：DiscussionStatus 的 Display 实现
    #[test]
    fn test_proposal_status_display() {
        assert_eq!(format!("{}", ProposalStatus::Proposed), "已提出");
        assert_eq!(format!("{}", ProposalStatus::UnderReview), "审查中");
        assert_eq!(format!("{}", ProposalStatus::Accepted), "已接受");
        assert_eq!(format!("{}", ProposalStatus::Rejected), "已拒绝");
    }
}
