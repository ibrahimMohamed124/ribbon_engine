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
    assert!((s.effects_volume - d.effects_volume).abs() < 1e-6);
    assert!((s.ambient_volume - d.ambient_volume).abs() < 1e-6);
    assert!(!s.mute_all);
    assert!(s.mouse_look_on_start);
    assert!((s.mouse_sensitivity - 0.0025).abs() < 1e-6);
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
    let mut s = Settings {
        master_volume: 0.25,
        hfov_degrees: 100.0,
        render_width: 400,
        render_height: 225,
        window_scale: 2,
        flashlight_on_start: false,
        mouse_look_on_start: true,
        invert_y: true,
        mouse_sensitivity: 0.004,
        ..Settings::default()
    };
    s.bindings.set(Action::Forward, vec![Key::Up, Key::W]);
    s.bindings.set(Action::ToggleMouseLook, vec![Key::M]);

    let back = Settings::parse(&s.to_text());
    assert!((back.master_volume - 0.25).abs() < 1e-6);
    assert!((back.hfov_degrees - 100.0).abs() < 1e-4);
    assert_eq!(back.render_width, 400);
    assert_eq!(back.render_height, 225);
    assert_eq!(back.window_scale, 2);
    assert!(!back.flashlight_on_start);
    assert!((back.mouse_sensitivity - 0.004).abs() < 1e-6);
    assert!(back.mouse_look_on_start);
    assert!(back.invert_y);
    assert_eq!(back.bindings.keys(Action::ToggleMouseLook), &[Key::M]);

    let keys = back.bindings.keys(Action::Forward);
    assert_eq!(keys.len(), 2);
    assert!(matches!(keys[0], Key::Up));
    assert!(matches!(keys[1], Key::W));
}

#[test]
fn mouse_settings_parse_and_serialize() {
    let settings = Settings::parse(
        "mouse_look_on_start = true\ninvert_y = true\nmouse_sensitivity = 0.003\nkey.toggle_mouse_look = M\n",
    );
    assert!(settings.mouse_look_on_start);
    assert!(settings.invert_y);
    assert!((settings.mouse_sensitivity - 0.003).abs() < 1e-6);
    assert_eq!(settings.bindings.keys(Action::ToggleMouseLook), &[Key::M]);

    let parsed = Settings::parse(&settings.to_text());
    assert!(parsed.mouse_look_on_start);
    assert!(parsed.invert_y);
    assert_eq!(parsed.bindings.keys(Action::ToggleMouseLook), &[Key::M]);
}

#[test]
fn old_zero_mouse_sensitivity_migrates_to_a_usable_default() {
    let settings = Settings::parse("settings_version = 3\nmouse_sensitivity = 0\n");
    assert!((settings.mouse_sensitivity - Settings::default().mouse_sensitivity).abs() < 1e-6);

    let current = Settings::parse("settings_version = 4\nmouse_sensitivity = 0\n");
    assert_eq!(current.mouse_sensitivity, 0.0);
}

#[test]
fn bad_binding_keeps_the_default() {
    let s = Settings::parse("key.forward = Banana\n");
    let keys = s.bindings.keys(Action::Forward);
    assert_eq!(keys.len(), 1);
    assert!(matches!(keys[0], Key::W));
}

#[test]
fn save_and_load_file() {
    let dir = std::env::temp_dir().join(format!("raycast_blocks_test_{}", std::process::id()));
    let path = dir.join("nested").join("settings.cfg");

    let s = Settings {
        window_scale: 4,
        ..Settings::default()
    };
    s.save(&path).expect("save");
    let loaded = Settings::load(&path).expect("load");
    assert_eq!(loaded.window_scale, 4);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_file_falls_back_to_defaults() {
    let path = std::env::temp_dir().join("raycast_blocks_definitely_missing.cfg");
    let s = Settings::load_or_default(&path);
    assert_eq!(s.window_scale, Settings::default().window_scale);
}

#[test]
fn audio_bus_settings_round_trip_and_clamp() {
    let settings = Settings::parse(
        "master_volume = 0.5\neffects_volume = 0.65\nambient_volume = 0.2\nmute_all = true\n",
    );
    assert!((settings.effects_volume - 0.65).abs() < 1e-6);
    assert!((settings.ambient_volume - 0.2).abs() < 1e-6);
    assert!(settings.mute_all);

    let restored = Settings::parse(&settings.to_text());
    assert!((restored.effects_volume - 0.65).abs() < 1e-6);
    assert!((restored.ambient_volume - 0.2).abs() < 1e-6);
    assert!(restored.mute_all);
}

#[test]
fn version_two_settings_keep_custom_movement_but_reset_escape_quit() {
    let settings = Settings::parse("settings_version = 2\nkey.forward = I\nkey.quit = Escape,Q\n");
    assert!(matches!(settings.bindings.keys(Action::Forward)[0], Key::I));
    assert_eq!(settings.bindings.keys(Action::Quit), &[Key::Q]);
}

#[test]
fn legacy_file_keeps_new_movement_defaults() {
    // A file without `settings_version` holds the old IJKL/WASD-look bindings.
    let s = Settings::parse("key.forward = I\nkey.look_left = A,Left\nkey.flashlight = G\n");
    assert!(matches!(s.bindings.keys(Action::Forward)[0], Key::W));
    assert_eq!(s.bindings.keys(Action::LookLeft).len(), 1);
    // non-movement bindings are still honoured
    assert!(matches!(s.bindings.keys(Action::Flashlight)[0], Key::G));
    // current-version files are honoured in full
    let s = Settings::parse("settings_version = 2\nkey.forward = I\n");
    assert!(matches!(s.bindings.keys(Action::Forward)[0], Key::I));
}
