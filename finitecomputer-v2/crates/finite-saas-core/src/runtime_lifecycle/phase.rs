use super::RuntimeLifecycleStage;

#[derive(Debug, Clone, Copy)]
pub struct Requested;
#[derive(Debug, Clone, Copy)]
pub struct Launching;
#[derive(Debug, Clone, Copy)]
pub struct ComputeUp;
#[derive(Debug, Clone, Copy)]
pub struct Ready;
#[derive(Debug, Clone, Copy)]
pub struct Succeeded;
#[derive(Debug, Clone, Copy)]
pub struct Stopped;
#[derive(Debug, Clone, Copy)]
pub struct Failed {
    pub stage: RuntimeLifecycleStage,
}
