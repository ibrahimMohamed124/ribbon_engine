use super::*;
use ribbon_app::{App, Stage, World};
use ribbon_events::WorldEventsExt;
use ribbon_plugins::Plugin;

/// Installs an asset server and polls CPU load completions and hot reload once per frame.
#[derive(Default)]
pub struct AssetPlugin;

impl Plugin for AssetPlugin {
    fn name(&self) -> &'static str {
        "AssetPlugin"
    }

    fn build(&self, app: &mut App) {
        if app.world.try_resource::<AssetServer>().is_none() {
            app.insert_resource(AssetServer::default());
        }
        app.world
            .resource_mut::<AssetServer>()
            .set_async_hot_reload(true);
        app.add_event::<AssetEvent>();
        app.add_system(Stage::First, update_asset_server);
    }
}

fn update_asset_server(world: &mut World) {
    world.resource_scope::<AssetServer, _>(|world, server| {
        for event in server.poll_pending_loads() {
            world.send_event(event);
        }
        for event in server.poll_hot_reload() {
            world.send_event(event);
        }
    });
}
