use crate::*;

pub(crate) async fn root_handler() -> &'static str {
    "FiniteBrain Rust smoke server"
}

pub(crate) async fn health_handler() -> Json<HealthStatus> {
    Json(health_status())
}
