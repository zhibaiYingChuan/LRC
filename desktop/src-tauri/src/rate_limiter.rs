/// L3 运行时保护：请求频率限制
///
/// 使用令牌桶（Token Bucket）算法实现每 Agent 每秒最多 100 次请求。
/// 超过限制返回 should_throttle() = true，调用方应返回 HTTP 429。
///
/// 安全级别：L3（运行时保护层）
/// PRD 对应：L3-03 请求频率限制
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// 令牌桶配置
#[derive(Debug, Clone)]
pub struct RateLimiterConfig {
    /// 令牌补充速率（每秒）
    pub rate: f64,
    /// 桶容量（最大突发请求数）
    pub burst: u32,
}

impl Default for RateLimiterConfig {
    fn default() -> Self {
        Self {
            rate: 100.0, // 每秒 100 个请求
            burst: 20,   // 允许 20 个突发请求
        }
    }
}

/// 单个令牌桶的状态
#[derive(Debug)]
struct TokenBucket {
    /// 当前令牌数
    tokens: f64,
    /// 上次补充令牌的时间
    last_refill: Instant,
    /// 配置
    config: RateLimiterConfig,
}

impl TokenBucket {
    fn new(config: RateLimiterConfig) -> Self {
        Self {
            tokens: config.burst as f64,
            last_refill: Instant::now(),
            config,
        }
    }

    /// 尝试消费一个令牌
    ///
    /// 返回 true 表示允许请求，false 表示被限流。
    fn try_consume(&mut self) -> bool {
        self.refill();

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// 补充令牌（基于经过的时间）
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.last_refill = now;

        // 补充令牌 = 经过时间 × 速率
        self.tokens = (self.tokens + elapsed * self.config.rate).min(self.config.burst as f64);
    }
}

/// 全局频率限制器
///
/// 为每个 Agent（通过 key 标识）维护独立的令牌桶。
/// 线程安全：使用 &mut self，调用方负责同步。
#[derive(Debug)]
pub struct RateLimiter {
    /// Agent key → 令牌桶
    buckets: HashMap<String, TokenBucket>,
    /// 默认配置
    config: RateLimiterConfig,
}

impl RateLimiter {
    /// 创建新的频率限制器
    pub fn new(config: RateLimiterConfig) -> Self {
        Self {
            buckets: HashMap::new(),
            config,
        }
    }
}

/// 使用默认配置创建（100 req/s, burst=20）
impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(RateLimiterConfig::default())
    }
}

impl RateLimiter {
    /// 检查请求是否被限流
    ///
    /// # 参数
    /// - `client_key`: 客户端标识（如 Agent ID 或 IP）
    ///
    /// # 返回
    /// - `true`：请求被限流，应返回 HTTP 429
    /// - `false`：请求通过
    pub fn should_throttle(&mut self, client_key: &str) -> bool {
        let bucket = self
            .buckets
            .entry(client_key.to_string())
            .or_insert_with(|| TokenBucket::new(self.config.clone()));

        !bucket.try_consume()
    }

    /// 获取当前桶中的令牌数（用于监控/调试）
    pub fn available_tokens(&mut self, client_key: &str) -> f64 {
        let bucket = self
            .buckets
            .entry(client_key.to_string())
            .or_insert_with(|| TokenBucket::new(self.config.clone()));
        bucket.refill();
        bucket.tokens
    }

    /// 清理长时间未活跃的桶（防止内存泄漏）
    pub fn cleanup(&mut self, max_age: Duration) {
        let now = Instant::now();
        self.buckets
            .retain(|_, bucket| now.duration_since(bucket.last_refill) < max_age);
    }

    /// 获取活跃桶数量（用于监控）
    pub fn active_buckets(&self) -> usize {
        self.buckets.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TDD：新创建的限流器应允许前 N 个突发请求
    #[test]
    fn test_initial_burst_allowed() {
        let mut limiter = RateLimiter::new(RateLimiterConfig {
            rate: 100.0,
            burst: 10,
        });

        for i in 0..10 {
            assert!(!limiter.should_throttle("agent-1"), "第 {i} 个请求应被允许");
        }
    }

    /// TDD：超过 burst 的请求应被限流
    #[test]
    fn test_throttle_after_burst_exhausted() {
        let mut limiter = RateLimiter::new(RateLimiterConfig {
            rate: 100.0,
            burst: 3,
        });

        // 前 3 个请求通过
        for _ in 0..3 {
            assert!(!limiter.should_throttle("agent-1"));
        }
        // 第 4 个被限流
        assert!(limiter.should_throttle("agent-1"), "第 4 个请求应被限流");
    }

    /// TDD：不同客户端使用独立桶，互不影响
    #[test]
    fn test_independent_buckets() {
        let mut limiter = RateLimiter::new(RateLimiterConfig {
            rate: 100.0,
            burst: 1,
        });

        // agent-1 消耗唯一令牌
        assert!(!limiter.should_throttle("agent-1"));
        // agent-1 被限流
        assert!(limiter.should_throttle("agent-1"));
        // agent-2 仍有令牌
        assert!(!limiter.should_throttle("agent-2"));
    }

    /// TDD：令牌应随时间自然补充
    #[test]
    fn test_token_refill_over_time() {
        let mut limiter = RateLimiter::new(RateLimiterConfig {
            rate: 1000.0, // 每秒 1000 个（非常大，确保微秒级补充）
            burst: 1,
        });

        // 消耗唯一令牌
        assert!(!limiter.should_throttle("agent-1"));
        assert!(limiter.should_throttle("agent-1"));

        // 等待 2ms（1000/s = 1ms 补充 1 个令牌）
        std::thread::sleep(Duration::from_millis(2));

        // 应补充了至少 2 个令牌
        assert!(!limiter.should_throttle("agent-1"), "等待后应有令牌可用");
    }

    /// TDD：cleanup 清理过期桶
    #[test]
    fn test_cleanup_removes_stale_buckets() {
        let mut limiter = RateLimiter::new(RateLimiterConfig {
            rate: 100.0,
            burst: 5,
        });

        // 创建 agent-1 的桶
        limiter.should_throttle("agent-1");
        assert_eq!(limiter.active_buckets(), 1);

        // 清理 0ms 以上的桶（应清空所有）
        limiter.cleanup(Duration::from_millis(0));
        assert_eq!(limiter.active_buckets(), 0, "过期桶应被清理");
    }

    /// TDD：模拟 100+ 请求压力测试，验证限流生效
    ///
    /// PRD L3-03 验收标准：超过 100 req/s 返回 429（should_throttle = true）
    #[test]
    fn test_stress_100_requests_per_second() {
        let mut limiter = RateLimiter::new(RateLimiterConfig {
            rate: 100.0,
            burst: 100, // 允许首批 100 个请求
        });

        // 首批 100 个请求应全部通过
        let mut passed = 0;
        for _ in 0..100 {
            if !limiter.should_throttle("agent-stress") {
                passed += 1;
            }
        }
        assert_eq!(passed, 100, "首批 100 个请求应全部通过");

        // 第 101 个请求应被限流
        assert!(
            limiter.should_throttle("agent-stress"),
            "第 101 个请求应被限流（超过 burst=100）"
        );
    }

    /// TDD：多客户端并发压力测试
    #[test]
    fn test_stress_multi_client() {
        let mut limiter = RateLimiter::new(RateLimiterConfig {
            rate: 100.0,
            burst: 5,
        });

        // 10 个客户端各发 5 个请求，应互不影响
        for client_id in 0..10 {
            let key = format!("client-{client_id}");
            let mut client_passed = 0;
            for _ in 0..5 {
                if !limiter.should_throttle(&key) {
                    client_passed += 1;
                }
            }
            assert_eq!(client_passed, 5, "客户端 {client_id} 的 5 个请求应全部通过");
            // 第 6 个应被限流
            assert!(
                limiter.should_throttle(&key),
                "客户端 {client_id} 第 6 个请求应被限流"
            );
        }

        assert_eq!(limiter.active_buckets(), 10, "应有 10 个活跃桶");
    }
}
