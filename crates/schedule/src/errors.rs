use super::Stage;
use std::fmt;

/// Typed errors produced when a schedule is built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScheduleError {
    /// An ordering constraint names a label not present in the same stage.
    UnknownLabel { system: String, label: String },
    /// System names must be unique throughout one schedule.
    DuplicateName { name: String },
    /// Labels must be unique within one stage.
    DuplicateLabel { stage: Stage, label: String },
    /// The ordering constraints contain a directed cycle.
    Cycle { stage: Stage, systems: Vec<String> },
    /// A parallel system must declare at least one component/resource access.
    MissingAccess { system: String },
    /// Access metadata only applies to access-declared parallel systems.
    AccessDeclarationOnExclusiveSystem { system: String },
    /// A system declared the same component/resource access more than once, even with a
    /// different read/write mode.
    DuplicateAccess { system: String, access: String },
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownLabel { system, label } => {
                write!(f, "system `{system}` references unknown label `{label}`")
            }
            Self::DuplicateName { name } => write!(f, "duplicate system name `{name}`"),
            Self::DuplicateLabel { stage, label } => {
                write!(f, "duplicate label `{label}` in {stage:?}")
            }
            Self::Cycle { stage, systems } => {
                write!(f, "ordering cycle in {stage:?}: {}", systems.join(", "))
            }
            Self::MissingAccess { system } => write!(
                f,
                "parallel system `{system}` must declare component/resource access"
            ),
            Self::AccessDeclarationOnExclusiveSystem { system } => write!(
                f,
                "exclusive system `{system}` must not declare parallel access metadata"
            ),
            Self::DuplicateAccess { system, access } => {
                write!(f, "system `{system}` declares `{access}` more than once")
            }
        }
    }
}

impl std::error::Error for ScheduleError {}
