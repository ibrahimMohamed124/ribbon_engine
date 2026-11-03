//! Save plugin and event handlers.
use crate::{
    section::SaveRegistry,
    store::{advance_play_time, load_from_slot, save_to_slot},
    LoadFinished, LoadRequest, SaveConfig, SaveFinished, SaveRequest,
};
use ribbon_app::{App, EventCursor, LogLevel, Stage, World, WorldEventsExt};
use ribbon_plugins::Plugin;
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayTime(pub f64);
pub struct SavePlugin {
    pub config: SaveConfig,
}
struct SaveCursor(EventCursor<SaveRequest>);
struct LoadCursor(EventCursor<LoadRequest>);
impl Plugin for SavePlugin {
    fn name(&self) -> &'static str {
        "SavePlugin"
    }
    fn build(&self, app: &mut App) {
        app.insert_resource(self.config.clone())
            .insert_resource(SaveRegistry::default())
            .insert_resource(PlayTime::default());
        app.insert_resource(SaveCursor(EventCursor::default()))
            .insert_resource(LoadCursor(EventCursor::default()));
        app.add_event::<SaveRequest>()
            .add_event::<LoadRequest>()
            .add_event::<SaveFinished>()
            .add_event::<LoadFinished>();
        app.add_system(Stage::Update, advance_play_time)
            .add_system(Stage::Last, handle_save_requests)
            .add_system(Stage::PreUpdate, handle_load_requests);
    }
}
fn handle_save_requests(world: &mut World) {
    world.resource_scope::<SaveCursor, _>(|world, cursor| {
        let reqs = cursor
            .0
            .read(world.events::<SaveRequest>())
            .cloned()
            .collect::<Vec<_>>();
        for req in reqs {
            let result = save_to_slot(world, &req.slot, &req.label);
            world.send_event(SaveFinished {
                slot: req.slot,
                result,
            });
        }
    });
}
fn handle_load_requests(world: &mut World) {
    world.resource_scope::<LoadCursor, _>(|world, cursor| {
        let reqs = cursor
            .0
            .read(world.events::<LoadRequest>())
            .cloned()
            .collect::<Vec<_>>();
        for req in reqs {
            let result = load_from_slot(world, &req.slot);
            if let Err(ref error) = result {
                world.resource_mut::<ribbon_app::LogBuffer>().push(
                    LogLevel::Warn,
                    format!("load `{}` failed: {error}", req.slot),
                );
            }
            world.send_event(LoadFinished {
                slot: req.slot,
                result,
            });
        }
    });
}
