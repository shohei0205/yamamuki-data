use crate::{
    boundaries::Location,
    points::{PROVENANCE_FIELDS, Point, Points},
};
use eframe::egui;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

#[derive(Default, PartialEq, Eq, Clone, Copy)]
enum Sort {
    #[default]
    Source,
    Name,
    Elevation,
    Location,
    Coordinates,
    Reading,
    Aliases,
    Tags,
    Svg,
    Id,
}

#[derive(Default)]
pub struct PointList {
    query: String,
    tags: BTreeSet<String>,
    tag_counts: Vec<(String, usize)>,
    provenance_filters: [BTreeSet<String>; 3],
    provenance_counts: [BTreeMap<String, usize>; 3],
    sort: Sort,
    descending: bool,
    rows: Vec<usize>,
    prefix: Vec<usize>,
    reveal: Option<usize>,
    locations: Option<Vec<Location>>,
    reset_scroll: bool,
    pub filter_revision: u64,
}

impl PointList {
    pub fn reset(&mut self, points: &Points) {
        *self = Self::default();
        let mut counts = BTreeMap::new();
        for index in 0..points.len() {
            let point = points.point(index);
            for ((key, _), counts) in PROVENANCE_FIELDS
                .into_iter()
                .zip(&mut self.provenance_counts)
            {
                for code in point.provenance_codes(key) {
                    *counts.entry(code.into()).or_insert(0) += 1;
                }
            }
            for tag in point.tags.as_deref().unwrap_or_default() {
                *counts.entry(tag.clone()).or_insert(0) += 1;
            }
        }
        self.tag_counts = counts.into_iter().collect();
        self.rebuild(points);
    }

    fn rebuild(&mut self, points: &Points) {
        self.filter_revision += 1;
        let query = self.query.to_lowercase();
        let terms: Vec<_> = query.split_whitespace().collect();
        self.rows.clear();
        self.prefix.clear();
        self.prefix.push(0);
        for index in 0..points.len() {
            let point = points.point(index);
            let tags = point.tags.as_deref().unwrap_or_default();
            if (self.tags.is_empty() || tags.iter().any(|tag| self.tags.contains(tag)))
                && PROVENANCE_FIELDS
                    .into_iter()
                    .zip(&self.provenance_filters)
                    .all(|((key, _), selected)| {
                        selected.is_empty()
                            || point
                                .provenance_codes(key)
                                .any(|code| selected.contains(code))
                    })
                && matches_terms(point, &terms)
            {
                self.rows.push(index);
            }
            self.prefix.push(self.rows.len());
        }
        if self.rows.len() == points.len() {
            self.prefix.clear();
        }
        self.sort_rows(points);
        self.reset_scroll = true;
    }

    fn sort_rows(&mut self, points: &Points) {
        let locations = self.locations.as_deref();
        let location = |i| {
            locations
                .and_then(|locations| locations.get(i))
                .filter(|location: &&Location| !location.label.is_empty())
        };
        self.rows.sort_unstable_by(|&i, &j| {
            let (a, b) = (points.point(i), points.point(j));
            let text = |a: String, b: String| {
                compare_optional(
                    (!a.is_empty()).then_some(a),
                    (!b.is_empty()).then_some(b),
                    self.descending,
                )
            };
            let order = match self.sort {
                Sort::Source => return points.source_index(i).cmp(&points.source_index(j)),
                Sort::Location => {
                    return compare_optional(location(i), location(j), self.descending)
                        .then_with(|| i.cmp(&j));
                }
                Sort::Coordinates => a
                    .latitude
                    .total_cmp(&b.latitude)
                    .then_with(|| a.longitude.total_cmp(&b.longitude)),
                Sort::Reading => return text(reading(a), reading(b)).then_with(|| i.cmp(&j)),
                Sort::Aliases => return text(aliases(a), aliases(b)).then_with(|| i.cmp(&j)),
                Sort::Tags => return text(tags(a), tags(b)).then_with(|| i.cmp(&j)),
                Sort::Svg => a.graphic.is_some().cmp(&b.graphic.is_some()),
                Sort::Id => a.id.cmp(&b.id),
                Sort::Name => a.name.cmp(&b.name),
                Sort::Elevation => match (a.elevation_m, b.elevation_m) {
                    (Some(a), Some(b)) => a.total_cmp(&b),
                    (Some(_), None) => return Ordering::Less,
                    (None, Some(_)) => return Ordering::Greater,
                    _ => Ordering::Equal,
                },
            };
            let order = if self.descending {
                order.reverse()
            } else {
                order
            };
            order.then_with(|| a.id.cmp(&b.id)).then_with(|| i.cmp(&j))
        });
    }

    pub fn set_locations(&mut self, points: &Points, locations: Vec<Location>) {
        self.locations = Some(locations);
        if self.sort == Sort::Location {
            self.sort_rows(points);
        }
    }

    fn location(&self, index: usize) -> Option<&Location> {
        self.locations
            .as_ref()
            .and_then(|locations| locations.get(index))
            .filter(|location| !location.label.is_empty())
    }

    pub fn location_label(&self, index: usize) -> &str {
        self.location(index).map_or(
            if self.locations.is_some() {
                "不明"
            } else {
                "未判定"
            },
            |location| &location.label,
        )
    }

    pub fn matches(&self, index: usize) -> bool {
        self.prefix.is_empty() || self.prefix[index + 1] > self.prefix[index]
    }

    pub fn visible_count(&self) -> usize {
        self.rows.len()
    }

    pub fn prefix(&self) -> &[usize] {
        &self.prefix
    }

    pub fn clear_filter_to_reveal(&mut self, points: &Points, index: usize) -> bool {
        if self.matches(index) {
            return false;
        }
        self.query.clear();
        self.tags.clear();
        self.provenance_filters.iter_mut().for_each(BTreeSet::clear);
        self.rebuild(points);
        true
    }

    pub fn reveal(&mut self, index: usize) {
        self.reveal = Some(index);
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        points: &Points,
        selected: Option<usize>,
    ) -> (Option<usize>, bool) {
        ui.heading("地点一覧");
        let edit = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .hint_text("地点名・読み・別名・IDで検索")
                .desired_width(f32::INFINITY),
        );
        let mut changed = edit.changed();
        ui.horizontal_wrapped(|ui| {
            ui.menu_button("タグ", |ui| {
                if ui.button("すべて").clicked() && !self.tags.is_empty() {
                    self.tags.clear();
                    changed = true;
                }
                egui::ScrollArea::vertical()
                    .max_height(320.)
                    .show(ui, |ui| {
                        for (tag, count) in &self.tag_counts {
                            let mut checked = self.tags.contains(tag);
                            if ui
                                .checkbox(&mut checked, format!("{tag} ({count})"))
                                .changed()
                            {
                                if checked {
                                    self.tags.insert(tag.clone());
                                } else {
                                    self.tags.remove(tag);
                                }
                                changed = true;
                            }
                        }
                    });
            });
            for (i, (_, label)) in PROVENANCE_FIELDS.into_iter().enumerate() {
                ui.menu_button(label, |ui| {
                    let selected = &mut self.provenance_filters[i];
                    if ui.button("すべて").clicked() && !selected.is_empty() {
                        selected.clear();
                        changed = true;
                    }
                    egui::ScrollArea::vertical()
                        .max_height(320.)
                        .show(ui, |ui| {
                            for (code, count) in &self.provenance_counts[i] {
                                let mut checked = selected.contains(code);
                                let name = if code == "unknown" {
                                    "出典情報なし"
                                } else {
                                    code
                                };
                                if ui
                                    .checkbox(&mut checked, format!("{name} ({count})"))
                                    .changed()
                                {
                                    if checked {
                                        selected.insert(code.clone());
                                    } else {
                                        selected.remove(code);
                                    }
                                    changed = true;
                                }
                            }
                        });
                });
            }
            if ui
                .add_enabled(
                    !self.query.is_empty()
                        || !self.tags.is_empty()
                        || self
                            .provenance_filters
                            .iter()
                            .any(|selected| !selected.is_empty()),
                    egui::Button::new("絞り込みを解除"),
                )
                .clicked()
            {
                self.query.clear();
                self.tags.clear();
                self.provenance_filters.iter_mut().for_each(BTreeSet::clear);
                changed = true;
            }
        });
        if changed {
            self.rebuild(points);
        }
        ui.label(format!("{} / {} 地点", self.rows.len(), points.len()));
        let id = ui.id().with("地点一覧のキーボード");
        ui.interact(
            ui.available_rect_before_wrap(),
            id,
            egui::Sense::focusable_noninteractive(),
        );
        let mut next = None;
        if ui.memory(|memory| memory.has_focus(id)) {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        vertical_arrows: true,
                        ..Default::default()
                    },
                );
            });
            let movement = ui.input_mut(|input| {
                if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    Some(1_isize)
                } else if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    Some(-1_isize)
                } else {
                    None
                }
            });
            if let Some(movement) = movement
                && !self.rows.is_empty()
            {
                let row = selected
                    .and_then(|index| self.rows.iter().position(|&i| i == index))
                    .map_or(0, |row| {
                        row.saturating_add_signed(movement).min(self.rows.len() - 1)
                    });
                next = Some(self.rows[row]);
                self.reveal = next;
            }
        }
        let reveal = self
            .reveal
            .take()
            .and_then(|i| self.rows.iter().position(|&row| row == i));
        let mut sort_clicked = None;
        let reset_scroll = std::mem::take(&mut self.reset_scroll);
        egui::ScrollArea::horizontal()
            .id_salt("地点の列")
            .show(ui, |ui| {
                let mut table = egui_extras::TableBuilder::new(ui)
                    .striped(true)
                    .resizable(true)
                    .sense(egui::Sense::click())
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
                for width in [54., 150., 90., 145., 145., 115., 130., 120., 55., 125.] {
                    table = table.column(egui_extras::Column::initial(width).clip(true));
                }
                if reset_scroll {
                    table = table.vertical_scroll_offset(0.);
                }
                if let Some(row) = reveal {
                    table = table.animate_scrolling(false).scroll_to_row(row, None);
                }
                table
                    .header(24., |mut header| {
                        for (title, sort) in COLUMNS {
                            header.col(|ui| {
                                if let Some(sort) = sort {
                                    let suffix = if self.sort != sort {
                                        ""
                                    } else if self.descending {
                                        " ▼"
                                    } else {
                                        " ▲"
                                    };
                                    let enabled =
                                        sort != Sort::Location || self.locations.is_some();
                                    if ui
                                        .add_enabled(
                                            enabled,
                                            egui::Button::new(format!("{title}{suffix}"))
                                                .truncate(),
                                        )
                                        .clicked()
                                    {
                                        sort_clicked = Some(sort);
                                    }
                                } else {
                                    ui.label(title);
                                }
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(24., self.rows.len(), |mut row| {
                            let number = row.index();
                            let index = self.rows[number];
                            row.set_selected(next.or(selected) == Some(index));
                            row.col(|ui| {
                                ui.label((number + 1).to_string());
                            });
                            point_cells(&mut row, points.point(index), self.location_label(index));
                            if row.response().clicked() {
                                next = Some(index);
                                row.response()
                                    .ctx
                                    .memory_mut(|memory| memory.request_focus(id));
                                row.response().ctx.request_repaint();
                            }
                        })
                    });
            });
        if let Some(sort) = sort_clicked {
            self.descending = if self.sort == sort {
                !self.descending
            } else {
                sort == Sort::Elevation
            };
            self.sort = sort;
            self.sort_rows(points);
            ui.ctx().request_repaint();
        }
        (next, changed)
    }
}

const COLUMNS: [(&str, Option<Sort>); 10] = [
    ("番号", None),
    ("地点名", Some(Sort::Name)),
    ("標高 (m)", Some(Sort::Elevation)),
    ("所在地（参考）", Some(Sort::Location)),
    ("緯度・経度", Some(Sort::Coordinates)),
    ("よみがな", Some(Sort::Reading)),
    ("別名", Some(Sort::Aliases)),
    ("タグ", Some(Sort::Tags)),
    ("SVG", Some(Sort::Svg)),
    ("ID", Some(Sort::Id)),
];

fn compare_optional<T: Ord>(a: Option<T>, b: Option<T>, descending: bool) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            if descending {
                b.cmp(&a)
            } else {
                a.cmp(&b)
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

pub fn reading(point: &Point) -> String {
    point
        .name_reading
        .as_ref()
        .map_or_else(String::new, |r| r.display().into_owned())
}
pub fn aliases(point: &Point) -> String {
    point
        .aliases
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|a| {
            a.reading().map_or_else(
                || a.name().to_owned(),
                |r| format!("{}（{}）", a.name(), r.display()),
            )
        })
        .collect::<Vec<_>>()
        .join("、")
}
pub fn tags(point: &Point) -> String {
    point.tags.as_deref().unwrap_or_default().join("、")
}

pub fn point_cells(row: &mut egui_extras::TableRow<'_, '_>, point: &Point, location: &str) {
    let values = [
        point.name.clone(),
        point.elevation_m.map_or(String::new(), |v| v.to_string()),
        location.into(),
        format!("{:.6}, {:.6}", point.latitude, point.longitude),
        reading(point),
        aliases(point),
        tags(point),
        if point.graphic.is_some() {
            "あり".into()
        } else {
            String::new()
        },
    ];
    for text in values {
        row.col(|ui| {
            ui.add(egui::Label::new(text).truncate());
        });
    }
    row.col(|ui| {
        if let Some(osm) = point.osm_id.as_ref().and_then(|v| v.as_u64()) {
            ui.hyperlink_to(
                osm.to_string(),
                format!("https://www.openstreetmap.org/node/{osm}"),
            );
        } else {
            ui.add(egui::Label::new(&point.id).truncate());
        }
    });
}
fn matches_terms(point: &Point, terms: &[&str]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let mut fields = vec![point.name.to_lowercase(), point.id.to_lowercase()];
    if let Some(reading) = &point.name_reading {
        fields.extend(reading.readings().iter().map(|text| text.to_lowercase()));
    }
    for alias in point.aliases.as_deref().unwrap_or_default() {
        fields.push(alias.name().to_lowercase());
        if let Some(reading) = alias.reading() {
            fields.extend(reading.readings().iter().map(|text| text.to_lowercase()));
        }
    }
    terms
        .iter()
        .all(|term| fields.iter().any(|field| field.contains(term)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &str = r#"{"schemaFormat":"yamamuki-points","schemaVersion":6,"points":[
        {"id":"gsi:1","name":"白馬岳","nameReading":["しろうまだけ","はくばだけ"],"latitude":36.75,"longitude":137.75,"elevationM":2932,"tags":["日本百名山","山岳標高"]},
        {"id":"gsi:2","name":"富士山","provenance":{"dataSource":"osm","classification":"natural=peak;natural=volcano"},"aliases":[{"name":"富岳","nameReading":"ふがく"}],"latitude":35.36,"longitude":138.73,"elevationM":3776,"tags":["日本百名山"]},
        {"id":"gsi:3","name":"地点A","latitude":36.0,"longitude":138.0,"elevationM":null,"tags":["注記"]}
    ]}"#;

    #[test]
    fn initial_rows_follow_json_order() {
        let points = Points::read(DATA.as_bytes()).unwrap();
        let mut list = PointList::default();
        list.reset(&points);
        let ids: Vec<_> = list
            .rows
            .iter()
            .map(|&i| points.point(i).id.as_str())
            .collect();
        assert_eq!(ids, ["gsi:1", "gsi:2", "gsi:3"]);
    }

    #[test]
    fn arrow_keys_move_selection_repeatedly() {
        let points = Points::read(DATA.as_bytes()).unwrap();
        let mut list = PointList::default();
        list.reset(&points);
        let expected = list.rows.clone();
        let mut selected = Some(expected[2]);
        let ctx = egui::Context::default();
        let mut frame = |key: Option<(egui::Key, bool)>, give_focus| {
            let events = key
                .into_iter()
                .map(|(key, pressed)| egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                })
                .collect();
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let (next, _) = list.show(ui, &points, selected);
                        selected = next.or(selected);
                        if give_focus {
                            let id = ui.id().with("地点一覧のキーボード");
                            ui.memory_mut(|memory| memory.request_focus(id));
                        }
                    });
                },
            );
            selected
        };
        frame(None, true);
        frame(None, false);
        for (key, row) in [
            (egui::Key::ArrowUp, 1),
            (egui::Key::ArrowUp, 0),
            (egui::Key::ArrowDown, 1),
            (egui::Key::ArrowDown, 2),
        ] {
            assert_eq!(frame(Some((key, true)), false), Some(expected[row]));
            frame(Some((key, false)), false);
        }
    }

    #[test]
    fn search_uses_readings_and_aliases_and_combines_with_tags() {
        let points = Points::read(DATA.as_bytes()).unwrap();
        let mut list = PointList::default();
        list.reset(&points);
        list.query = "ふがく GSI".into();
        list.tags.insert("日本百名山".into());
        list.tags.insert("注記".into());
        list.rebuild(&points);
        assert_eq!(list.rows.len(), 1);
        let selected = list.rows[0];
        assert_eq!(points.point(selected).name, "富士山");
        let marks = points.filtered_marks([[0.; 2], [1.; 2]], 2_f64.powi(26), list.prefix());
        assert_eq!(marks.iter().filter(|mark| mark.matched).count(), 1);
        assert_eq!(
            marks.iter().find(|mark| mark.matched).unwrap().index,
            selected
        );
        list.query = "はくばだけ".into();
        list.rebuild(&points);
        assert_eq!(points.point(list.rows[0]).name, "白馬岳");
    }

    #[test]
    fn filters_source_and_classification_without_removing_map_markers() {
        let points = Points::read(DATA.as_bytes()).unwrap();
        let mut list = PointList::default();
        list.reset(&points);
        list.provenance_filters[0].insert("osm".into());
        list.provenance_filters[1].insert("natural=volcano".into());
        list.rebuild(&points);
        assert_eq!(list.visible_count(), 1);
        let marks = points.filtered_marks([[0.; 2], [1.; 2]], 2_f64.powi(26), list.prefix());
        assert_eq!(marks.len(), 3);
        assert_eq!(marks.iter().filter(|mark| !mark.matched).count(), 2);
        assert_eq!(points.point(list.rows[0]).name, "富士山");
        let other = marks.iter().find(|mark| !mark.matched).unwrap().index;
        assert!(list.clear_filter_to_reveal(&points, other));
        assert_eq!(list.visible_count(), 3);
    }

    #[test]
    fn elevation_sort_keeps_missing_values_last_in_both_directions() {
        let points = Points::read(DATA.as_bytes()).unwrap();
        let mut list = PointList::default();
        list.reset(&points);
        list.sort = Sort::Elevation;
        for descending in [false, true] {
            list.descending = descending;
            list.sort_rows(&points);
            let values: Vec<_> = list
                .rows
                .iter()
                .map(|&i| points.point(i).elevation_m)
                .collect();
            let expected = if descending {
                [Some(3776.), Some(2932.), None]
            } else {
                [Some(2932.), Some(3776.), None]
            };
            assert_eq!(values, expected);
        }
    }
}
