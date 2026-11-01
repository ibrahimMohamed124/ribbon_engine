//! Platform-independent sound identifiers and sound-bank descriptors.

use std::collections::HashMap;

/// Opaque sound identifier owned by the application using the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SoundId(pub u32);

/// Opaque ambience identifier owned by the application using the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AmbienceId(pub u32);

#[derive(Clone, Copy)]
pub struct SoundAsset {
    pub id: SoundId,
    pub data: &'static [u8],
    pub gain: f32,
    pub speed: f32,
    pub spatial: bool,
}

#[derive(Clone, Copy)]
pub struct AmbienceAsset {
    pub id: AmbienceId,
    pub data: &'static [u8],
    pub gain: f32,
}

#[derive(Clone)]
pub struct SoundBank {
    sounds: Vec<SoundAsset>,
    ambiences: Vec<AmbienceAsset>,
    sound_index: HashMap<SoundId, usize>,
    ambience_index: HashMap<AmbienceId, usize>,
    duplicate_sound_ids: std::collections::HashSet<SoundId>,
    duplicate_ambience_ids: std::collections::HashSet<AmbienceId>,
}

impl SoundBank {
    pub fn new(sounds: Vec<SoundAsset>, ambiences: Vec<AmbienceAsset>) -> Self {
        // Preserve the legacy unchecked constructor but never pick an arbitrary winner for a
        // duplicate ID: ambiguous lookups become None and `validate` reports the duplicate.
        let mut sound_index = HashMap::with_capacity(sounds.len());
        let mut duplicate_sound_ids = std::collections::HashSet::new();
        for (index, asset) in sounds.iter().enumerate() {
            if duplicate_sound_ids.contains(&asset.id) {
                continue;
            }
            if sound_index.insert(asset.id, index).is_some() {
                sound_index.remove(&asset.id);
                duplicate_sound_ids.insert(asset.id);
            }
        }
        let mut ambience_index = HashMap::with_capacity(ambiences.len());
        let mut duplicate_ambience_ids = std::collections::HashSet::new();
        for (index, asset) in ambiences.iter().enumerate() {
            if duplicate_ambience_ids.contains(&asset.id) {
                continue;
            }
            if ambience_index.insert(asset.id, index).is_some() {
                ambience_index.remove(&asset.id);
                duplicate_ambience_ids.insert(asset.id);
            }
        }
        Self {
            sounds,
            ambiences,
            sound_index,
            ambience_index,
            duplicate_sound_ids,
            duplicate_ambience_ids,
        }
    }

    /// Construct a bank and reject duplicate IDs or invalid per-asset metadata.
    pub fn try_new(sounds: Vec<SoundAsset>, ambiences: Vec<AmbienceAsset>) -> Result<Self, String> {
        let bank = Self::new(sounds, ambiences);
        bank.validate()?;
        Ok(bank)
    }

    /// Validate uniqueness and metadata before handing a bank to a backend.
    pub fn validate(&self) -> Result<(), String> {
        use std::collections::HashSet;
        let mut sound_ids = HashSet::new();
        for asset in &self.sounds {
            if !sound_ids.insert(asset.id) {
                return Err(format!("duplicate sound id {:?}", asset.id));
            }
            if !asset.gain.is_finite() || !(0.0..=4.0).contains(&asset.gain) {
                return Err(format!("invalid gain for sound {:?}", asset.id));
            }
            if !asset.speed.is_finite() || !(0.25..=4.0).contains(&asset.speed) {
                return Err(format!("invalid playback speed for sound {:?}", asset.id));
            }
            if asset.data.is_empty() {
                return Err(format!("sound {:?} has no encoded data", asset.id));
            }
        }
        let mut ambience_ids = HashSet::new();
        for asset in &self.ambiences {
            if !ambience_ids.insert(asset.id) {
                return Err(format!("duplicate ambience id {:?}", asset.id));
            }
            if !asset.gain.is_finite() || !(0.0..=4.0).contains(&asset.gain) {
                return Err(format!("invalid gain for ambience {:?}", asset.id));
            }
            if asset.data.is_empty() {
                return Err(format!("ambience {:?} has no encoded data", asset.id));
            }
        }
        Ok(())
    }

    pub fn sound_count(&self) -> usize {
        self.sounds.len()
    }
    pub fn ambience_count(&self) -> usize {
        self.ambiences.len()
    }

    pub fn estimated_encoded_bytes(&self) -> usize {
        self.sounds
            .iter()
            .map(|asset| asset.data.len())
            .fold(0usize, usize::saturating_add)
            .saturating_add(
                self.ambiences
                    .iter()
                    .map(|asset| asset.data.len())
                    .fold(0usize, usize::saturating_add),
            )
    }

    pub fn sound(&self, id: SoundId) -> Option<SoundAsset> {
        if self.duplicate_sound_ids.contains(&id) {
            return None;
        }
        self.sound_index
            .get(&id)
            .and_then(|index| self.sounds.get(*index))
            .copied()
    }

    pub fn ambience(&self, id: AmbienceId) -> Option<AmbienceAsset> {
        if self.duplicate_ambience_ids.contains(&id) {
            return None;
        }
        self.ambience_index
            .get(&id)
            .and_then(|index| self.ambiences.get(*index))
            .copied()
    }

    pub fn sounds(&self) -> impl Iterator<Item = SoundAsset> + '_ {
        self.sounds.iter().copied()
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;

    #[test]
    fn rejects_duplicate_ids_and_invalid_metadata() {
        let asset = SoundAsset {
            id: SoundId(1),
            data: b"wav",
            gain: 1.0,
            speed: 1.0,
            spatial: false,
        };
        assert!(SoundBank::try_new(vec![asset, asset], vec![]).is_err());
        let unchecked = SoundBank::new(vec![asset, asset], vec![]);
        assert!(
            unchecked.sound(SoundId(1)).is_none(),
            "ambiguous duplicate IDs must not resolve to the first registration"
        );
        assert!(SoundBank::try_new(
            vec![SoundAsset {
                gain: f32::NAN,
                ..asset
            }],
            vec![]
        )
        .is_err());
    }

    #[test]
    fn reports_encoded_memory_estimate() {
        let bank = SoundBank::new(
            vec![SoundAsset {
                id: SoundId(1),
                data: b"123",
                gain: 1.0,
                speed: 1.0,
                spatial: false,
            }],
            vec![],
        );
        assert_eq!(bank.estimated_encoded_bytes(), 3);
    }
}
