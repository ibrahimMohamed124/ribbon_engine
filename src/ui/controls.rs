use crate::input::Action;

pub const HEADER: &str = "REBIND CONTROLS";
pub const MOUSE_NOTE: &str = "MOUSE LOOK: MOVE THE MOUSE";
pub const HINT: &str = "CLICK / ENTER REBIND   ESC BACK";

pub fn label(action: Action) -> &'static str {
    match action {
        Action::Forward => "MOVE FORWARD",
        Action::Back => "MOVE BACK",
        Action::Left => "MOVE LEFT",
        Action::Right => "MOVE RIGHT",
        Action::LookUp => "LOOK UP",
        Action::LookDown => "LOOK DOWN",
        Action::LookLeft => "LOOK LEFT",
        Action::LookRight => "LOOK RIGHT",
        Action::Jump => "JUMP",
        Action::Sprint => "SPRINT",
        Action::Flashlight => "FLASHLIGHT",
        Action::Restart => "RESTART RUN",
        Action::Quit => "QUIT",
        Action::ToggleMouseLook => "TOGGLE MOUSE LOOK",
    }
}

pub const ACTIONS: [Action; 14] = [
    Action::Forward,
    Action::Back,
    Action::Left,
    Action::Right,
    Action::LookUp,
    Action::LookDown,
    Action::LookLeft,
    Action::LookRight,
    Action::Jump,
    Action::Sprint,
    Action::Flashlight,
    Action::Restart,
    Action::ToggleMouseLook,
    Action::Quit,
];
