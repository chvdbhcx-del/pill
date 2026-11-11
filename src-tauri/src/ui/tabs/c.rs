//! Native views of panel set C: tabs added after the A / B ports. Today: the Phone tab (`phone`), see `phone.rs`.
#![allow(dead_code)] // wired by the island view

mod phone;
mod phone_card;
mod phone_dash;
mod phone_draw;
mod phone_pair;
mod phone_img;

use crate::ui::view::View;

/// A fresh view for a tab of set C (`phone`); `None` for ids of the other sets.
pub fn create(id: &str) -> Option<Box<dyn View>> {
    Some(match id {
        "phone" => Box::new(phone::PhoneView::new()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_knows_the_phone_tab_only() {
        let v = create("phone").expect("phone");
        assert_eq!(v.size_hint(), Some(phone::SIZE));
        for id in ["home", "agents", "note", "", "settings"] {
            assert!(create(id).is_none(), "{id}");
        }
        assert!(crate::ui::tabs::create("phone").is_some());
    }
}
