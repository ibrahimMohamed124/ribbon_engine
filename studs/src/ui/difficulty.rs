use crate::game::Difficulty;

pub const TITLE: &str = "CHOOSE DIFFICULTY";
pub const ITEMS: &[&str] = &["EASY", "NORMAL", "HARD", "BACK"];
pub const OPTIONS: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard];

pub const fn hint(selected: usize) -> &'static str {
    match selected {
        0 => "MORE TIME   SLOWER CHASE",
        1 => "ORIGINAL ENEMY BALANCE",
        2 => "LESS TIME   FASTER CHASE",
        _ => "ESC BACK",
    }
}
