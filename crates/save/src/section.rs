//! Typed section registry and migration hooks.
use crate::SaveError;
use ribbon_app::{App, LogLevel};
use ribbon_ecs::World;
pub trait SaveSection:
    serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static
{
    const KEY: &'static str;
    const VERSION: u32;
    fn capture(world: &World) -> Self;
    fn apply(self, world: &mut World) -> Result<(), SaveError>;
    fn migrate(from_version: u32, _raw_ron: &str) -> Result<Self, SaveError> {
        Err(SaveError::UnsupportedSectionVersion {
            key: Self::KEY.into(),
            found: from_version,
            expected: Self::VERSION,
        })
    }
}
pub(crate) type ApplyFn = Box<dyn FnOnce(&mut World) -> Result<(), SaveError> + Send>;
pub(crate) trait SectionOps: Send + Sync {
    fn key(&self) -> &'static str;
    fn version(&self) -> u32;
    fn capture_ron(&self, world: &World) -> Result<String, SaveError>;
    fn prepare_apply(&self, raw: &str, version: u32) -> Result<ApplyFn, SaveError>;
}
pub(crate) struct TypedSection<T>(std::marker::PhantomData<T>);
impl<T> Default for TypedSection<T> {
    fn default() -> Self {
        Self(std::marker::PhantomData)
    }
}
impl<T: SaveSection> SectionOps for TypedSection<T> {
    fn key(&self) -> &'static str {
        T::KEY
    }
    fn version(&self) -> u32 {
        T::VERSION
    }
    fn capture_ron(&self, world: &World) -> Result<String, SaveError> {
        ron::to_string(&T::capture(world)).map_err(|e| SaveError::Ron(e.to_string()))
    }
    fn prepare_apply(&self, raw: &str, version: u32) -> Result<ApplyFn, SaveError> {
        let value = if version == T::VERSION {
            ron::from_str(raw).map_err(|e| SaveError::Ron(e.to_string()))?
        } else {
            T::migrate(version, raw)?
        };
        Ok(Box::new(move |world| value.apply(world)))
    }
}
#[derive(Default)]
pub(crate) struct SaveRegistry(pub Vec<Box<dyn SectionOps>>);
pub trait AppSaveExt {
    fn register_save_section<T: SaveSection>(&mut self) -> &mut Self;
}
impl AppSaveExt for App {
    fn register_save_section<T: SaveSection>(&mut self) -> &mut Self {
        if T::KEY.is_empty()
            || !T::KEY
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        {
            self.world.resource_mut::<ribbon_app::LogBuffer>().push(
                LogLevel::Error,
                format!("invalid save section key `{}`", T::KEY),
            );
            return self;
        }
        let duplicate = self
            .world
            .resource::<SaveRegistry>()
            .0
            .iter()
            .any(|s| s.key() == T::KEY);
        if duplicate {
            self.world.resource_mut::<ribbon_app::LogBuffer>().push(
                LogLevel::Error,
                format!("duplicate save section `{}` ignored", T::KEY),
            );
            return self;
        }
        self.world
            .resource_mut::<SaveRegistry>()
            .0
            .push(Box::new(TypedSection::<T>::default()));
        self
    }
}
