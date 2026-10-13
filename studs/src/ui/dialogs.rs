pub const MAIN_MENU_TITLE: &str = "LEAVE THIS RUN?";
pub const MAIN_MENU_BODY: &str = "UNSAVED PROGRESS WILL BE LOST";

pub fn result_copy(won: bool) -> (&'static str, &'static str) {
    if won {
        ("YOU ESCAPED", "ALL SIX PAGES FOUND")
    } else {
        ("GAME OVER", "THE CREATURE CAUGHT YOU")
    }
}
