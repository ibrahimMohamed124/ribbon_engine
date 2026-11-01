//! Validated runtime mixer graph for user-defined buses and non-hierarchical sends.
//!
//! The graph is backend-neutral. The Kira backend mirrors it with real mixer tracks/send tracks;
//! this type owns validation, IDs, hierarchy invariants and effective-gain calculation.

use std::collections::{HashMap, HashSet};

use crate::registry::AudioBus;

pub const MAX_CUSTOM_BUSES: usize = 16;
pub const MAX_SEND_TRACKS: usize = 16;
pub const MAX_SENDS_PER_BUS: usize = 8;
const MAX_NAME_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CustomBusId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AudioSendId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MixerBusParent {
    BuiltIn(AudioBus),
    Custom(CustomBusId),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixerSendRoute {
    pub send: AudioSendId,
    /// Linear send gain in `[0, 2]`.
    pub gain: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CustomBusConfig {
    pub id: CustomBusId,
    pub name: String,
    pub parent: MixerBusParent,
    pub gain: f32,
    pub muted: bool,
    pub sends: Vec<MixerSendRoute>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AudioSendEffect {
    Reverb {
        wet: f32,
        decay_seconds: f32,
    },
    Delay {
        delay_seconds: f32,
        feedback_db: f32,
        wet: f32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioSendConfig {
    pub id: AudioSendId,
    pub name: String,
    pub effect: AudioSendEffect,
    pub gain: f32,
}

#[derive(Clone, Debug)]
pub struct MixerBusNode {
    pub config: CustomBusConfig,
}

#[derive(Clone, Default)]
pub struct MixerGraph {
    buses: HashMap<CustomBusId, MixerBusNode>,
    sends: HashMap<AudioSendId, AudioSendConfig>,
}

impl MixerGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_send(&mut self, config: AudioSendConfig) -> Result<(), String> {
        validate_name("send", &config.name)?;
        if self.sends.len() >= MAX_SEND_TRACKS {
            return Err(format!("mixer is limited to {MAX_SEND_TRACKS} send tracks"));
        }
        if self.sends.contains_key(&config.id) {
            return Err(format!("send ID {:?} is already registered", config.id));
        }
        if !config.gain.is_finite() || !(0.0..=2.0).contains(&config.gain) {
            return Err("send gain must be finite and within [0, 2]".into());
        }
        match config.effect {
            AudioSendEffect::Reverb { wet, decay_seconds }
                if !wet.is_finite()
                    || !(0.0..=1.0).contains(&wet)
                    || !decay_seconds.is_finite()
                    || !(0.0..=10.0).contains(&decay_seconds) =>
            {
                return Err("send reverb parameters are outside their supported ranges".into());
            }
            AudioSendEffect::Delay {
                delay_seconds,
                feedback_db,
                wet,
            } if !delay_seconds.is_finite()
                || !(0.01..=1.0).contains(&delay_seconds)
                || !feedback_db.is_finite()
                || !(-60.0..=0.0).contains(&feedback_db)
                || !wet.is_finite()
                || !(0.0..=1.0).contains(&wet) =>
            {
                return Err("send delay parameters are outside their supported ranges".into());
            }
            _ => {}
        }
        self.sends.insert(config.id, config);
        Ok(())
    }

    pub fn remove_send(&mut self, id: AudioSendId) -> Result<AudioSendConfig, String> {
        if self
            .buses
            .values()
            .any(|node| node.config.sends.iter().any(|route| route.send == id))
        {
            return Err(format!(
                "send {:?} is still routed from one or more buses",
                id
            ));
        }
        self.sends
            .remove(&id)
            .ok_or_else(|| format!("unknown send {:?}", id))
    }

    pub fn create_bus(&mut self, config: CustomBusConfig) -> Result<(), String> {
        validate_name("bus", &config.name)?;
        if self.buses.len() >= MAX_CUSTOM_BUSES {
            return Err(format!(
                "mixer is limited to {MAX_CUSTOM_BUSES} custom buses"
            ));
        }
        if self.buses.contains_key(&config.id) {
            return Err(format!(
                "custom bus ID {:?} is already registered",
                config.id
            ));
        }
        self.validate_bus_config(&config)?;
        if let MixerBusParent::Custom(parent) = config.parent {
            if !self.buses.contains_key(&parent) {
                return Err(format!("custom parent bus {:?} does not exist", parent));
            }
        }
        self.buses.insert(config.id, MixerBusNode { config });
        Ok(())
    }

    pub fn update_bus(&mut self, config: CustomBusConfig) -> Result<(), String> {
        if !self.buses.contains_key(&config.id) {
            return Err(format!("unknown custom bus {:?}", config.id));
        }
        validate_name("bus", &config.name)?;
        self.validate_bus_config(&config)?;
        if let MixerBusParent::Custom(parent) = config.parent {
            if !self.buses.contains_key(&parent) {
                return Err(format!("custom parent bus {:?} does not exist", parent));
            }
            if parent == config.id || self.is_descendant(parent, config.id) {
                return Err("mixer bus hierarchy cannot contain a cycle".into());
            }
        }
        self.buses.insert(config.id, MixerBusNode { config });
        Ok(())
    }

    pub fn remove_bus(&mut self, id: CustomBusId) -> Result<CustomBusConfig, String> {
        if self
            .buses
            .values()
            .any(|node| node.config.parent == MixerBusParent::Custom(id))
        {
            return Err(format!("custom bus {:?} still has child buses", id));
        }
        self.buses
            .remove(&id)
            .map(|node| node.config)
            .ok_or_else(|| format!("unknown custom bus {:?}", id))
    }

    pub fn set_send_gain(
        &mut self,
        bus: CustomBusId,
        send: AudioSendId,
        gain: f32,
    ) -> Result<(), String> {
        if !gain.is_finite() || !(0.0..=2.0).contains(&gain) {
            return Err("send gain must be finite and within [0, 2]".into());
        }
        let node = self
            .buses
            .get_mut(&bus)
            .ok_or_else(|| format!("unknown custom bus {:?}", bus))?;
        let route = node
            .config
            .sends
            .iter_mut()
            .find(|route| route.send == send)
            .ok_or_else(|| format!("bus {:?} has no route to send {:?}", bus, send))?;
        route.gain = gain;
        Ok(())
    }

    pub fn bus(&self, id: CustomBusId) -> Option<&CustomBusConfig> {
        self.buses.get(&id).map(|node| &node.config)
    }

    pub fn set_bus_gain(&mut self, id: CustomBusId, gain: f32) -> Result<(), String> {
        if !gain.is_finite() || !(0.0..=2.0).contains(&gain) {
            return Err("custom bus gain must be finite and within [0, 2]".into());
        }
        let node = self
            .buses
            .get_mut(&id)
            .ok_or_else(|| format!("unknown custom bus {:?}", id))?;
        node.config.gain = gain;
        Ok(())
    }

    pub fn set_bus_muted(&mut self, id: CustomBusId, muted: bool) -> Result<(), String> {
        let node = self
            .buses
            .get_mut(&id)
            .ok_or_else(|| format!("unknown custom bus {:?}", id))?;
        node.config.muted = muted;
        Ok(())
    }

    pub fn set_send_master_gain(&mut self, id: AudioSendId, gain: f32) -> Result<(), String> {
        if !gain.is_finite() || !(0.0..=2.0).contains(&gain) {
            return Err("send master gain must be finite and within [0, 2]".into());
        }
        let send = self
            .sends
            .get_mut(&id)
            .ok_or_else(|| format!("unknown send {:?}", id))?;
        send.gain = gain;
        Ok(())
    }

    pub fn send(&self, id: AudioSendId) -> Option<&AudioSendConfig> {
        self.sends.get(&id)
    }
    pub fn buses(&self) -> impl Iterator<Item = &CustomBusConfig> {
        self.buses.values().map(|node| &node.config)
    }
    pub fn sends(&self) -> impl Iterator<Item = &AudioSendConfig> {
        self.sends.values()
    }

    /// Returns true when `candidate` is this bus or any nested child bus below it.
    pub fn contains_bus_in_subtree(&self, ancestor: CustomBusId, candidate: CustomBusId) -> bool {
        let mut current = candidate;
        let mut visited = HashSet::new();
        loop {
            if current == ancestor {
                return self.buses.contains_key(&current);
            }
            if !visited.insert(current) {
                return false;
            }
            match self.buses.get(&current).map(|node| node.config.parent) {
                Some(MixerBusParent::Custom(parent)) => current = parent,
                Some(MixerBusParent::BuiltIn(_)) | None => return false,
            }
        }
    }

    /// Collect the bus IDs affected by a parent-bus control. This is called on the mixer owner
    /// thread, never from the audio render callback.
    pub fn buses_in_subtree(&self, ancestor: CustomBusId) -> Vec<CustomBusId> {
        self.buses
            .keys()
            .copied()
            .filter(|candidate| self.contains_bus_in_subtree(ancestor, *candidate))
            .collect()
    }

    pub fn root_audio_bus(&self, id: CustomBusId) -> Result<AudioBus, String> {
        let mut current = id;
        let mut visited = HashSet::new();
        loop {
            if !visited.insert(current) {
                return Err("mixer bus hierarchy contains a cycle".into());
            }
            let node = self
                .buses
                .get(&current)
                .ok_or_else(|| format!("unknown custom bus {:?}", current))?;
            match node.config.parent {
                MixerBusParent::BuiltIn(bus) => return Ok(bus),
                MixerBusParent::Custom(parent) => current = parent,
            }
        }
    }

    pub fn effective_gain(&self, id: CustomBusId) -> Result<f32, String> {
        let mut current = id;
        let mut gain = 1.0f32;
        let mut visited = HashSet::new();
        loop {
            if !visited.insert(current) {
                return Err("mixer bus hierarchy contains a cycle".into());
            }
            let node = self
                .buses
                .get(&current)
                .ok_or_else(|| format!("unknown custom bus {:?}", current))?;
            gain *= if node.config.muted {
                0.0
            } else {
                node.config.gain
            };
            match node.config.parent {
                MixerBusParent::BuiltIn(_) => return Ok(gain),
                MixerBusParent::Custom(parent) => current = parent,
            }
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        for node in self.buses.values() {
            self.validate_bus_config(&node.config)?;
        }
        for id in self.buses.keys().copied() {
            let _ = self.effective_gain(id)?;
        }
        Ok(())
    }

    fn validate_bus_config(&self, config: &CustomBusConfig) -> Result<(), String> {
        if !config.gain.is_finite() || !(0.0..=2.0).contains(&config.gain) {
            return Err("custom bus gain must be finite and within [0, 2]".into());
        }
        if config.sends.len() > MAX_SENDS_PER_BUS {
            return Err(format!(
                "a custom bus may route to at most {MAX_SENDS_PER_BUS} sends"
            ));
        }
        let mut seen = HashSet::with_capacity(config.sends.len());
        for route in &config.sends {
            if !seen.insert(route.send) {
                return Err("custom bus contains a duplicate send route".into());
            }
            if !self.sends.contains_key(&route.send) {
                return Err(format!("unknown send {:?}", route.send));
            }
            if !route.gain.is_finite() || !(0.0..=2.0).contains(&route.gain) {
                return Err("send route gain must be finite and within [0, 2]".into());
            }
        }
        Ok(())
    }

    fn is_descendant(&self, candidate: CustomBusId, ancestor: CustomBusId) -> bool {
        let mut current = candidate;
        let mut visited = HashSet::new();
        loop {
            if current == ancestor {
                return true;
            }
            if !visited.insert(current) {
                return true;
            }
            match self.buses.get(&current).map(|node| node.config.parent) {
                Some(MixerBusParent::Custom(parent)) => current = parent,
                _ => return false,
            }
        }
    }
}

fn validate_name(kind: &str, name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > MAX_NAME_BYTES {
        return Err(format!(
            "{kind} name must contain 1..={MAX_NAME_BYTES} bytes"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus(id: u64, parent: MixerBusParent, gain: f32) -> CustomBusConfig {
        CustomBusConfig {
            id: CustomBusId(id),
            name: format!("bus-{id}"),
            parent,
            gain,
            muted: false,
            sends: Vec::new(),
        }
    }

    #[test]
    fn custom_bus_hierarchy_composes_gain_and_mute() {
        let mut graph = MixerGraph::new();
        graph
            .create_bus(bus(1, MixerBusParent::BuiltIn(AudioBus::Sfx), 0.5))
            .unwrap();
        graph
            .create_bus(bus(2, MixerBusParent::Custom(CustomBusId(1)), 0.4))
            .unwrap();
        assert!((graph.effective_gain(CustomBusId(2)).unwrap() - 0.2).abs() < 1.0e-6);
        let mut muted = bus(1, MixerBusParent::BuiltIn(AudioBus::Sfx), 0.5);
        muted.muted = true;
        graph.update_bus(muted).unwrap();
        assert_eq!(graph.effective_gain(CustomBusId(2)).unwrap(), 0.0);
    }

    #[test]
    fn hierarchy_rejects_cycles_and_removal_with_children() {
        let mut graph = MixerGraph::new();
        graph
            .create_bus(bus(1, MixerBusParent::BuiltIn(AudioBus::Sfx), 1.0))
            .unwrap();
        graph
            .create_bus(bus(2, MixerBusParent::Custom(CustomBusId(1)), 1.0))
            .unwrap();
        assert!(graph.remove_bus(CustomBusId(1)).is_err());
        let mut cyclic = bus(1, MixerBusParent::Custom(CustomBusId(2)), 1.0);
        assert!(graph.update_bus(cyclic.clone()).is_err());
        cyclic.parent = MixerBusParent::BuiltIn(AudioBus::Master);
        graph.update_bus(cyclic).unwrap();
        graph.validate().unwrap();
    }

    #[test]
    fn subtree_membership_includes_nested_buses_but_not_siblings() {
        let mut graph = MixerGraph::new();
        graph
            .create_bus(bus(1, MixerBusParent::BuiltIn(AudioBus::Sfx), 1.0))
            .unwrap();
        graph
            .create_bus(bus(2, MixerBusParent::Custom(CustomBusId(1)), 1.0))
            .unwrap();
        graph
            .create_bus(bus(3, MixerBusParent::Custom(CustomBusId(2)), 1.0))
            .unwrap();
        graph
            .create_bus(bus(4, MixerBusParent::BuiltIn(AudioBus::Music), 1.0))
            .unwrap();
        assert!(graph.contains_bus_in_subtree(CustomBusId(1), CustomBusId(1)));
        assert!(graph.contains_bus_in_subtree(CustomBusId(1), CustomBusId(3)));
        assert!(!graph.contains_bus_in_subtree(CustomBusId(2), CustomBusId(1)));
        assert!(!graph.contains_bus_in_subtree(CustomBusId(1), CustomBusId(4)));
    }

    #[test]
    fn send_routes_are_validated_and_cannot_be_removed_while_used() {
        let mut graph = MixerGraph::new();
        let send = AudioSendConfig {
            id: AudioSendId(7),
            name: "shared-reverb".into(),
            effect: AudioSendEffect::Reverb {
                wet: 1.0,
                decay_seconds: 2.0,
            },
            gain: 1.0,
        };
        graph.create_send(send).unwrap();
        let mut config = bus(1, MixerBusParent::BuiltIn(AudioBus::Sfx), 1.0);
        config.sends.push(MixerSendRoute {
            send: AudioSendId(7),
            gain: 0.5,
        });
        graph.create_bus(config).unwrap();
        assert!(graph.remove_send(AudioSendId(7)).is_err());
        assert!(graph
            .set_send_gain(CustomBusId(1), AudioSendId(7), 0.25)
            .is_ok());
    }
}
