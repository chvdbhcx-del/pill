//! Medeo Sans (SIL OFL, `src/fonts/OFL.txt`), the face of every native Pill surface, embedded once. Install it with
//! `host::set_fonts(&fonts::FONTS)` before the first window is created; headless tests build their harness over the same set.
use glasskit::fonts::FontSet;

fn family() -> &'static str {
    "Medeo Sans"
}

fn gdi() -> &'static [u8] {
    include_bytes!("../../../assets/fonts/MedeoSans-Regular.ttf")
}

pub static FONTS: FontSet = FontSet::new(
    &[
        include_bytes!("../../../assets/fonts/MedeoSans-Regular.ttf"),
        include_bytes!("../../../assets/fonts/MedeoSans-Medium.ttf"),
        include_bytes!("../../../assets/fonts/MedeoSans-SemiBold.ttf"),
        include_bytes!("../../../assets/fonts/MedeoSans-Bold.ttf"),
    ],
    family,
    gdi,
);
