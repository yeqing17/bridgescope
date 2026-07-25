use std::{fs, path::Path};

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

pub fn configure(context: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for candidate in font_candidates() {
        if let Ok(bytes) = fs::read(candidate) {
            fonts.font_data.insert(
                "bridgescope-cjk".to_owned(),
                FontData::from_owned(bytes).into(),
            );
            if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
                family.push("bridgescope-cjk".to_owned());
            }
            break;
        }
    }
    context.set_fonts(fonts);
    context.set_visuals(egui::Visuals::dark());
}

fn font_candidates() -> impl Iterator<Item = &'static Path> {
    [
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simhei.ttf",
        "/System/Library/Fonts/PingFang.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    ]
    .into_iter()
    .map(Path::new)
}
