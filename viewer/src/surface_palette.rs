//! 森林・水域の濃度を保ったまま、表示色だけを切り替える。
use eframe::egui::{self, Color32};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Palette {
    #[default]
    Source,
    Natural,
    Soft,
    Contrast,
    Muted,
}
impl Palette {
    pub const ALL: [Self; 5] = [
        Self::Source,
        Self::Natural,
        Self::Soft,
        Self::Contrast,
        Self::Muted,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Source => "データの既定色",
            Self::Natural => "深緑",
            Self::Soft => "淡色",
            Self::Contrast => "高コントラスト",
            Self::Muted => "落ち着いた色",
        }
    }
    // その他・森林・水域の順。Sourceの色見本はESAの参考色。
    pub fn colors(self) -> [[u8; 4]; 3] {
        match self {
            Self::Source => [
                [224, 216, 196, 255],
                [111, 148, 88, 255],
                [114, 172, 204, 255],
            ],
            Self::Natural => [
                [239, 231, 210, 255],
                [79, 120, 73, 255],
                [96, 163, 199, 255],
            ],
            Self::Soft => [
                [241, 237, 221, 255],
                [157, 183, 134, 255],
                [155, 202, 222, 255],
            ],
            Self::Contrast => [
                [241, 225, 184, 255],
                [45, 108, 58, 255],
                [55, 140, 193, 255],
            ],
            Self::Muted => [
                [226, 226, 215, 255],
                [110, 134, 114, 255],
                [112, 154, 175, 255],
            ],
        }
    }
    pub fn mask(self, id: &str) -> Option<[u8; 4]> {
        if self == Self::Source {
            return None;
        }
        match id {
            "forest" => Some(self.colors()[1]),
            "water" => Some(self.colors()[2]),
            _ => None,
        }
    }
    pub fn background(self, id: &str) -> Option<[u8; 4]> {
        (self != Self::Source && id == "forest").then(|| self.colors()[0])
    }
    pub fn menu(&mut self, ui: &mut egui::Ui) -> bool {
        let before = *self;
        ui.small("色見本：その他・森林・水域");
        for palette in Self::ALL {
            ui.horizontal(|ui| {
                if ui
                    .selectable_value(self, palette, palette.label())
                    .clicked()
                {
                    ui.close_menu();
                }
                for color in palette.colors() {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(14., 14.), egui::Sense::hover());
                    ui.painter().rect_filled(
                        rect,
                        3.,
                        Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]),
                    );
                }
            });
        }
        *self != before
    }
}
