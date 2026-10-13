use minifb::Key;
use raycast_blocks::input::{
    key_name, parse_key, Action, Input, KeyBindings, KeyState, MouseSource,
};

struct FakeKeys(Vec<Key>);

struct FakeMouse((f32, f32));

impl MouseSource for FakeMouse {
    fn delta(&mut self) -> (f32, f32) {
        self.0
    }
}

impl KeyState for FakeKeys {
    fn is_down(&self, key: Key) -> bool {
        self.0
            .iter()
            .any(|k| std::mem::discriminant(k) == std::mem::discriminant(&key))
    }
}

fn input() -> Input {
    Input::new(KeyBindings::default(), 0.0)
}

#[test]
fn default_bindings_map_to_controls() {
    let mut i = input();
    let f = i.poll(&FakeKeys(vec![Key::W, Key::LeftShift, Key::E]), None);
    assert!(f.controls.forward && f.controls.sprint && f.controls.jump);
    assert!(!f.controls.back && !f.controls.left && !f.controls.right);
    assert_eq!(
        KeyBindings::default().keys(Action::ToggleMouseLook),
        &[Key::Tab, Key::M]
    );
}

#[test]
fn arrows_look_and_wasd_move() {
    let mut i = input();
    assert!(i.poll(&FakeKeys(vec![Key::Left]), None).controls.look_left);
    let f = i.poll(&FakeKeys(vec![Key::A]), None);
    assert!(f.controls.left && !f.controls.look_left);
}

#[test]
fn flashlight_toggles_on_press_edge_only() {
    let mut i = input();
    let held = FakeKeys(vec![Key::F]);
    let none = FakeKeys(vec![]);
    assert!(i.poll(&held, None).toggle_flashlight);
    assert!(!i.poll(&held, None).toggle_flashlight);
    assert!(!i.poll(&none, None).toggle_flashlight);
    assert!(i.poll(&held, None).toggle_flashlight);
}

#[test]
fn quit_and_restart_keys() {
    let mut i = input();
    assert!(i.poll(&FakeKeys(vec![Key::Q]), None).quit);
    assert!(i.poll(&FakeKeys(vec![Key::Escape]), None).quit);
    assert!(i.poll(&FakeKeys(vec![Key::R]), None).restart);
    assert!(!i.poll(&FakeKeys(vec![]), None).quit);
}

#[test]
fn rebinding_replaces_the_default() {
    let mut b = KeyBindings::default();
    b.set(Action::Forward, vec![Key::Up]);
    let mut i = Input::new(b, 0.0);
    assert!(!i.poll(&FakeKeys(vec![Key::W]), None).controls.forward);
    assert!(i.poll(&FakeKeys(vec![Key::Up]), None).controls.forward);
}

#[test]
fn mouse_look_uses_deltas_and_sensitivity() {
    let mut i = Input::new(KeyBindings::default(), 0.01);
    let none = FakeKeys(vec![]);
    let first = i.poll(&none, Some((100.0, 100.0)));
    assert_eq!(first.controls.mouse_yaw, 0.0); // no previous position yet
    let second = i.poll(&none, Some((110.0, 120.0)));
    assert!((second.controls.mouse_yaw - 0.1).abs() < 1e-5);
    assert!((second.controls.mouse_pitch + 0.2).abs() < 1e-5); // moving down looks down
}

#[test]
fn mouse_look_is_off_at_zero_sensitivity() {
    let mut i = input();
    let none = FakeKeys(vec![]);
    i.poll(&none, Some((0.0, 0.0)));
    let f = i.poll(&none, Some((50.0, 50.0)));
    assert_eq!(f.controls.mouse_yaw, 0.0);
    assert_eq!(f.controls.mouse_pitch, 0.0);
}

#[test]
fn key_names_round_trip() {
    assert!(matches!(parse_key("w"), Some(Key::W)));
    assert!(matches!(parse_key(" LeftShift "), Some(Key::LeftShift)));
    assert!(parse_key("NoSuchKey").is_none());
    assert_eq!(key_name(Key::Up), Some("Up"));
    assert_eq!(key_name(Key::Z), Some("Z"));
}

#[test]
fn action_names_round_trip() {
    for a in Action::ALL {
        assert_eq!(Action::from_name(a.name()), Some(a));
    }
    assert_eq!(Action::from_name("nope"), None);
}

#[test]
fn mouse_look_toggle_uses_a_press_edge() {
    let mut input = Input::with_mouse_look(KeyBindings::default(), 0.0025, false, false);
    let mut mouse = FakeMouse((0.0, 0.0));
    let toggle = FakeKeys(vec![Key::Tab]);
    let none = FakeKeys(vec![]);

    assert!(
        input
            .poll_with_mouse_source(&toggle, &mut mouse, true)
            .toggle_mouse_look
    );
    assert!(input.mouse_look_enabled());
    assert!(
        !input
            .poll_with_mouse_source(&toggle, &mut mouse, true)
            .toggle_mouse_look
    );
    input.poll_with_mouse_source(&none, &mut mouse, true);
    assert!(
        input
            .poll_with_mouse_source(&toggle, &mut mouse, true)
            .toggle_mouse_look
    );
    assert!(!input.mouse_look_enabled());
}

#[test]
fn mouse_source_scales_deltas_and_inverts_pitch() {
    let mut normal = Input::with_mouse_look(KeyBindings::default(), 0.01, true, false);
    let none = FakeKeys(vec![]);
    let mut mouse = FakeMouse((0.0, 0.0));
    normal.poll_with_mouse_source(&none, &mut mouse, true); // activation baseline
    mouse.0 = (10.0, 20.0);
    let frame = normal.poll_with_mouse_source(&none, &mut mouse, true);
    assert!((frame.controls.mouse_yaw - 0.1).abs() < 1e-6);
    assert!((frame.controls.mouse_pitch + 0.2).abs() < 1e-6);

    let mut inverted = Input::with_mouse_look(KeyBindings::default(), 0.01, true, true);
    inverted.poll_with_mouse_source(&none, &mut mouse, true);
    let frame = inverted.poll_with_mouse_source(&none, &mut mouse, true);
    assert!(frame.controls.mouse_pitch > 0.0);
}

#[test]
fn mouse_look_ignores_activation_frame_and_clamps_large_deltas() {
    let mut input = Input::with_mouse_look(KeyBindings::default(), 0.0025, false, false);
    let mut mouse = FakeMouse((0.0, 0.0));
    let none = FakeKeys(vec![]);
    let toggle = FakeKeys(vec![Key::M]);

    let enabled = input.poll_with_mouse_source(&toggle, &mut mouse, true);
    assert!(enabled.toggle_mouse_look);
    assert_eq!(enabled.controls.mouse_yaw, 0.0);
    mouse.0 = (10_000.0, -10_000.0);
    let clamped = input.poll_with_mouse_source(&none, &mut mouse, true);
    assert!((clamped.controls.mouse_yaw - 0.25).abs() < 1e-6);
    assert!((clamped.controls.mouse_pitch - 0.25).abs() < 1e-6);
}

#[test]
fn disabled_mouse_look_ignores_mouse_motion() {
    let mut input = Input::with_mouse_look(KeyBindings::default(), 0.0025, false, false);
    let mut mouse = FakeMouse((500.0, -200.0));
    let frame = input.poll_with_mouse_source(&FakeKeys(vec![]), &mut mouse, true);
    assert_eq!(frame.controls.mouse_yaw, 0.0);
    assert_eq!(frame.controls.mouse_pitch, 0.0);
}
