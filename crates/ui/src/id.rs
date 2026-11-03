//! Stable immediate-mode widget IDs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Id(pub u64);
pub fn hash_id(parent: u64, kind: &str, label: &str, salt: u64) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in parent
        .to_le_bytes()
        .into_iter()
        .chain(kind.bytes())
        .chain(label.bytes())
        .chain(salt.to_le_bytes())
    {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
