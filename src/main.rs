#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod games;

use eframe::egui::{self, Color32, Margin, Pos2, Rect, RichText, Rounding, Sense, Stroke, Vec2};
use games::{default_games, Category, Game, Status};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewFilter {
    All,
    Priority,
    Todo,
    Played,
    Dropped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformFilter {
    All,
    GameJolt,
    ItchIo,
    Steam,
}

impl PlatformFilter {
    fn label(self) -> &'static str {
        match self {
            PlatformFilter::All => "All Platforms",
            PlatformFilter::GameJolt => "Game Jolt",
            PlatformFilter::ItchIo => "itch.io",
            PlatformFilter::Steam => "Steam",
        }
    }

    fn matches(self, platform: &str) -> bool {
        match self {
            PlatformFilter::All => true,
            PlatformFilter::GameJolt => platform.contains("Game Jolt"),
            PlatformFilter::ItchIo => platform.contains("itch.io"),
            PlatformFilter::Steam => platform.contains("Steam"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedState {
    games: Vec<Game>,
    separate_played_section: bool,
}

#[derive(Debug, Clone)]
enum UndoAction {
    StatusChange { id: String, prev: Status },
    PriorityToggle { id: String, prev: bool, prev_rank: u32 },
}

enum CardAction {
    SetStatus(String, Status),
    TogglePriority(String),
    MovePriority(String, i32), // -1 up, +1 down
    OpenUrl(String),
    DeleteCustom(String),
}

struct BacklogApp {
    games: Vec<Game>,
    view_filter: ViewFilter,
    category_filter: Option<Category>,
    platform_filter: PlatformFilter,
    search: String,
    separate_played_section: bool,

    show_add_panel: bool,
    new_title: String,
    new_creator: String,
    new_platform: String,
    new_category: Category,
    new_hook: String,
    new_url: String,
    new_priority: bool,

    toast: Option<(String, Instant)>,
    last_undo: Option<UndoAction>,
    confirm_reset: bool,
}

impl BacklogApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_theme(&cc.egui_ctx);
        let loaded = load_state();
        let mut merged_new = false;
        let (mut games, separate_played_section) = match loaded {
            Some(st) if !st.games.is_empty() => (st.games, st.separate_played_section),
            _ => (default_games(), true),
        };
        let before_dedup = games.len();
        games.retain(|g| g.id != "bondees-barnyard-safety-violation");
        if games.len() != before_dedup {
            merged_new = true;
        }
        for def in default_games() {
            if let Some(existing) = games.iter_mut().find(|g| g.id == def.id) {
                if existing.direct_url.is_none() && def.direct_url.is_some() {
                    existing.direct_url = def.direct_url;
                    merged_new = true;
                }
            } else {
                games.push(def);
                merged_new = true;
            }
        }

        let mut app = Self {
            games,
            view_filter: ViewFilter::All,
            category_filter: None,
            platform_filter: PlatformFilter::All,
            search: String::new(),
            separate_played_section,
            show_add_panel: false,
            new_title: String::new(),
            new_creator: String::new(),
            new_platform: "Game Jolt".to_string(),
            new_category: Category::Favorites,
            new_hook: String::new(),
            new_url: String::new(),
            new_priority: true,
            toast: None,
            last_undo: None,
            confirm_reset: false,
        };
        app.normalize_priority_ranks();
        if merged_new {
            app.persist();
        }
        app
    }

    fn normalize_priority_ranks(&mut self) {
        let mut prio_indices: Vec<usize> = self
            .games
            .iter()
            .enumerate()
            .filter(|(_, g)| g.priority && g.status == Status::Todo)
            .map(|(i, _)| i)
            .collect();
        prio_indices.sort_by_key(|&i| (self.games[i].priority_rank, self.games[i].title.clone()));
        for (rank_idx, &gi) in prio_indices.iter().enumerate() {
            self.games[gi].priority_rank = (rank_idx as u32) + 1;
        }
    }

    fn persist(&self) {
        let st = PersistedState {
            games: self.games.clone(),
            separate_played_section: self.separate_played_section,
        };
        if let Ok(json) = serde_json::to_string_pretty(&st) {
            let _ = std::fs::write(state_file_path(), json);
        }
    }

    fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn next_priority_rank(&self) -> u32 {
        self.games
            .iter()
            .filter(|g| g.priority && g.status == Status::Todo)
            .map(|g| g.priority_rank)
            .max()
            .unwrap_or(0)
            + 1
    }

    fn apply_actions(&mut self, actions: Vec<CardAction>) {
        if actions.is_empty() {
            return;
        }
        let mut changed = false;
        for action in actions {
            match action {
                CardAction::SetStatus(id, new_status) => {
                    let next_rank = self.next_priority_rank();
                    if let Some(g) = self.games.iter_mut().find(|g| g.id == id) {
                        if g.status != new_status {
                            let prev = g.status;
                            let title = g.title.clone();
                            g.status = new_status;
                            if new_status == Status::Todo && g.priority && g.priority_rank == 0 {
                                g.priority_rank = next_rank;
                            }
                            self.last_undo = Some(UndoAction::StatusChange { id, prev });
                            let label = match new_status {
                                Status::Played => "Moved to Played Vault",
                                Status::Todo => "Moved to To-Do",
                                Status::Dropped => "Moved to Dropped",
                            };
                            self.set_toast(format!("{title} -> {label}"));
                            changed = true;
                        }
                    }
                }
                CardAction::TogglePriority(id) => {
                    let next_rank = self.next_priority_rank();
                    if let Some(g) = self.games.iter_mut().find(|g| g.id == id) {
                        let prev = g.priority;
                        let prev_rank = g.priority_rank;
                        let title = g.title.clone();
                        g.priority = !prev;
                        if g.priority {
                            g.priority_rank = next_rank;
                            if g.status == Status::Dropped {
                                g.status = Status::Todo;
                            }
                            self.set_toast(format!("{title} pinned to Priority Queue"));
                        } else {
                            g.priority_rank = 0;
                            self.set_toast(format!("{title} removed from Priority Queue"));
                        }
                        self.last_undo = Some(UndoAction::PriorityToggle {
                            id,
                            prev,
                            prev_rank,
                        });
                        changed = true;
                    }
                }
                CardAction::MovePriority(id, dir) => {
                    self.normalize_priority_ranks();
                    let mut prio_indices: Vec<usize> = self
                        .games
                        .iter()
                        .enumerate()
                        .filter(|(_, g)| g.priority && g.status == Status::Todo)
                        .map(|(i, _)| i)
                        .collect();
                    prio_indices.sort_by_key(|&i| {
                        (self.games[i].priority_rank, self.games[i].title.clone())
                    });
                    if let Some(pos) = prio_indices.iter().position(|&i| self.games[i].id == id) {
                        let swap_pos = if dir < 0 && pos > 0 {
                            Some(pos - 1)
                        } else if dir > 0 && pos + 1 < prio_indices.len() {
                            Some(pos + 1)
                        } else {
                            None
                        };
                        if let Some(sp) = swap_pos {
                            let a = prio_indices[pos];
                            let b = prio_indices[sp];
                            let ra = self.games[a].priority_rank;
                            self.games[a].priority_rank = self.games[b].priority_rank;
                            self.games[b].priority_rank = ra;
                            changed = true;
                        }
                    }
                }
                CardAction::OpenUrl(url) => {
                    open_url(&url);
                }
                CardAction::DeleteCustom(id) => {
                    let before = self.games.len();
                    self.games.retain(|g| g.id != id);
                    if self.games.len() != before {
                        self.last_undo = None;
                        self.set_toast("Removed custom game");
                        changed = true;
                    }
                }
            }
        }
        if changed {
            self.normalize_priority_ranks();
            self.persist();
        }
    }

    fn apply_undo(&mut self) {
        if let Some(undo) = self.last_undo.take() {
            match undo {
                UndoAction::StatusChange { id, prev } => {
                    if let Some(g) = self.games.iter_mut().find(|g| g.id == id) {
                        g.status = prev;
                    }
                }
                UndoAction::PriorityToggle {
                    id,
                    prev,
                    prev_rank,
                } => {
                    if let Some(g) = self.games.iter_mut().find(|g| g.id == id) {
                        g.priority = prev;
                        g.priority_rank = prev_rank;
                    }
                }
            }
            self.normalize_priority_ranks();
            self.toast = None;
            self.persist();
        }
    }

    fn matches_filters(&self, g: &Game) -> bool {
        match self.view_filter {
            ViewFilter::All => {}
            ViewFilter::Priority => {
                if !g.priority || g.status != Status::Todo {
                    return false;
                }
            }
            ViewFilter::Todo => {
                if g.status != Status::Todo {
                    return false;
                }
            }
            ViewFilter::Played => {
                if g.status != Status::Played {
                    return false;
                }
            }
            ViewFilter::Dropped => {
                if g.status != Status::Dropped {
                    return false;
                }
            }
        }
        if let Some(cat) = self.category_filter {
            if g.category != cat {
                return false;
            }
        }
        if !self.platform_filter.matches(&g.platform) {
            return false;
        }
        let q = self.search.trim().to_lowercase();
        if !q.is_empty() {
            let hit = g.title.to_lowercase().contains(&q)
                || g.creator.to_lowercase().contains(&q)
                || g.platform.to_lowercase().contains(&q)
                || g.hook.to_lowercase().contains(&q)
                || g.youtubers.iter().any(|y| y.to_lowercase().contains(&q));
            if !hit {
                return false;
            }
        }
        true
    }
}

impl eframe::App for BacklogApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some((_, ts)) = &self.toast {
            if ts.elapsed() > Duration::from_secs(5) {
                self.toast = None;
            }
        }

        // Top Panel: Dashboard, Progress Bar, Filters, Add Game
        egui::TopBottomPanel::top("header_panel")
            .frame(egui::Frame {
                fill: Color32::from_rgb(18, 21, 28),
                inner_margin: Margin::symmetric(18.0, 14.0),
                stroke: Stroke::new(1.0_f32, Color32::from_rgb(42, 48, 60)),
                ..Default::default()
            })
            .show(ctx, |ui| {
                // Row 1: Title + Top Buttons
                ui.horizontal_wrapped(|ui| {
                    ui.heading(
                        RichText::new("FNAF Fangame Priority & Backlog Tracker")
                            .size(21.0)
                            .strong()
                            .color(Color32::from_rgb(243, 245, 249)),
                    );
                    ui.add_space(6.0);
                    let badge = egui::Frame {
                        fill: Color32::from_rgb(32, 37, 48),
                        rounding: Rounding::same(12.0),
                        inner_margin: Margin::symmetric(10.0, 3.0),
                        stroke: Stroke::new(1.0_f32, Color32::from_rgb(56, 64, 82)),
                        ..Default::default()
                    };
                    badge.show(ui, |ui| {
                        ui.label(
                            RichText::new(format!("{} Games", self.games.len()))
                                .size(12.0)
                                .color(Color32::from_rgb(185, 194, 210)),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.confirm_reset {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Confirm Reset").color(Color32::WHITE),
                                    )
                                    .fill(Color32::from_rgb(190, 45, 65)),
                                )
                                .clicked()
                            {
                                self.games = default_games();
                                self.separate_played_section = true;
                                self.confirm_reset = false;
                                self.last_undo = None;
                                self.persist();
                                self.set_toast("Reset backlog to defaults");
                            }
                            if ui.button("Cancel").clicked() {
                                self.confirm_reset = false;
                            }
                        } else if ui.button("Reset Defaults").clicked() {
                            self.confirm_reset = true;
                        }

                        let layout_label = if self.separate_played_section {
                            "Layout: Separate Played Vault"
                        } else {
                            "Layout: Keep Played Inline"
                        };
                        if ui.button(layout_label).clicked() {
                            self.separate_played_section = !self.separate_played_section;
                            self.persist();
                        }

                        let add_btn = egui::Button::new(
                            RichText::new(if self.show_add_panel {
                                "Close Add Form"
                            } else {
                                "+ Add Game"
                            })
                            .strong()
                            .color(Color32::from_rgb(15, 23, 20)),
                        )
                        .fill(Color32::from_rgb(245, 185, 66));
                        if ui.add(add_btn).clicked() {
                            self.show_add_panel = !self.show_add_panel;
                        }
                    });
                });

                ui.add_space(10.0);

                // Row 2: Segmented Visual Progress Bar
                let total = self.games.len().max(1) as f32;
                let played_count = self
                    .games
                    .iter()
                    .filter(|g| g.status == Status::Played)
                    .count();
                let prio_count = self
                    .games
                    .iter()
                    .filter(|g| g.status == Status::Todo && g.priority)
                    .count();
                let todo_full_count = self
                    .games
                    .iter()
                    .filter(|g| g.status == Status::Todo && !g.priority && !g.is_demo)
                    .count();
                let todo_demo_count = self
                    .games
                    .iter()
                    .filter(|g| g.status == Status::Todo && !g.priority && g.is_demo)
                    .count();
                let dropped_count = self
                    .games
                    .iter()
                    .filter(|g| g.status == Status::Dropped)
                    .count();

                let todo_total = prio_count + todo_full_count + todo_demo_count;
                let active_total = played_count + todo_total;
                let pct = if active_total > 0 {
                    ((played_count as f32 / active_total as f32) * 100.0).round() as usize
                } else {
                    0
                };

                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "Completion: {pct}% ({played_count}/{active_total})"
                        ))
                        .size(13.0)
                        .strong()
                        .color(Color32::from_rgb(52, 211, 153)),
                    );
                    ui.add_space(14.0);
                    paint_legend_dot(ui, Color32::from_rgb(16, 185, 129), "Played");
                    paint_legend_dot(ui, Color32::from_rgb(245, 185, 66), "Priority");
                    paint_legend_dot(ui, Color32::from_rgb(217, 119, 6), "To Play");
                    paint_legend_dot(ui, Color32::from_rgb(6, 182, 212), "Demos");
                    paint_legend_dot(ui, Color32::from_rgb(225, 29, 72), "Dropped");
                });

                ui.add_space(4.0);
                let bar_w = ui.available_width();
                let (bar_rect, _) =
                    ui.allocate_exact_size(Vec2::new(bar_w, 12.0), Sense::hover());
                let painter = ui.painter();
                painter.rect_filled(bar_rect, Rounding::same(6.0), Color32::from_rgb(28, 33, 44));

                let segments = [
                    (played_count as f32 / total, Color32::from_rgb(16, 185, 129)),
                    (prio_count as f32 / total, Color32::from_rgb(245, 185, 66)),
                    (
                        todo_full_count as f32 / total,
                        Color32::from_rgb(217, 119, 6),
                    ),
                    (
                        todo_demo_count as f32 / total,
                        Color32::from_rgb(6, 182, 212),
                    ),
                    (dropped_count as f32 / total, Color32::from_rgb(225, 29, 72)),
                ];
                let mut x = bar_rect.left();
                for (frac, col) in segments {
                    if frac > 0.0 {
                        let w = (bar_rect.width() * frac).max(3.0);
                        let seg = Rect::from_min_max(
                            Pos2::new(x, bar_rect.top()),
                            Pos2::new((x + w).min(bar_rect.right()), bar_rect.bottom()),
                        );
                        painter.rect_filled(seg, Rounding::same(2.0), col);
                        x += w;
                    }
                }

                ui.add_space(10.0);

                // Row 3: Status Filter Tabs
                ui.horizontal_wrapped(|ui| {
                    let tabs = [
                        (
                            ViewFilter::All,
                            format!("All ({})", self.games.len()),
                            Color32::from_rgb(148, 163, 184),
                        ),
                        (
                            ViewFilter::Priority,
                            format!("Priority Queue ({prio_count})"),
                            Color32::from_rgb(245, 185, 66),
                        ),
                        (
                            ViewFilter::Todo,
                            format!("To Play ({todo_total})"),
                            Color32::from_rgb(217, 119, 6),
                        ),
                        (
                            ViewFilter::Played,
                            format!("Played Vault ({played_count})"),
                            Color32::from_rgb(16, 185, 129),
                        ),
                        (
                            ViewFilter::Dropped,
                            format!("Dropped ({dropped_count})"),
                            Color32::from_rgb(225, 29, 72),
                        ),
                    ];

                    for (vf, text, accent) in tabs {
                        let active = self.view_filter == vf;
                        let fill = if active {
                            accent.linear_multiply(0.28)
                        } else {
                            Color32::from_rgb(26, 30, 40)
                        };
                        let stroke = if active {
                            Stroke::new(1.5_f32, accent)
                        } else {
                            Stroke::new(1.0_f32, Color32::from_rgb(48, 55, 70))
                        };
                        let btn = egui::Button::new(
                            RichText::new(text)
                                .size(13.0)
                                .strong()
                                .color(if active {
                                    Color32::WHITE
                                } else {
                                    Color32::from_rgb(190, 198, 212)
                                }),
                        )
                        .fill(fill)
                        .stroke(stroke)
                        .rounding(Rounding::same(8.0));
                        if ui.add(btn).clicked() {
                            self.view_filter = vf;
                        }
                    }
                });

                ui.add_space(8.0);

                // Row 4: Search Box + Category + Platform Filters
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .desired_width(300.0)
                            .hint_text("Search title, creator, mechanic, or YouTuber (3FS, Pastra...)"),
                    );
                    if !self.search.is_empty() && ui.button("Clear").clicked() {
                        self.search.clear();
                    }

                    ui.separator();

                    egui::ComboBox::from_id_salt("platform_combo")
                        .selected_text(self.platform_filter.label())
                        .show_ui(ui, |ui| {
                            for pf in [
                                PlatformFilter::All,
                                PlatformFilter::GameJolt,
                                PlatformFilter::ItchIo,
                                PlatformFilter::Steam,
                            ] {
                                ui.selectable_value(&mut self.platform_filter, pf, pf.label());
                            }
                        });

                    ui.separator();

                    let all_active = self.category_filter.is_none();
                    if ui
                        .selectable_label(all_active, RichText::new("All Categories").size(12.5))
                        .clicked()
                    {
                        self.category_filter = None;
                    }
                    for cat in Category::ALL {
                        let active = self.category_filter == Some(cat);
                        if ui
                            .selectable_label(active, RichText::new(cat.short_label()).size(12.5))
                            .clicked()
                        {
                            self.category_filter = if active { None } else { Some(cat) };
                        }
                    }
                });

                // Optional Row 5: Add Game Form
                if self.show_add_panel {
                    ui.add_space(10.0);
                    egui::Frame {
                        fill: Color32::from_rgb(24, 29, 39),
                        stroke: Stroke::new(1.0_f32, Color32::from_rgb(70, 80, 100)),
                        rounding: Rounding::same(10.0),
                        inner_margin: Margin::same(12.0),
                        ..Default::default()
                    }
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Add a Game to Your Backlog")
                                .strong()
                                .size(14.0),
                        );
                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.new_title)
                                    .desired_width(210.0)
                                    .hint_text("Game Title *"),
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut self.new_creator)
                                    .desired_width(140.0)
                                    .hint_text("Creator"),
                            );
                            egui::ComboBox::from_id_salt("new_platform_combo")
                                .selected_text(&self.new_platform)
                                .show_ui(ui, |ui| {
                                    for p in ["Game Jolt", "itch.io", "Game Jolt & itch.io", "Steam"]
                                    {
                                        ui.selectable_value(
                                            &mut self.new_platform,
                                            p.to_string(),
                                            p,
                                        );
                                    }
                                });
                            egui::ComboBox::from_id_salt("new_cat_combo")
                                .selected_text(self.new_category.short_label())
                                .show_ui(ui, |ui| {
                                    for c in Category::ALL {
                                        ui.selectable_value(
                                            &mut self.new_category,
                                            c,
                                            c.short_label(),
                                        );
                                    }
                                });
                            ui.checkbox(&mut self.new_priority, "Pin as Priority");
                        });
                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.new_hook)
                                    .desired_width(380.0)
                                    .hint_text("What makes it cool / unique?"),
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut self.new_url)
                                    .desired_width(260.0)
                                    .hint_text("Direct URL (optional; auto-searches otherwise)"),
                            );
                            let submit_btn = egui::Button::new(
                                RichText::new("Save Game")
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(16, 185, 129));
                            if ui.add(submit_btn).clicked() && !self.new_title.trim().is_empty() {
                                let title = self.new_title.trim().to_string();
                                let rank = self.next_priority_rank();
                                let id = format!(
                                    "custom-{}",
                                    std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_millis())
                                        .unwrap_or(1)
                                );
                                let is_demo = self.new_category == Category::Demos;
                                let direct_url = if self.new_url.trim().is_empty() {
                                    None
                                } else {
                                    Some(self.new_url.trim().to_string())
                                };
                                let new_game = Game {
                                    id,
                                    title: title.clone(),
                                    creator: if self.new_creator.trim().is_empty() {
                                        "Indie Dev".to_string()
                                    } else {
                                        self.new_creator.trim().to_string()
                                    },
                                    platform: self.new_platform.clone(),
                                    release: if is_demo {
                                        "Playable Demo".to_string()
                                    } else {
                                        "Full Game".to_string()
                                    },
                                    category: self.new_category,
                                    youtubers: Vec::new(),
                                    hook: if self.new_hook.trim().is_empty() {
                                        "Added manually to backlog.".to_string()
                                    } else {
                                        self.new_hook.trim().to_string()
                                    },
                                    status: Status::Todo,
                                    priority: self.new_priority,
                                    priority_rank: if self.new_priority { rank } else { 0 },
                                    is_demo,
                                    direct_url,
                                };
                                self.games.insert(0, new_game);
                                self.new_title.clear();
                                self.new_creator.clear();
                                self.new_hook.clear();
                                self.new_url.clear();
                                self.show_add_panel = false;
                                self.persist();
                                self.set_toast(format!("Added '{title}' to backlog"));
                            }
                        });
                    });
                }
            });

        // Bottom Toast Bar (with Undo)
        if let Some((msg, _)) = self.toast.clone() {
            egui::TopBottomPanel::bottom("toast_bar")
                .frame(egui::Frame {
                    fill: Color32::from_rgb(24, 30, 40),
                    inner_margin: Margin::symmetric(18.0, 10.0),
                    stroke: Stroke::new(1.0_f32, Color32::from_rgb(52, 211, 153)),
                    ..Default::default()
                })
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(msg)
                                .size(13.5)
                                .strong()
                                .color(Color32::from_rgb(230, 244, 238)),
                        );
                        if self.last_undo.is_some()
                            && ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Undo")
                                            .strong()
                                            .color(Color32::from_rgb(245, 185, 66)),
                                    )
                                    .fill(Color32::from_rgb(38, 44, 58)),
                                )
                                .clicked()
                        {
                            self.apply_undo();
                        }
                    });
                });
        }

        // Main Scrollable Backlog Content
        let mut pending_actions: Vec<CardAction> = Vec::new();
        egui::CentralPanel::default()
            .frame(egui::Frame {
                fill: Color32::from_rgb(13, 15, 20),
                inner_margin: Margin::symmetric(18.0, 14.0),
                ..Default::default()
            })
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let filtered: Vec<Game> = self
                            .games
                            .iter()
                            .filter(|g| self.matches_filters(g))
                            .cloned()
                            .collect();

                        if filtered.is_empty() {
                            ui.add_space(40.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new("No games match your current filters.")
                                        .size(16.0)
                                        .strong(),
                                );
                                ui.label(
                                    RichText::new(
                                        "Try clearing your search or switching status tabs.",
                                    )
                                    .size(13.0)
                                    .color(Color32::from_rgb(148, 163, 184)),
                                );
                            });
                            return;
                        }

                        // 1. PRIORITY QUEUE SECTION (Pinned at Top when viewing All, Priority, or Todo)
                        if matches!(
                            self.view_filter,
                            ViewFilter::All | ViewFilter::Priority | ViewFilter::Todo
                        ) {
                            let mut prio_games: Vec<Game> = filtered
                                .iter()
                                .filter(|g| g.priority && g.status == Status::Todo)
                                .cloned()
                                .collect();
                            prio_games.sort_by_key(|g| (g.priority_rank, g.title.clone()));

                            if !prio_games.is_empty() {
                                render_section_header(
                                    ui,
                                    "PRIORITY QUEUE (NEXT TO PLAY)",
                                    "Star any game below to pin it here, or use Up/Dn to order your queue",
                                    prio_games.len(),
                                    Color32::from_rgb(245, 185, 66),
                                );
                                let prio_len = prio_games.len();
                                for (idx, g) in prio_games.iter().enumerate() {
                                    render_game_card(
                                        ui,
                                        g,
                                        true,
                                        idx > 0,
                                        idx + 1 < prio_len,
                                        &mut pending_actions,
                                    );
                                    ui.add_space(7.0);
                                }
                                ui.add_space(14.0);
                            }
                        }

                        if self.view_filter == ViewFilter::Priority {
                            return;
                        }

                        if self.view_filter == ViewFilter::Played {
                            let played: Vec<Game> = filtered
                                .into_iter()
                                .filter(|g| g.status == Status::Played)
                                .collect();
                            render_section_header(
                                ui,
                                "PLAYED / COMPLETED VAULT",
                                "Green checkmark + emerald highlight means played. Click 'Move to To-Do' to restore.",
                                played.len(),
                                Color32::from_rgb(16, 185, 129),
                            );
                            for g in &played {
                                render_game_card(ui, g, false, false, false, &mut pending_actions);
                                ui.add_space(7.0);
                            }
                            return;
                        }

                        if self.view_filter == ViewFilter::Dropped {
                            let dropped: Vec<Game> = filtered
                                .into_iter()
                                .filter(|g| g.status == Status::Dropped)
                                .collect();
                            render_section_header(
                                ui,
                                "DROPPED / SKIPPED",
                                "Skipped games. Click 'Restore' anytime to put back into To-Do.",
                                dropped.len(),
                                Color32::from_rgb(225, 29, 72),
                            );
                            for g in &dropped {
                                render_game_card(ui, g, false, false, false, &mut pending_actions);
                                ui.add_space(7.0);
                            }
                            return;
                        }

                        // ViewFilter::All or ViewFilter::Todo
                        for cat in Category::ALL {
                            let cat_games: Vec<Game> = filtered
                                .iter()
                                .filter(|g| {
                                    if g.category != cat {
                                        return false;
                                    }
                                    // Avoid duplicating priority games already rendered in Priority Queue above
                                    if g.priority && g.status == Status::Todo {
                                        return false;
                                    }
                                    if self.separate_played_section && g.status != Status::Todo {
                                        return false;
                                    }
                                    true
                                })
                                .cloned()
                                .collect();

                            if !cat_games.is_empty() {
                                render_section_header(
                                    ui,
                                    cat.title(),
                                    "",
                                    cat_games.len(),
                                    category_color(cat),
                                );
                                for g in &cat_games {
                                    render_game_card(
                                        ui,
                                        g,
                                        false,
                                        false,
                                        false,
                                        &mut pending_actions,
                                    );
                                    ui.add_space(7.0);
                                }
                                ui.add_space(12.0);
                            }
                        }

                        // Separate Played Vault & Dropped sections at bottom when enabled
                        if self.separate_played_section && self.view_filter == ViewFilter::All {
                            let played: Vec<Game> = filtered
                                .iter()
                                .filter(|g| g.status == Status::Played)
                                .cloned()
                                .collect();
                            if !played.is_empty() {
                                ui.separator();
                                ui.add_space(10.0);
                                render_section_header(
                                    ui,
                                    "PLAYED / COMPLETED VAULT",
                                    "Completed games automatically move here so your queue stays clean",
                                    played.len(),
                                    Color32::from_rgb(16, 185, 129),
                                );
                                for g in &played {
                                    render_game_card(
                                        ui,
                                        g,
                                        false,
                                        false,
                                        false,
                                        &mut pending_actions,
                                    );
                                    ui.add_space(7.0);
                                }
                                ui.add_space(12.0);
                            }

                            let dropped: Vec<Game> = filtered
                                .iter()
                                .filter(|g| g.status == Status::Dropped)
                                .cloned()
                                .collect();
                            if !dropped.is_empty() {
                                ui.separator();
                                ui.add_space(10.0);
                                render_section_header(
                                    ui,
                                    "DROPPED / SKIPPED",
                                    "Click 'Restore' to move any game back to your To-Do list",
                                    dropped.len(),
                                    Color32::from_rgb(225, 29, 72),
                                );
                                for g in &dropped {
                                    render_game_card(
                                        ui,
                                        g,
                                        false,
                                        false,
                                        false,
                                        &mut pending_actions,
                                    );
                                    ui.add_space(7.0);
                                }
                            }
                        }
                    });
            });

        self.apply_actions(pending_actions);
    }
}

fn configure_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.window_fill = Color32::from_rgb(13, 15, 20);
    style.visuals.panel_fill = Color32::from_rgb(13, 15, 20);
    style.visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(22, 26, 35);
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(30, 35, 48);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(44, 52, 70);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(58, 68, 90);
    ctx.set_style(style);
}

fn category_color(cat: Category) -> Color32 {
    match cat {
        Category::Favorites => Color32::from_rgb(245, 158, 11),
        Category::Rewritten => Color32::from_rgb(167, 139, 250),
        Category::Gritty => Color32::from_rgb(239, 68, 68),
        Category::Subversive => Color32::from_rgb(59, 130, 246),
        Category::Demos => Color32::from_rgb(6, 182, 212),
        Category::SoonToRelease => Color32::from_rgb(16, 185, 129),
    }
}

fn paint_legend_dot(ui: &mut egui::Ui, color: Color32, label: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 14.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.5, color);
        ui.label(
            RichText::new(label)
                .size(12.0)
                .color(Color32::from_rgb(175, 184, 200)),
        );
    });
}

fn render_section_header(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: &str,
    count: usize,
    accent: Color32,
) {
    ui.horizontal_wrapped(|ui| {
        let (dot_rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 20.0), Sense::hover());
        ui.painter().rect_filled(
            Rect::from_center_size(dot_rect.center(), Vec2::new(4.0, 16.0)),
            Rounding::same(2.0),
            accent,
        );
        ui.label(
            RichText::new(title)
                .size(16.0)
                .strong()
                .color(Color32::from_rgb(240, 242, 248)),
        );
        let pill = egui::Frame {
            fill: accent.linear_multiply(0.22),
            stroke: Stroke::new(1.0_f32, accent.linear_multiply(0.6)),
            rounding: Rounding::same(10.0),
            inner_margin: Margin::symmetric(8.0, 1.5),
            ..Default::default()
        };
        pill.show(ui, |ui| {
            ui.label(
                RichText::new(count.to_string())
                    .size(12.0)
                    .strong()
                    .color(accent),
            );
        });
    });
    if !subtitle.is_empty() {
        ui.label(
            RichText::new(subtitle)
                .size(12.0)
                .color(Color32::from_rgb(142, 152, 170)),
        );
    }
    ui.add_space(6.0);
}

fn render_game_card(
    ui: &mut egui::Ui,
    game: &Game,
    in_priority_section: bool,
    can_move_up: bool,
    can_move_down: bool,
    actions: &mut Vec<CardAction>,
) {
    let is_played = game.status == Status::Played;
    let is_dropped = game.status == Status::Dropped;
    let is_priority = game.priority && game.status == Status::Todo;

    let (bg_fill, border_col, stripe_col) = if is_played {
        (
            Color32::from_rgb(16, 42, 29),
            Color32::from_rgb(34, 150, 98),
            Color32::from_rgb(16, 185, 129),
        )
    } else if is_dropped {
        (
            Color32::from_rgb(30, 20, 24),
            Color32::from_rgb(110, 45, 60),
            Color32::from_rgb(225, 29, 72),
        )
    } else if is_priority {
        (
            Color32::from_rgb(40, 32, 16),
            Color32::from_rgb(180, 130, 35),
            Color32::from_rgb(245, 185, 66),
        )
    } else {
        (
            Color32::from_rgb(22, 26, 35),
            Color32::from_rgb(46, 54, 70),
            category_color(game.category),
        )
    };

    let frame = egui::Frame {
        fill: bg_fill,
        stroke: Stroke::new(1.0_f32, border_col),
        rounding: Rounding::same(10.0),
        inner_margin: Margin {
            left: 16.0,
            right: 12.0,
            top: 10.0,
            bottom: 10.0,
        },
        ..Default::default()
    };

    let frame_resp = frame.show(ui, |ui| {
        ui.horizontal_top(|ui| {
            // Left Tactile Painted Status Indicator (Click to toggle Played!)
            let (check_rect, check_resp) =
                ui.allocate_exact_size(Vec2::new(28.0, 28.0), Sense::click());
            paint_status_circle(ui.painter(), check_rect.center(), game.status, check_resp.hovered());
            if check_resp
                .on_hover_text(if is_played {
                    "Click to move back to To-Do"
                } else {
                    "Click to mark as PLAYED"
                })
                .clicked()
            {
                let next = if is_played || is_dropped {
                    Status::Todo
                } else {
                    Status::Played
                };
                actions.push(CardAction::SetStatus(game.id.clone(), next));
            }

            ui.add_space(6.0);

            // Middle: Title, Status Pill, Metadata, Hook, YouTubers
            ui.vertical(|ui| {
                ui.set_min_width((ui.available_width() - 250.0).max(220.0));

                ui.horizontal_wrapped(|ui| {
                    if is_played {
                        draw_pill(
                            ui,
                            "PLAYED",
                            Color32::from_rgb(16, 185, 129),
                            Color32::WHITE,
                        );
                    } else if is_dropped {
                        draw_pill(
                            ui,
                            "DROPPED",
                            Color32::from_rgb(159, 18, 57),
                            Color32::from_rgb(255, 228, 230),
                        );
                    } else if is_priority {
                        draw_pill(
                            ui,
                            &format!("#{} PRIORITY", game.priority_rank.max(1)),
                            Color32::from_rgb(245, 185, 66),
                            Color32::from_rgb(22, 16, 4),
                        );
                    } else if game.is_demo {
                        draw_pill(
                            ui,
                            "DEMO / ALPHA",
                            Color32::from_rgb(14, 116, 144),
                            Color32::from_rgb(207, 250, 254),
                        );
                    }

                    let title_text = if is_played || is_dropped {
                        RichText::new(&game.title)
                            .size(15.5)
                            .strong()
                            .strikethrough()
                            .color(Color32::from_rgb(148, 163, 184))
                    } else {
                        RichText::new(&game.title)
                            .size(15.5)
                            .strong()
                            .color(Color32::from_rgb(244, 246, 250))
                    };
                    ui.label(title_text);

                    draw_outline_pill(ui, &game.platform, stripe_col);
                });

                ui.label(
                    RichText::new(format!("By {}  |  {}", game.creator, game.release))
                        .size(12.0)
                        .color(Color32::from_rgb(155, 165, 184)),
                );

                ui.add_space(2.0);
                ui.label(
                    RichText::new(&game.hook)
                        .size(13.0)
                        .color(if is_played || is_dropped {
                            Color32::from_rgb(148, 163, 184)
                        } else {
                            Color32::from_rgb(215, 222, 235)
                        }),
                );

                if !game.youtubers.is_empty() {
                    ui.add_space(3.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new("Covered by:")
                                .size(12.0)
                                .color(Color32::from_rgb(138, 148, 166)),
                        );
                        for yt in &game.youtubers {
                            draw_outline_pill(ui, yt, Color32::from_rgb(90, 102, 125));
                        }
                    });
                }
            });

            // Right Action Column: Priority, Mark Played/Undo, Drop, Lookup Links
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        // Priority reorder arrows when in priority section
                        if in_priority_section {
                            if ui
                                .add_enabled(can_move_up, egui::Button::new("Up"))
                                .on_hover_text("Move higher in Priority Queue")
                                .clicked()
                            {
                                actions.push(CardAction::MovePriority(game.id.clone(), -1));
                            }
                            if ui
                                .add_enabled(can_move_down, egui::Button::new("Dn"))
                                .on_hover_text("Move lower in Priority Queue")
                                .clicked()
                            {
                                actions.push(CardAction::MovePriority(game.id.clone(), 1));
                            }
                        }

                        // Priority Star Button
                        let (star_rect, star_resp) =
                            ui.allocate_exact_size(Vec2::new(26.0, 24.0), Sense::click());
                        let star_bg = if game.priority {
                            Color32::from_rgb(72, 54, 18)
                        } else if star_resp.hovered() {
                            Color32::from_rgb(44, 50, 64)
                        } else {
                            Color32::from_rgb(30, 35, 46)
                        };
                        ui.painter().rect_filled(star_rect, Rounding::same(6.0), star_bg);
                        paint_star(
                            ui.painter(),
                            star_rect.center(),
                            8.0,
                            if game.priority {
                                Color32::from_rgb(245, 185, 66)
                            } else {
                                Color32::from_rgb(115, 126, 148)
                            },
                            game.priority,
                        );
                        if star_resp
                            .on_hover_text(if game.priority {
                                "Remove from Priority Queue"
                            } else {
                                "Pin to Priority Queue"
                            })
                            .clicked()
                        {
                            actions.push(CardAction::TogglePriority(game.id.clone()));
                        }

                        // Primary Status Action Button
                        if is_played {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Move to To-Do").size(12.5).strong(),
                                    )
                                    .fill(Color32::from_rgb(30, 41, 59)),
                                )
                                .clicked()
                            {
                                actions.push(CardAction::SetStatus(game.id.clone(), Status::Todo));
                            }
                        } else if is_dropped {
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("Restore").size(12.5).strong())
                                        .fill(Color32::from_rgb(30, 41, 59)),
                                )
                                .clicked()
                            {
                                actions.push(CardAction::SetStatus(game.id.clone(), Status::Todo));
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Mark Played")
                                            .size(12.5)
                                            .strong()
                                            .color(Color32::WHITE),
                                    )
                                    .fill(Color32::from_rgb(16, 145, 100)),
                                )
                                .clicked()
                            {
                                actions
                                    .push(CardAction::SetStatus(game.id.clone(), Status::Played));
                            }
                        } else {
                            if ui.button("Drop").clicked() {
                                actions
                                    .push(CardAction::SetStatus(game.id.clone(), Status::Dropped));
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Mark Played")
                                            .size(12.5)
                                            .strong()
                                            .color(Color32::WHITE),
                                    )
                                    .fill(Color32::from_rgb(16, 145, 100)),
                                )
                                .clicked()
                            {
                                actions
                                    .push(CardAction::SetStatus(game.id.clone(), Status::Played));
                            }
                        }

                        if game.id.starts_with("custom-")
                            && ui
                                .button("Del")
                                .on_hover_text("Delete custom game")
                                .clicked()
                        {
                            actions.push(CardAction::DeleteCustom(game.id.clone()));
                        }
                    });

                    ui.add_space(4.0);

                    // Lookup / Browser Link Row
                    ui.horizontal(|ui| {
                        let primary_label = if game.direct_url.is_some() {
                            "Open Page ->"
                        } else if game.platform.contains("Steam") {
                            "Find on Steam ->"
                        } else if game.platform.contains("itch.io")
                            && !game.platform.contains("Game Jolt")
                        {
                            "Find on itch.io ->"
                        } else {
                            "Find on Game Jolt ->"
                        };

                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(primary_label)
                                        .size(12.0)
                                        .strong()
                                        .color(Color32::from_rgb(191, 219, 254)),
                                )
                                .fill(Color32::from_rgb(30, 50, 82))
                                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(59, 130, 246))),
                            )
                            .on_hover_text(game.lookup_url())
                            .clicked()
                        {
                            actions.push(CardAction::OpenUrl(game.lookup_url()));
                        }

                        if ui
                            .add(egui::Button::new(RichText::new("GJ").size(12.0)))
                            .on_hover_text("Search on Game Jolt")
                            .clicked()
                        {
                            actions.push(CardAction::OpenUrl(game.search_url_for("gj")));
                        }
                        if ui
                            .add(egui::Button::new(RichText::new("itch").size(12.0)))
                            .on_hover_text("Search on itch.io")
                            .clicked()
                        {
                            actions.push(CardAction::OpenUrl(game.search_url_for("itch")));
                        }
                        if ui
                            .add(egui::Button::new(RichText::new("YT").size(12.0)))
                            .on_hover_text("Search playthroughs on YouTube")
                            .clicked()
                        {
                            actions.push(CardAction::OpenUrl(game.search_url_for("youtube")));
                        }
                    });
                });
            });
        });
    });

    // Paint thick colored accent stripe on left edge of card
    let r = frame_resp.response.rect;
    let stripe_rect = Rect::from_min_max(r.left_top(), Pos2::new(r.left() + 5.5, r.bottom()));
    ui.painter().rect_filled(
        stripe_rect,
        Rounding {
            nw: 10.0,
            sw: 10.0,
            ne: 0.0,
            se: 0.0,
        },
        stripe_col,
    );
}

fn draw_pill(ui: &mut egui::Ui, text: &str, fill: Color32, text_color: Color32) {
    egui::Frame {
        fill,
        rounding: Rounding::same(6.0),
        inner_margin: Margin::symmetric(7.0, 2.0),
        ..Default::default()
    }
    .show(ui, |ui| {
        ui.label(RichText::new(text).size(12.0).strong().color(text_color));
    });
}

fn draw_outline_pill(ui: &mut egui::Ui, text: &str, border_color: Color32) {
    egui::Frame {
        fill: border_color.linear_multiply(0.14),
        stroke: Stroke::new(1.0_f32, border_color.linear_multiply(0.55)),
        rounding: Rounding::same(6.0),
        inner_margin: Margin::symmetric(7.0, 1.5),
        ..Default::default()
    }
    .show(ui, |ui| {
        ui.label(
            RichText::new(text)
                .size(12.0)
                .color(Color32::from_rgb(218, 224, 236)),
        );
    });
}

fn paint_status_circle(painter: &egui::Painter, center: Pos2, status: Status, hovered: bool) {
    let radius = 11.5;
    match status {
        Status::Played => {
            painter.circle_filled(center, radius, Color32::from_rgb(16, 185, 129));
            let p1 = Pos2::new(center.x - 5.0, center.y + 0.5);
            let p2 = Pos2::new(center.x - 1.2, center.y + 4.3);
            let p3 = Pos2::new(center.x + 5.5, center.y - 3.8);
            painter.line_segment([p1, p2], Stroke::new(2.5_f32, Color32::WHITE));
            painter.line_segment([p2, p3], Stroke::new(2.5_f32, Color32::WHITE));
        }
        Status::Dropped => {
            painter.circle_filled(center, radius, Color32::from_rgba_unmultiplied(225, 29, 72, 45));
            painter.circle_stroke(center, radius, Stroke::new(1.8_f32, Color32::from_rgb(225, 29, 72)));
            let d = 4.5;
            painter.line_segment(
                [
                    Pos2::new(center.x - d, center.y - d),
                    Pos2::new(center.x + d, center.y + d),
                ],
                Stroke::new(2.0_f32, Color32::from_rgb(251, 113, 133)),
            );
            painter.line_segment(
                [
                    Pos2::new(center.x + d, center.y - d),
                    Pos2::new(center.x - d, center.y + d),
                ],
                Stroke::new(2.0_f32, Color32::from_rgb(251, 113, 133)),
            );
        }
        Status::Todo => {
            let fill = if hovered {
                Color32::from_rgba_unmultiplied(16, 185, 129, 38)
            } else {
                Color32::from_rgb(20, 24, 32)
            };
            let stroke = if hovered {
                Color32::from_rgb(16, 185, 129)
            } else {
                Color32::from_rgb(110, 122, 144)
            };
            painter.circle_filled(center, radius, fill);
            painter.circle_stroke(center, radius, Stroke::new(2.0_f32, stroke));
            if hovered {
                let p1 = Pos2::new(center.x - 4.5, center.y + 0.5);
                let p2 = Pos2::new(center.x - 1.0, center.y + 3.8);
                let p3 = Pos2::new(center.x + 4.8, center.y - 3.2);
                painter.line_segment([p1, p2], Stroke::new(2.0_f32, Color32::from_rgb(52, 211, 153)));
                painter.line_segment([p2, p3], Stroke::new(2.0_f32, Color32::from_rgb(52, 211, 153)));
            }
        }
    }
}

fn paint_star(painter: &egui::Painter, center: Pos2, outer_r: f32, color: Color32, filled: bool) {
    let inner_r = outer_r * 0.44;
    let mut pts = [Pos2::ZERO; 10];
    for (i, pt) in pts.iter_mut().enumerate() {
        let angle = -std::f32::consts::FRAC_PI_2 + (i as f32) * (std::f32::consts::PI / 5.0);
        let r = if i % 2 == 0 { outer_r } else { inner_r };
        *pt = Pos2::new(center.x + r * angle.cos(), center.y + r * angle.sin());
    }
    if filled {
        for i in 0..5 {
            let tip = pts[i * 2];
            let v_right = pts[(i * 2 + 1) % 10];
            let v_left = pts[(i * 2 + 9) % 10];
            painter.add(egui::Shape::convex_polygon(
                vec![center, v_left, tip, v_right],
                color,
                Stroke::NONE,
            ));
        }
    } else {
        let mut outline = pts.to_vec();
        outline.push(pts[0]);
        painter.add(egui::Shape::line(outline, Stroke::new(1.5_f32, color)));
    }
}

fn state_file_path() -> PathBuf {
    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        exe.push("fnaf_backlog_state.json");
        return exe;
    }
    PathBuf::from("fnaf_backlog_state.json")
}

fn load_state() -> Option<PersistedState> {
    let path = state_file_path();
    let raw = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&raw).ok()
}

#[cfg(windows)]
fn open_url(url: &str) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

#[cfg(not(windows))]
fn open_url(url: &str) {
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1160.0, 820.0])
            .with_min_inner_size([780.0, 560.0])
            .with_title("FNAF Fangame Priority & Backlog Tracker"),
        ..Default::default()
    };

    eframe::run_native(
        "FNAF Fangame Priority & Backlog Tracker",
        options,
        Box::new(|cc| Ok(Box::new(BacklogApp::new(cc)))),
    )
}
