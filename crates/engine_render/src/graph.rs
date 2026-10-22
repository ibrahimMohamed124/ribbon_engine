//! Ordered, validated render pass graph.

#[cfg(feature = "gpu")]
use crate::gpu_window::FrameTarget;
#[cfg(feature = "gpu")]
use crate::profiling::{RenderGraphTiming, RenderPassTiming};
#[cfg(feature = "gpu")]
use engine_app::World;
use std::any::{Any, TypeId};
use std::collections::{BTreeSet, HashMap};
use std::fmt;
#[cfg(feature = "gpu")]
use std::time::Instant;

/// Error returned by a rendering node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderError(pub String);

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RenderError {}

/// A render execution context with typed, backend-neutral attachments.
#[derive(Default)]
pub struct RenderContext {
    values: HashMap<TypeId, Box<dyn Any>>,
}

impl RenderContext {
    /// Insert a context value, returning its previous value if one existed.
    pub fn insert<T: Any>(&mut self, value: T) -> Option<T> {
        self.values
            .insert(TypeId::of::<T>(), Box::new(value))
            .and_then(|old| old.downcast::<T>().ok().map(|boxed| *boxed))
    }

    /// Get a typed context value.
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.values.get(&TypeId::of::<T>())?.downcast_ref()
    }

    /// Mutably get a typed context value.
    pub fn get_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.values.get_mut(&TypeId::of::<T>())?.downcast_mut()
    }
}

/// One ordered render pass.
pub trait RenderNode: Send + Sync {
    /// Stable diagnostic name.
    fn name(&self) -> &str;
    /// Named resources consumed by this pass.
    fn reads(&self) -> &[&str];
    /// Named resources produced by this pass.
    fn writes(&self) -> &[&str];

    /// Owned read names; override this when resource names depend on node state.
    fn read_resources(&self) -> Vec<String> {
        self.reads().iter().map(|name| (*name).to_owned()).collect()
    }

    /// Owned write names; override this when resource names depend on node state.
    fn write_resources(&self) -> Vec<String> {
        self.writes()
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    /// Execute the pass in a backend-neutral context.
    fn run(&mut self, context: &mut RenderContext) -> Result<(), RenderError>;

    /// Execute the pass as part of a GPU window frame. Existing graph nodes remain
    /// backend-neutral and use `run`; GPU render nodes can override this hook.
    #[cfg(feature = "gpu")]
    fn run_frame(
        &mut self,
        context: &mut RenderContext,
        _world: &mut World,
        _target: &FrameTarget<'_>,
    ) -> Result<(), RenderError> {
        self.run(context)
    }
}

/// Validation errors for the render graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderGraphError {
    /// A named resource has no producer in this graph.
    UnwrittenRead { node: String, resource: String },
    /// Pass dependencies form a cycle.
    Cycle { nodes: Vec<String> },
}

impl fmt::Display for RenderGraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnwrittenRead { node, resource } => {
                write!(
                    f,
                    "render node `{node}` reads unwritten resource `{resource}`"
                )
            }
            Self::Cycle { nodes } => write!(f, "render graph cycle: {}", nodes.join(", ")),
        }
    }
}

impl std::error::Error for RenderGraphError {}

/// Stable insertion-ordered collection of render nodes.
#[derive(Default)]
pub struct RenderGraph {
    nodes: Vec<Box<dyn RenderNode>>,
    order: Vec<usize>,
}

impl RenderGraph {
    /// Add a node and invalidate the previously computed order.
    pub fn add_node(&mut self, node: impl RenderNode + 'static) {
        self.nodes.push(Box::new(node));
        self.order.clear();
    }

    /// Validate resource producers and compute a stable topological order.
    pub fn build(&mut self) -> Result<(), RenderGraphError> {
        let mut writers: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, node) in self.nodes.iter().enumerate() {
            for resource in node.write_resources() {
                writers.entry(resource).or_default().push(index);
            }
        }
        let mut edges: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); self.nodes.len()];
        let mut indegree = vec![0; self.nodes.len()];
        for (reader, node) in self.nodes.iter().enumerate() {
            for resource in node.read_resources() {
                let Some(producers) = writers.get(&resource) else {
                    return Err(RenderGraphError::UnwrittenRead {
                        node: node.name().to_owned(),
                        resource,
                    });
                };
                for &producer in producers {
                    if producer != reader && edges[producer].insert(reader) {
                        indegree[reader] += 1;
                    }
                }
            }
        }
        let mut ready: BTreeSet<_> = indegree
            .iter()
            .enumerate()
            .filter_map(|(index, &degree)| (degree == 0).then_some(index))
            .collect();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(index) = ready.pop_first() {
            order.push(index);
            for &target in &edges[index] {
                indegree[target] -= 1;
                if indegree[target] == 0 {
                    ready.insert(target);
                }
            }
        }
        if order.len() != self.nodes.len() {
            return Err(RenderGraphError::Cycle {
                nodes: indegree
                    .iter()
                    .enumerate()
                    .filter(|(_, degree)| **degree > 0)
                    .map(|(index, _)| self.nodes[index].name().to_owned())
                    .collect(),
            });
        }
        self.order = order;
        Ok(())
    }

    /// Execute all nodes in the validated order.
    pub fn run(&mut self, context: &mut RenderContext) -> Result<(), RenderError> {
        if self.order.len() != self.nodes.len() {
            self.build()
                .map_err(|error| RenderError(error.to_string()))?;
        }
        for &index in &self.order {
            self.nodes[index].run(context)?;
        }
        Ok(())
    }

    /// Execute the same validated graph for one native GPU frame. The ECS world and
    /// frame target are borrowed only for this call and are never stored in resources.
    #[cfg(feature = "gpu")]
    pub fn run_frame(
        &mut self,
        context: &mut RenderContext,
        world: &mut World,
        target: &FrameTarget<'_>,
    ) -> Result<(), RenderError> {
        if self.order.len() != self.nodes.len() {
            self.build()
                .map_err(|error| RenderError(error.to_string()))?;
        }
        for &index in &self.order {
            self.nodes[index].run_frame(context, world, target)?;
        }
        Ok(())
    }

    /// Execute one native GPU frame and return CPU timing for every graph node.
    ///
    /// These durations measure CPU-side invocation/encoding, not asynchronous GPU execution.
    /// Backends with timestamp queries should publish their GPU samples separately.
    #[cfg(feature = "gpu")]
    pub fn run_frame_profiled(
        &mut self,
        context: &mut RenderContext,
        world: &mut World,
        target: &FrameTarget<'_>,
    ) -> Result<RenderGraphTiming, RenderError> {
        if self.order.len() != self.nodes.len() {
            self.build()
                .map_err(|error| RenderError(error.to_string()))?;
        }
        let graph_started = Instant::now();
        let mut timing = RenderGraphTiming::default();
        timing.passes.reserve(self.order.len());
        for &index in &self.order {
            let node_started = Instant::now();
            self.nodes[index].run_frame(context, world, target)?;
            timing.passes.push(RenderPassTiming {
                name: self.nodes[index].name().to_owned(),
                cpu_time_ns: node_started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64,
            });
        }
        timing.cpu_time_ns = graph_started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        Ok(timing)
    }
}

#[cfg(test)]
mod tests {
    use super::{RenderContext, RenderError, RenderGraph, RenderGraphError, RenderNode};
    use std::sync::{Arc, Mutex};

    struct TestNode {
        name: &'static str,
        reads: &'static [&'static str],
        writes: &'static [&'static str],
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl RenderNode for TestNode {
        fn name(&self) -> &str {
            self.name
        }

        fn reads(&self) -> &[&str] {
            self.reads
        }

        fn writes(&self) -> &[&str] {
            self.writes
        }

        fn run(&mut self, _context: &mut RenderContext) -> Result<(), RenderError> {
            self.log.lock().unwrap().push(self.name);
            Ok(())
        }
    }

    #[test]
    fn graph_orders_passes_by_written_resources_and_stable_ties() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut graph = RenderGraph::default();
        graph.add_node(TestNode {
            name: "present",
            reads: &["color"],
            writes: &["surface"],
            log: Arc::clone(&log),
        });
        graph.add_node(TestNode {
            name: "scene",
            reads: &[],
            writes: &["color"],
            log: Arc::clone(&log),
        });
        graph.add_node(TestNode {
            name: "ui",
            reads: &[],
            writes: &["ui"],
            log: Arc::clone(&log),
        });
        graph.build().unwrap();
        graph.run(&mut RenderContext::default()).unwrap();
        assert_eq!(*log.lock().unwrap(), ["scene", "present", "ui"]);
    }

    #[test]
    fn graph_reports_cycles_and_unwritten_resources() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut graph = RenderGraph::default();
        graph.add_node(TestNode {
            name: "read_a",
            reads: &["b"],
            writes: &["a"],
            log: Arc::clone(&log),
        });
        graph.add_node(TestNode {
            name: "read_b",
            reads: &["a"],
            writes: &["b"],
            log: Arc::clone(&log),
        });
        assert!(
            matches!(graph.build(), Err(RenderGraphError::Cycle { nodes }) if nodes == ["read_a", "read_b"])
        );

        let mut graph = RenderGraph::default();
        graph.add_node(TestNode {
            name: "orphan",
            reads: &["missing"],
            writes: &[],
            log,
        });
        assert_eq!(
            graph.build(),
            Err(RenderGraphError::UnwrittenRead {
                node: "orphan".to_owned(),
                resource: "missing".to_owned()
            })
        );
    }

    #[test]
    fn empty_graph_runs_headless() {
        RenderGraph::default()
            .run(&mut RenderContext::default())
            .unwrap();
    }
}
