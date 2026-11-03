//! Plugin registration.
use crate::{particle_update_system, ParticleBudget, ParticleDiagnostics};
use ribbon_app::{App, Stage};
use ribbon_plugins::Plugin;
use ribbon_render_contracts::ParticleRenderData;
pub struct ParticlePlugin;
impl Plugin for ParticlePlugin {
    fn name(&self) -> &'static str {
        "ParticlePlugin"
    }
    fn dependencies(&self) -> &'static [&'static str] {
        &["ScenePlugin"]
    }
    fn build(&self, app: &mut App) {
        if app.world.try_resource::<ParticleRenderData>().is_none() {
            app.insert_resource(ParticleRenderData::default());
        }
        if app.world.try_resource::<ParticleBudget>().is_none() {
            app.insert_resource(ParticleBudget::default());
        }
        if app.world.try_resource::<ParticleDiagnostics>().is_none() {
            app.insert_resource(ParticleDiagnostics::default());
        }
        app.add_system(Stage::Update, particle_update_system);
    }
}
