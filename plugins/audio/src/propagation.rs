//! Material- and portal-aware acoustic propagation.
//!
//! This graph complements `AudioOcclusionQuery`: games populate it from their room/door system,
//! then query the best transmission path between acoustic zones. It does not ray-cast arbitrary
//! triangle meshes; geometry/visibility remains the responsibility of the scene query provider.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

pub const MAX_ACOUSTIC_ZONES: usize = 256;
pub const MAX_ACOUSTIC_PORTALS: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AcousticZoneId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AcousticPortalId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AcousticMaterial {
    /// Linear sound transmission coefficient `[0, 1]`.
    pub transmission_gain: f32,
    /// Highest frequency that remains relatively unattenuated through the material.
    pub low_pass_cutoff_hz: f32,
}

impl AcousticMaterial {
    pub fn sanitized(self) -> Self {
        Self {
            transmission_gain: finite(self.transmission_gain, 0.0, 1.0, 0.0),
            low_pass_cutoff_hz: finite(self.low_pass_cutoff_hz, 20.0, 20_000.0, 20.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoorPortalState {
    /// `0` is fully closed, `1` is fully open. Continuous values support partially open doors.
    pub open_fraction: f32,
    pub closed: AcousticMaterial,
    pub open: AcousticMaterial,
}

impl DoorPortalState {
    pub fn sanitized(self) -> Self {
        Self {
            open_fraction: finite(self.open_fraction, 0.0, 1.0, 0.0),
            closed: self.closed.sanitized(),
            open: self.open.sanitized(),
        }
    }

    fn response(self) -> AcousticMaterial {
        let state = self.sanitized();
        let t = state.open_fraction;
        AcousticMaterial {
            transmission_gain: state.closed.transmission_gain
                + (state.open.transmission_gain - state.closed.transmission_gain) * t,
            low_pass_cutoff_hz: state.closed.low_pass_cutoff_hz
                + (state.open.low_pass_cutoff_hz - state.closed.low_pass_cutoff_hz) * t,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortalAcousticEdge {
    pub id: AcousticPortalId,
    pub a: AcousticZoneId,
    pub b: AcousticZoneId,
    pub material: AcousticMaterial,
    pub door: Option<DoorPortalState>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AcousticPropagationResult {
    pub transmission_gain: f32,
    pub low_pass_cutoff_hz: f32,
    pub portal_path: Vec<AcousticPortalId>,
    pub reachable: bool,
}

impl AcousticPropagationResult {
    /// Fold a zone/portal path into the per-emitter controls consumed by the audio backend.
    /// Existing direct-material transmission is multiplied; the most restrictive low-pass wins.
    pub fn apply_to_managed_emitter(&self, acoustics: &mut crate::ManagedEmitterAcoustics) {
        if !self.reachable {
            acoustics.transmission_gain = 0.0;
            acoustics.low_pass_cutoff_hz = 20.0;
            return;
        }
        let transmission = finite(self.transmission_gain, 0.0, 1.0, 0.0);
        let cutoff = finite(self.low_pass_cutoff_hz, 20.0, 20_000.0, 20.0);
        // The direct-material values may originate in application physics code. Sanitize them
        // before composing so a NaN cannot poison the worker command or backend DSP parameters.
        let direct_transmission = finite(acoustics.transmission_gain, 0.0, 1.0, 0.0);
        let direct_cutoff = finite(acoustics.low_pass_cutoff_hz, 20.0, 20_000.0, 20_000.0);
        acoustics.transmission_gain = (direct_transmission * transmission).clamp(0.0, 1.0);
        acoustics.low_pass_cutoff_hz = direct_cutoff.min(cutoff).clamp(20.0, 20_000.0);
    }
}

#[derive(Default)]
pub struct AcousticPropagationGraph {
    zones: HashSet<AcousticZoneId>,
    portals: HashMap<AcousticPortalId, PortalAcousticEdge>,
    adjacency: HashMap<AcousticZoneId, Vec<AcousticPortalId>>,
    revision: u64,
}

#[derive(Clone, Copy)]
struct Candidate {
    cost: f64,
    zone: AcousticZoneId,
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cost.to_bits() == other.cost.to_bits() && self.zone == other.zone
    }
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max-heap; reverse cost for Dijkstra.
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| self.zone.0.cmp(&other.zone.0))
    }
}

impl AcousticPropagationGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_zone(&mut self, id: AcousticZoneId) -> Result<(), String> {
        if self.zones.contains(&id) {
            return Err(format!("acoustic zone {:?} already exists", id));
        }
        if self.zones.len() >= MAX_ACOUSTIC_ZONES {
            return Err("acoustic zone limit reached".into());
        }
        self.zones.insert(id);
        self.adjacency.entry(id).or_default();
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    pub fn remove_zone(&mut self, id: AcousticZoneId) -> Result<(), String> {
        if !self.zones.contains(&id) {
            return Err(format!("unknown acoustic zone {:?}", id));
        }
        let connected = self.adjacency.get(&id).cloned().unwrap_or_default();
        for portal in connected {
            self.remove_portal(portal)?;
        }
        self.adjacency.remove(&id);
        self.zones.remove(&id);
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    pub fn add_portal(&mut self, edge: PortalAcousticEdge) -> Result<(), String> {
        if self.portals.len() >= MAX_ACOUSTIC_PORTALS {
            return Err("acoustic portal limit reached".into());
        }
        if self.portals.contains_key(&edge.id) {
            return Err(format!("portal {:?} already exists", edge.id));
        }
        if edge.a == edge.b {
            return Err("portal endpoints must be different zones".into());
        }
        if !self.zones.contains(&edge.a) || !self.zones.contains(&edge.b) {
            return Err("portal endpoints must reference existing zones".into());
        }
        let edge = PortalAcousticEdge {
            material: edge.material.sanitized(),
            door: edge.door.map(DoorPortalState::sanitized),
            ..edge
        };
        self.adjacency.entry(edge.a).or_default().push(edge.id);
        self.adjacency.entry(edge.b).or_default().push(edge.id);
        self.portals.insert(edge.id, edge);
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    pub fn update_portal(&mut self, edge: PortalAcousticEdge) -> Result<(), String> {
        if !self.portals.contains_key(&edge.id) {
            return Err(format!("unknown portal {:?}", edge.id));
        }
        if edge.a == edge.b || !self.zones.contains(&edge.a) || !self.zones.contains(&edge.b) {
            return Err("portal endpoints must reference two different existing zones".into());
        }
        let previous = self.portals.get(&edge.id).copied().expect("checked above");
        if previous.a != edge.a || previous.b != edge.b {
            self.adjacency
                .entry(previous.a)
                .or_default()
                .retain(|id| *id != edge.id);
            self.adjacency
                .entry(previous.b)
                .or_default()
                .retain(|id| *id != edge.id);
            self.adjacency.entry(edge.a).or_default().push(edge.id);
            self.adjacency.entry(edge.b).or_default().push(edge.id);
        }
        self.portals.insert(
            edge.id,
            PortalAcousticEdge {
                material: edge.material.sanitized(),
                door: edge.door.map(DoorPortalState::sanitized),
                ..edge
            },
        );
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    pub fn remove_portal(&mut self, id: AcousticPortalId) -> Result<(), String> {
        let edge = self
            .portals
            .remove(&id)
            .ok_or_else(|| format!("unknown portal {:?}", id))?;
        if let Some(edges) = self.adjacency.get_mut(&edge.a) {
            edges.retain(|candidate| *candidate != id);
        }
        if let Some(edges) = self.adjacency.get_mut(&edge.b) {
            edges.retain(|candidate| *candidate != id);
        }
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    pub fn query(
        &self,
        from: AcousticZoneId,
        to: AcousticZoneId,
    ) -> Result<AcousticPropagationResult, String> {
        if !self.zones.contains(&from) || !self.zones.contains(&to) {
            return Err("propagation query references an unknown acoustic zone".into());
        }
        if from == to {
            return Ok(AcousticPropagationResult {
                transmission_gain: 1.0,
                low_pass_cutoff_hz: 20_000.0,
                portal_path: Vec::new(),
                reachable: true,
            });
        }
        let mut distances: HashMap<AcousticZoneId, f64> = HashMap::new();
        let mut previous: HashMap<AcousticZoneId, (AcousticZoneId, AcousticPortalId)> =
            HashMap::new();
        let mut queue = BinaryHeap::new();
        distances.insert(from, 0.0);
        queue.push(Candidate {
            cost: 0.0,
            zone: from,
        });
        while let Some(Candidate { cost, zone }) = queue.pop() {
            if zone == to {
                break;
            }
            if cost > distances.get(&zone).copied().unwrap_or(f64::INFINITY) {
                continue;
            }
            for portal_id in self.adjacency.get(&zone).into_iter().flatten() {
                let edge = self
                    .portals
                    .get(portal_id)
                    .expect("adjacency references known portal");
                let next = if edge.a == zone { edge.b } else { edge.a };
                let material = edge
                    .door
                    .map(DoorPortalState::response)
                    .unwrap_or(edge.material)
                    .sanitized();
                // A fully closed/opaque portal is not an acoustically traversable edge. Skipping
                // zero gain also prevents it tying with a real but extremely quiet path after an
                // arbitrary epsilon clamp.
                if material.transmission_gain <= 0.0 {
                    continue;
                }
                // Minimize negative log transmission, so paths with higher total gain are selected.
                let next_cost = cost - (material.transmission_gain as f64).ln();
                if next_cost < distances.get(&next).copied().unwrap_or(f64::INFINITY) {
                    distances.insert(next, next_cost);
                    previous.insert(next, (zone, *portal_id));
                    queue.push(Candidate {
                        cost: next_cost,
                        zone: next,
                    });
                }
            }
        }
        if !distances.contains_key(&to) {
            return Ok(AcousticPropagationResult {
                transmission_gain: 0.0,
                low_pass_cutoff_hz: 20.0,
                portal_path: Vec::new(),
                reachable: false,
            });
        }
        let mut path = Vec::new();
        let mut current = to;
        let mut gain = 1.0f32;
        let mut cutoff = 20_000.0f32;
        while current != from {
            let (parent, portal_id) = previous
                .get(&current)
                .copied()
                .ok_or_else(|| "propagation predecessor chain is incomplete".to_owned())?;
            let edge = self
                .portals
                .get(&portal_id)
                .expect("path references known portal");
            let material = edge
                .door
                .map(DoorPortalState::response)
                .unwrap_or(edge.material)
                .sanitized();
            gain *= material.transmission_gain;
            cutoff = cutoff.min(material.low_pass_cutoff_hz);
            path.push(portal_id);
            current = parent;
        }
        path.reverse();
        Ok(AcousticPropagationResult {
            transmission_gain: gain.clamp(0.0, 1.0),
            low_pass_cutoff_hz: cutoff.clamp(20.0, 20_000.0),
            portal_path: path,
            reachable: true,
        })
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn zone_count(&self) -> usize {
        self.zones.len()
    }
    pub fn portal_count(&self) -> usize {
        self.portals.len()
    }
}

fn finite(value: f32, min: f32, max: f32, default: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn material(gain: f32, cutoff: f32) -> AcousticMaterial {
        AcousticMaterial {
            transmission_gain: gain,
            low_pass_cutoff_hz: cutoff,
        }
    }

    #[test]
    fn portal_query_composes_material_transmission_and_cutoff() {
        let mut graph = AcousticPropagationGraph::new();
        for id in 1..=3 {
            graph.add_zone(AcousticZoneId(id)).unwrap();
        }
        graph
            .add_portal(PortalAcousticEdge {
                id: AcousticPortalId(1),
                a: AcousticZoneId(1),
                b: AcousticZoneId(2),
                material: material(0.5, 4_000.0),
                door: None,
            })
            .unwrap();
        graph
            .add_portal(PortalAcousticEdge {
                id: AcousticPortalId(2),
                a: AcousticZoneId(2),
                b: AcousticZoneId(3),
                material: material(0.8, 8_000.0),
                door: None,
            })
            .unwrap();
        let path = graph.query(AcousticZoneId(1), AcousticZoneId(3)).unwrap();
        assert!((path.transmission_gain - 0.4).abs() < 1.0e-6);
        assert_eq!(path.low_pass_cutoff_hz, 4_000.0);
        assert_eq!(
            path.portal_path,
            vec![AcousticPortalId(1), AcousticPortalId(2)]
        );
    }

    #[test]
    fn open_door_increases_transmission_and_unblocks_high_frequencies() {
        let mut graph = AcousticPropagationGraph::new();
        graph.add_zone(AcousticZoneId(1)).unwrap();
        graph.add_zone(AcousticZoneId(2)).unwrap();
        let mut edge = PortalAcousticEdge {
            id: AcousticPortalId(5),
            a: AcousticZoneId(1),
            b: AcousticZoneId(2),
            material: material(0.1, 500.0),
            door: Some(DoorPortalState {
                open_fraction: 0.0,
                closed: material(0.05, 300.0),
                open: material(1.0, 20_000.0),
            }),
        };
        graph.add_portal(edge).unwrap();
        let closed = graph.query(AcousticZoneId(1), AcousticZoneId(2)).unwrap();
        edge.door.as_mut().unwrap().open_fraction = 1.0;
        graph.update_portal(edge).unwrap();
        let open = graph.query(AcousticZoneId(1), AcousticZoneId(2)).unwrap();
        assert!(open.transmission_gain > closed.transmission_gain);
        assert!(open.low_pass_cutoff_hz > closed.low_pass_cutoff_hz);
    }

    #[test]
    fn propagation_result_combines_with_managed_emitter_material_response() {
        let mut acoustics = crate::ManagedEmitterAcoustics {
            gain: 1.0,
            pan: 0.0,
            playback_rate: 1.0,
            occlusion: 0.25,
            transmission_gain: 0.8,
            low_pass_cutoff_hz: 8_000.0,
            #[cfg(feature = "steam-audio")]
            binaural_direction: None,
        };
        AcousticPropagationResult {
            transmission_gain: 0.5,
            low_pass_cutoff_hz: 2_000.0,
            portal_path: vec![AcousticPortalId(1)],
            reachable: true,
        }
        .apply_to_managed_emitter(&mut acoustics);
        assert!((acoustics.transmission_gain - 0.4).abs() < 1.0e-6);
        assert_eq!(acoustics.low_pass_cutoff_hz, 2_000.0);
    }

    #[test]
    fn propagation_sanitizes_non_finite_direct_material_inputs() {
        let mut acoustics = crate::ManagedEmitterAcoustics {
            gain: 1.0,
            pan: 0.0,
            playback_rate: 1.0,
            occlusion: 0.0,
            transmission_gain: f32::NAN,
            low_pass_cutoff_hz: f32::INFINITY,
            #[cfg(feature = "steam-audio")]
            binaural_direction: None,
        };
        AcousticPropagationResult {
            transmission_gain: 0.75,
            low_pass_cutoff_hz: 6_000.0,
            portal_path: vec![AcousticPortalId(4)],
            reachable: true,
        }
        .apply_to_managed_emitter(&mut acoustics);
        assert_eq!(acoustics.transmission_gain, 0.0);
        assert_eq!(acoustics.low_pass_cutoff_hz, 6_000.0);
        assert!(acoustics.transmission_gain.is_finite());
        assert!(acoustics.low_pass_cutoff_hz.is_finite());
    }

    #[test]
    fn unreachable_portal_path_mutes_managed_emitter() {
        let mut acoustics = crate::ManagedEmitterAcoustics {
            gain: 1.0,
            pan: 0.0,
            playback_rate: 1.0,
            occlusion: 0.0,
            transmission_gain: 1.0,
            low_pass_cutoff_hz: 20_000.0,
            #[cfg(feature = "steam-audio")]
            binaural_direction: None,
        };
        AcousticPropagationResult {
            transmission_gain: 0.0,
            low_pass_cutoff_hz: 20.0,
            portal_path: Vec::new(),
            reachable: false,
        }
        .apply_to_managed_emitter(&mut acoustics);
        assert_eq!(acoustics.transmission_gain, 0.0);
        assert_eq!(acoustics.low_pass_cutoff_hz, 20.0);
    }

    #[test]
    fn opaque_portal_does_not_hide_a_low_gain_but_audible_alternate_path() {
        let mut graph = AcousticPropagationGraph::new();
        for id in 1..=4 {
            graph.add_zone(AcousticZoneId(id)).unwrap();
        }
        graph
            .add_portal(PortalAcousticEdge {
                id: AcousticPortalId(1),
                a: AcousticZoneId(1),
                b: AcousticZoneId(4),
                material: material(0.0, 20.0),
                door: None,
            })
            .unwrap();
        graph
            .add_portal(PortalAcousticEdge {
                id: AcousticPortalId(2),
                a: AcousticZoneId(1),
                b: AcousticZoneId(2),
                material: material(0.01, 4_000.0),
                door: None,
            })
            .unwrap();
        graph
            .add_portal(PortalAcousticEdge {
                id: AcousticPortalId(3),
                a: AcousticZoneId(2),
                b: AcousticZoneId(4),
                material: material(0.01, 8_000.0),
                door: None,
            })
            .unwrap();
        let result = graph.query(AcousticZoneId(1), AcousticZoneId(4)).unwrap();
        assert!(result.reachable);
        assert!((result.transmission_gain - 0.0001).abs() < 1.0e-7);
        assert_eq!(
            result.portal_path,
            vec![AcousticPortalId(2), AcousticPortalId(3)]
        );
    }

    #[test]
    fn disconnected_zones_are_reported_as_unreachable() {
        let mut graph = AcousticPropagationGraph::new();
        graph.add_zone(AcousticZoneId(1)).unwrap();
        graph.add_zone(AcousticZoneId(2)).unwrap();
        assert!(
            !graph
                .query(AcousticZoneId(1), AcousticZoneId(2))
                .unwrap()
                .reachable
        );
    }
}
