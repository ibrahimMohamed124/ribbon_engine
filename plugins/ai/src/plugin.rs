//! AI plugin setup.
use crate::{
    agent::{nav_agent_system, NavSettings, PathCursor},
    behavior_tree::{bt_system, BtRegistry},
    perception::{perception_system, AiSettings, SightCursor},
};
use ribbon_app::{App, Stage};
use ribbon_plugins::Plugin;
pub struct AiPlugin;
impl Plugin for AiPlugin {
    fn name(&self) -> &'static str {
        "AiPlugin"
    }
    fn dependencies(&self) -> &'static [&'static str] {
        &["ScenePlugin", "PhysicsPlugin"]
    }
    fn build(&self, app: &mut App) {
        app.insert_resource(BtRegistry::default())
            .insert_resource(NavSettings::default())
            .insert_resource(AiSettings::default())
            .insert_resource(SightCursor::default());
        app.insert_resource(PathCursor::default());
        app.add_system(Stage::Update, perception_system)
            .add_system(Stage::Update, bt_system)
            .add_system(Stage::Update, nav_agent_system);
    }
}
