//! Reusable gameplay AI: finite state machines, behavior trees, navigation, and sight.
pub mod agent;
pub mod behavior_tree;
pub mod blackboard;
pub mod fsm;
pub mod nav;
pub mod perception;
pub mod plugin;
pub mod rng;
#[cfg(test)]
mod tests;
pub use agent::{NavAgent, NavDestination, NavSettings, NavStatus, NavSteering};
pub use behavior_tree::{BehaviorTree, BtContext, BtError, BtNode, BtRegistry, BtStatus};
pub use blackboard::{BbValue, Blackboard};
pub use fsm::{register_state_machine, StateChanged, StateMachine};
pub use nav::{bake_nav_grid, NavBakeConfig, NavError, NavGrid, NavPath, NavWorld};
pub use perception::{AiSettings, Perceived, Sight, SightTarget};
pub use plugin::AiPlugin;
