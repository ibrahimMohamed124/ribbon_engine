use std::collections::BTreeMap;
use std::time::Duration;

/// Per-system timing from the most recent scheduled run.
#[derive(Default)]
pub struct ScheduleStats {
    /// Last observed duration by system name.
    pub systems: BTreeMap<String, Duration>,
}
