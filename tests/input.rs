use minifb::Key;
use raycast_blocks::input::{key_name, parse_key, Action, Input, KeyBindings, KeyState};

struct FakeKeys(Vec<Key>);

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
    let f = i.poll(&FakeKeys(vec![Key::I, Key::LeftShift, Key::E]), None);
    assert!(f.controls.forward && f.controls.sprint && f.controls.jump);
    assert!(!f.controls.back && !f.controls.left && !f.controls.right);
}

#[test]
fn arrows_and_letters_both_look() {
    let mut i = input();
    assert!(i.poll(&FakeKeys(vec![Key::Left]), None).controls.look_left);
    assert!(i.poll(&FakeKeys(vec![Key::A]), None).controls.look_left);
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
    assert!(!i.poll(&FakeKeys(vec![Key::I]), None).controls.forward);
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
