//! Per-frame particle simulation counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleBudget {
    pub max_total_particles: usize,
}
impl Default for ParticleBudget {
    fn default() -> Self {
        Self {
            max_total_particles: 200_000,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParticleDiagnostics {
    pub emitters: usize,
    pub alive_particles: usize,
    pub spawned_this_frame: usize,
    pub dropped_by_budget: u64,
    pub update_time_ns: u64,
}
