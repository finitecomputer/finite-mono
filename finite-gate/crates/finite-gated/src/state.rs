use crate::{config::GateConfig, limiter::PublicRouteLimiter};
pub struct GateState {
    pub config: GateConfig,
    pub limiter: PublicRouteLimiter,
}
impl GateState {
    pub fn new(config: GateConfig) -> Self {
        Self {
            config,
            limiter: PublicRouteLimiter::default(),
        }
    }
}
