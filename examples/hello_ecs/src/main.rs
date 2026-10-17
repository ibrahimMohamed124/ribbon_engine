use engine_app::{App, Time};
use engine_ecs::World;
use engine_schedule::Stage;
use std::time::Duration;

#[derive(Clone, Copy)]
struct Counter(u64);

fn main() {
    let mut app = App::new();
    app.world.spawn((Counter(0),));
    app.add_system(Stage::Update, |world: &mut World| {
        for (_, counter) in world.query_mut::<&mut Counter>() {
            counter.0 = counter.0.wrapping_add(1);
        }
    });
    let mut app = app.build().expect("valid example schedule");
    for _ in 0..60 {
        app.world.resource_mut::<Time>().delta = Duration::from_secs_f64(1.0 / 60.0);
        app.update();
    }
    let checksum = app
        .world
        .query::<&Counter>()
        .iter()
        .fold(0_u64, |sum, (_, count)| {
            sum.wrapping_mul(16_777_619).wrapping_add(count.0)
        });
    println!("hello_ecs checksum: {checksum:016x}");
}
