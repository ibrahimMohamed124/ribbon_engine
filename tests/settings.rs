use minifb::Key;
use raycast_blocks::input::Action;
use raycast_blocks::settings::Settings;

#[test]
fn empty_text_gives_defaults() {
    let s = Settings::parse("");
    let d = Settings::default();
    assert_eq!(s.render_width, d.render_width);
    assert_eq!(s.window_scale, d.window_scale);
    assert!((s.master_volume - d.master_volume).abs() < 1e-6);
}

#[test]
fn values_are_clamped_and_garbage_ignored() {
    let s = Settings::parse(
        "master_volume = 5\n\
         hfov_degrees = 1\n\
         render_width = banana\n\
         window_scale = 3\n\
         mouse_sensitivity = nan\n\
         # a comment\n\
         no equals sign here\n\
         unknown_key = 1\n",
    );
    let d = Settings::default();
    assert_eq!(s.master_volume, 1.0);
    assert_eq!(s.hfov_degrees, 40.0);
    assert_eq!(s.render_width, d.render_width);
    assert_eq!(s.window_scale, d.window_scale); // 3 is not a valid scale
    assert_eq!(s.mouse_sensitivity, d.mouse_sensitivity);
}

#[test]
fn inline_comments_are_stripped() {
    let s = Settings::parse("master_volume = 0.25 # quiet\n");
    assert!((s.master_volume - 0.25).abs() < 1e-6);
}

#[test]
fn round_trip_through_text() {
    let mut s = Settings::default();
    s.master_volume = 0.25;
    s.hfov_degrees = 100.0;
    s.render_width = 400;
    s.render_height = 225;
    s.window_scale = 2;
    s.flashlight_on_start = false;
    s.mouse_sensitivity = 0.004;
    s.bindings.set(Action::Forward, vec![Key::Up, Key::W]);

    let back = Settings::parse(&s.to_text());
    assert!((back.master_volume - 0.25).abs() < 1e-6);
    assert!((back.hfov_degrees - 100.0).abs() < 1e-4);
    assert_eq!(back.render_width, 400);
    assert_eq!(back.render_height, 225);
    assert_eq!(back.window_scale, 2);
    assert!(!back.flashlight_on_start);
    assert!((back.mouse_sensitivity - 0.004).abs() < 1e-6);

    let keys = back.bindings.keys(Action::Forward);
    assert_eq!(keys.len(), 2);
    assert!(matches!(keys[0], Key::Up));
    assert!(matches!(keys[1], Key::W));
}

#[test]
fn bad_binding_keeps_the_default() {
    let s = Settings::parse("key.forward = Banana\n");
    let keys = s.bindings.keys(Action::Forward);
    assert_eq!(keys.len(), 1);
    assert!(matches!(keys[0], Key::I));
}

#[test]
fn save_and_load_file() {
    let dir = std::env::temp_dir().join(format!("raycast_blocks_test_{}", std::process::id()));
    let path = dir.join("nested").join("settings.cfg");

    let mut s = Settings::default();
    s.window_scale = 8;
    s.save(&path).expect("save");
    let loaded = Settings::load(&path).expect("load");
    assert_eq!(loaded.window_scale, 8);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_file_falls_back_to_defaults() {
    let path = std::env::temp_dir().join("raycast_blocks_definitely_missing.cfg");
    let s = Settings::load_or_default(&path);
    assert_eq!(s.window_scale, Settings::default().window_scale);
}
