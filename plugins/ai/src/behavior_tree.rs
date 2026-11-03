//! Deterministic behavior-tree nodes and runtime evaluation.
use crate::Blackboard;
use ribbon_app::{LogLevel, World};
use ribbon_ecs::Entity;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BtStatus {
    Success,
    Failure,
    Running,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BtNode {
    Sequence(Vec<BtNode>),
    Selector(Vec<BtNode>),
    Inverter(Box<BtNode>),
    Succeeder(Box<BtNode>),
    Repeat { child: Box<BtNode>, times: u32 },
    Wait { seconds: f32 },
    Leaf(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BtError {
    Ron(String),
    UnknownLeaf(String),
}
impl BtNode {
    pub fn from_ron(src: &str) -> Result<Self, BtError> {
        ron::from_str(src).map_err(|e| BtError::Ron(e.to_string()))
    }
}
pub struct BtContext<'a> {
    pub world: &'a mut World,
    pub entity: Entity,
    pub dt: f32,
    pub blackboard: &'a mut Blackboard,
}
pub type BtLeaf = Box<dyn Fn(&mut BtContext<'_>) -> BtStatus + Send + Sync>;
#[derive(Default)]
pub struct BtRegistry {
    leaves: HashMap<String, BtLeaf>,
}
impl BtRegistry {
    pub fn register(
        &mut self,
        name: &str,
        f: impl Fn(&mut BtContext<'_>) -> BtStatus + Send + Sync + 'static,
    ) {
        self.leaves.insert(name.into(), Box::new(f));
    }
}
#[derive(Clone, Debug, Default)]
struct NodeState {
    cursor: usize,
    elapsed: f32,
}
pub struct BehaviorTree {
    pub root: Arc<BtNode>,
    pub blackboard: Blackboard,
    states: Vec<NodeState>,
    root_ptr: usize,
    unknown_logged: HashSet<String>,
}
impl BehaviorTree {
    pub fn new(root: BtNode) -> Self {
        Self {
            root: Arc::new(root),
            blackboard: Blackboard::default(),
            states: Vec::new(),
            root_ptr: 0,
            unknown_logged: HashSet::new(),
        }
    }
    pub fn validate(&self, registry: &BtRegistry) -> Result<(), BtError> {
        validate_node(&self.root, registry)
    }
    pub fn tick(
        &mut self,
        world: &mut World,
        registry: &BtRegistry,
        entity: Entity,
        dt: f32,
    ) -> BtStatus {
        let ptr = Arc::as_ptr(&self.root) as usize;
        if ptr != self.root_ptr {
            self.states = vec![NodeState::default(); count_nodes(&self.root)];
            self.root_ptr = ptr;
        }
        let mut index = 0;
        let mut ctx = BtContext {
            world,
            entity,
            dt: dt.max(0.0),
            blackboard: &mut self.blackboard,
        };
        let status = eval(
            &self.root,
            &mut self.states,
            &mut index,
            &mut ctx,
            registry,
            &mut self.unknown_logged,
        );
        if status != BtStatus::Running {
            self.states
                .iter_mut()
                .for_each(|s| *s = NodeState::default());
        }
        status
    }
}
fn count_nodes(n: &BtNode) -> usize {
    1 + match n {
        BtNode::Sequence(v) | BtNode::Selector(v) => v.iter().map(count_nodes).sum(),
        BtNode::Inverter(c) | BtNode::Succeeder(c) | BtNode::Repeat { child: c, .. } => {
            count_nodes(c)
        }
        _ => 0,
    }
}
fn validate_node(n: &BtNode, registry: &BtRegistry) -> Result<(), BtError> {
    match n {
        BtNode::Leaf(name) if !registry.leaves.contains_key(name) => {
            Err(BtError::UnknownLeaf(name.clone()))
        }
        BtNode::Sequence(v) | BtNode::Selector(v) => {
            for c in v {
                validate_node(c, registry)?;
            }
            Ok(())
        }
        BtNode::Inverter(c) | BtNode::Succeeder(c) | BtNode::Repeat { child: c, .. } => {
            validate_node(c, registry)
        }
        _ => Ok(()),
    }
}
fn eval(
    n: &BtNode,
    states: &mut [NodeState],
    index: &mut usize,
    ctx: &mut BtContext<'_>,
    registry: &BtRegistry,
    logged: &mut HashSet<String>,
) -> BtStatus {
    let own = *index;
    *index += 1;
    match n {
        BtNode::Leaf(name) => match registry.leaves.get(name) {
            Some(leaf) => leaf(ctx),
            None => {
                if logged.insert(name.clone()) {
                    ctx.world.resource_mut::<ribbon_app::LogBuffer>().push(
                        LogLevel::Error,
                        format!("unknown behavior-tree leaf `{name}`"),
                    );
                }
                BtStatus::Failure
            }
        },
        BtNode::Wait { seconds } => {
            let state = &mut states[own];
            state.elapsed += ctx.dt;
            if state.elapsed >= seconds.max(0.0) {
                state.elapsed = 0.0;
                BtStatus::Success
            } else {
                BtStatus::Running
            }
        }
        BtNode::Sequence(children) => {
            if children.is_empty() {
                return BtStatus::Success;
            }
            let mut cursor = states[own].cursor.min(children.len());
            *index += children.iter().take(cursor).map(count_nodes).sum::<usize>();
            while cursor < children.len() {
                let status = eval(&children[cursor], states, index, ctx, registry, logged);
                match status {
                    BtStatus::Success => cursor += 1,
                    BtStatus::Failure => {
                        states[own].cursor = 0;
                        return BtStatus::Failure;
                    }
                    BtStatus::Running => {
                        states[own].cursor = cursor;
                        return BtStatus::Running;
                    }
                }
            }
            states[own].cursor = 0;
            BtStatus::Success
        }
        BtNode::Selector(children) => {
            if children.is_empty() {
                return BtStatus::Failure;
            }
            let mut cursor = states[own].cursor.min(children.len());
            *index += children.iter().take(cursor).map(count_nodes).sum::<usize>();
            while cursor < children.len() {
                let status = eval(&children[cursor], states, index, ctx, registry, logged);
                match status {
                    BtStatus::Failure => cursor += 1,
                    BtStatus::Success => {
                        states[own].cursor = 0;
                        return BtStatus::Success;
                    }
                    BtStatus::Running => {
                        states[own].cursor = cursor;
                        return BtStatus::Running;
                    }
                }
            }
            states[own].cursor = 0;
            BtStatus::Failure
        }
        BtNode::Inverter(child) => match eval(child, states, index, ctx, registry, logged) {
            BtStatus::Success => BtStatus::Failure,
            BtStatus::Failure => BtStatus::Success,
            BtStatus::Running => BtStatus::Running,
        },
        BtNode::Succeeder(child) => {
            let _ = eval(child, states, index, ctx, registry, logged);
            BtStatus::Success
        }
        BtNode::Repeat { child, times } => {
            let child_start = *index;
            if *times == 0 {
                let status = eval(child, states, index, ctx, registry, logged);
                *index = child_start + count_nodes(child);
                if status == BtStatus::Failure {
                    return status;
                }
                reset_subtree(child, states, own + 1);
                BtStatus::Running
            } else {
                let mut completed = 0;
                while completed < *times {
                    *index = child_start;
                    let status = eval(child, states, index, ctx, registry, logged);
                    *index = child_start + count_nodes(child);
                    match status {
                        BtStatus::Failure => return status,
                        BtStatus::Running => return status,
                        BtStatus::Success => {
                            completed += 1;
                            reset_subtree(child, states, own + 1);
                        }
                    }
                }
                BtStatus::Success
            }
        }
    }
}
fn reset_subtree(n: &BtNode, states: &mut [NodeState], start: usize) {
    let count = count_nodes(n);
    if let Some(slice) = states.get_mut(start..start.saturating_add(count)) {
        slice.iter_mut().for_each(|s| *s = NodeState::default());
    }
}

pub(crate) fn bt_system(world: &mut World) {
    let dt = world
        .try_resource::<ribbon_app::Time>()
        .map_or(0.0, |t| t.delta.as_secs_f32().max(0.0));
    let Some(registry) = world.remove_resource::<BtRegistry>() else {
        return;
    };
    let mut entities = world
        .query::<&BehaviorTree>()
        .iter()
        .map(|(e, _)| e)
        .collect::<Vec<_>>();
    entities.sort_by_key(|e| e.to_bits());
    for entity in entities {
        if let Ok(mut tree) = world.remove::<BehaviorTree>(entity) {
            let _ = tree.tick(world, &registry, entity, dt);
            if world.contains(entity) {
                let _ = world.insert(entity, tree);
            }
        }
    }
    world.insert_resource(registry);
}
