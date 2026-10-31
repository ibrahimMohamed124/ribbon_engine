use super::*;
use crate::GpuResourceWarmup;
use ribbon_app::App;
use ribbon_plugins::Plugin;
use ribbon_reflect::TypeRegistry;
use ribbon_scene::PrefabCodecRegistry;

pub struct Render3dPlugin;

impl Plugin for Render3dPlugin {
    fn name(&self) -> &'static str {
        "Render3dPlugin"
    }

    fn dependencies(&self) -> &'static [&'static str] {
        &["GpuWindowPlugin", "ScenePlugin"]
    }

    fn build(&self, app: &mut App) {
        if app.world.try_resource::<SunLight>().is_none() {
            app.insert_resource(SunLight::default());
        }
        if app.world.try_resource::<FogSettings>().is_none() {
            app.insert_resource(FogSettings::default());
        }
        if app.world.try_resource::<ModelWarmup>().is_none() {
            app.insert_resource(ModelWarmup::default());
        }
        if app.world.try_resource::<GpuResourceWarmup>().is_none() {
            app.insert_resource(GpuResourceWarmup::default());
        }
        if app.world.try_resource::<GpuUploadBudget>().is_none() {
            app.insert_resource(GpuUploadBudget::default());
        }
        if app
            .world
            .try_resource::<ModelGpuResidencyBudget>()
            .is_none()
        {
            app.insert_resource(ModelGpuResidencyBudget::default());
        }
        // Compose codecs with physics and any other scene-aware plugins; do not replace an
        // existing registry as plugin build order is not otherwise constrained between peers.
        let mut prefab_codecs = app
            .world
            .remove_resource::<PrefabCodecRegistry>()
            .unwrap_or_default();
        let registration = crate::prefab::register_prefab_components(
            app.world.resource_mut::<TypeRegistry>(),
            &mut prefab_codecs,
        );
        match registration {
            Ok(()) => {
                app.insert_resource(prefab_codecs);
            }
            Err(error) => {
                app.insert_resource(prefab_codecs);
                app.world.resource_mut::<ribbon_app::LogBuffer>().push(
                    ribbon_app::LogLevel::Error,
                    format!("render3d prefab reflection registration failed: {error}"),
                );
            }
        }
        // ScenePlugin has already registered PostUpdate transform propagation because it is
        // an explicit dependency. Install extraction after it so all snapshots are world-space.
        extract::install_extraction(app);
        app.world
            .resource_mut::<GpuWindow>()
            .add_pass(Box::new(MeshPass::default()));
    }
}
