//! Directional and sequential focus state.
use crate::Id;
#[derive(Clone, Debug, Default)]
pub struct FocusState {
    pub focused: Option<Id>,
    pub(crate) previous: Vec<(Id, crate::Rect)>,
    pub(crate) current: Vec<(Id, crate::Rect)>,
    _repeat_timer: f32,
}
