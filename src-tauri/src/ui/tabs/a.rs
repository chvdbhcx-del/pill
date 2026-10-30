//! Native panel tab views of this set; one agent owns this file. Return None for ids that belong to the other set.
use crate::ui::view::View;

#[allow(dead_code)] // used once the island view lands
pub fn create(_id: &str) -> Option<Box<dyn View>> {
    None
}
