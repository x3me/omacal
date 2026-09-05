//! The tray icon as today's date.
//!
//! A tray host on Linux draws icons and nothing else — there is no text
//! beside one, which is why `apply` sets a title only on macOS — so a date in
//! the tray has to *be* the icon, the way a calendar's icon has always shown
//! one (asked for 2026-09-04).
//!
//! Thirty-one images, rendered once by `icons/date/make.sh` and committed, in
//! the mark's own periwinkle so the tray still reads as this app's. Embedded
//! at compile time by `include_image!`, which decodes them during the build:
//! no font rasterizer in the binary, no text shaping at runtime, and the same
//! glyphs on every machine. A `match` rather than an array because that is
//! what the macro's literal path argument allows, and it is honest about
//! there being exactly thirty-one.

use tauri::image::Image;

/// The mark itself: what the tray wears when the date is switched off, and
/// the only icon it wore before this module existed.
pub(crate) fn mark() -> Image<'static> {
    tauri::include_image!("icons/tray.png")
}

/// The icon for a day of the month, `1..=31`. Anything else is the mark,
/// which is a state no clock produces and no reason to panic over.
pub(crate) fn icon_for(day: u32) -> Image<'static> {
    match day {
        1 => tauri::include_image!("icons/date/date-01.png"),
        2 => tauri::include_image!("icons/date/date-02.png"),
        3 => tauri::include_image!("icons/date/date-03.png"),
        4 => tauri::include_image!("icons/date/date-04.png"),
        5 => tauri::include_image!("icons/date/date-05.png"),
        6 => tauri::include_image!("icons/date/date-06.png"),
        7 => tauri::include_image!("icons/date/date-07.png"),
        8 => tauri::include_image!("icons/date/date-08.png"),
        9 => tauri::include_image!("icons/date/date-09.png"),
        10 => tauri::include_image!("icons/date/date-10.png"),
        11 => tauri::include_image!("icons/date/date-11.png"),
        12 => tauri::include_image!("icons/date/date-12.png"),
        13 => tauri::include_image!("icons/date/date-13.png"),
        14 => tauri::include_image!("icons/date/date-14.png"),
        15 => tauri::include_image!("icons/date/date-15.png"),
        16 => tauri::include_image!("icons/date/date-16.png"),
        17 => tauri::include_image!("icons/date/date-17.png"),
        18 => tauri::include_image!("icons/date/date-18.png"),
        19 => tauri::include_image!("icons/date/date-19.png"),
        20 => tauri::include_image!("icons/date/date-20.png"),
        21 => tauri::include_image!("icons/date/date-21.png"),
        22 => tauri::include_image!("icons/date/date-22.png"),
        23 => tauri::include_image!("icons/date/date-23.png"),
        24 => tauri::include_image!("icons/date/date-24.png"),
        25 => tauri::include_image!("icons/date/date-25.png"),
        26 => tauri::include_image!("icons/date/date-26.png"),
        27 => tauri::include_image!("icons/date/date-27.png"),
        28 => tauri::include_image!("icons/date/date-28.png"),
        29 => tauri::include_image!("icons/date/date-29.png"),
        30 => tauri::include_image!("icons/date/date-30.png"),
        31 => tauri::include_image!("icons/date/date-31.png"),
        _ => mark(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every day of the month has its own image, and each is a real one: the
    /// macro decodes at build time, so a missing or corrupt file is a
    /// compile error, and what is left to check here is that no two days
    /// share a picture — a copy-paste in the match above would show the
    /// wrong date in the tray for a month at a time.
    #[test]
    fn every_day_has_its_own_icon() {
        let mut seen: Vec<(u32, u32, Vec<u8>)> = Vec::new();
        for day in 1..=31 {
            let img = icon_for(day);
            assert_eq!((img.width(), img.height()), (128, 128), "day {day}");
            seen.push((img.width(), img.height(), img.rgba().to_vec()));
        }
        for i in 0..seen.len() {
            for j in (i + 1)..seen.len() {
                assert_ne!(seen[i].2, seen[j].2, "days {} and {} share an icon", i + 1, j + 1);
            }
        }
    }

    /// Out of range falls back rather than panicking, and the fallback is the
    /// mark — a tray that quietly keeps its old face beats one that dies.
    #[test]
    fn a_day_outside_the_month_is_the_mark() {
        assert_eq!(icon_for(0).rgba(), mark().rgba());
        assert_eq!(icon_for(32).rgba(), mark().rgba());
    }
}
