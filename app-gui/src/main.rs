#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;
use eframe::egui::{self, pos2, vec2, Align, Align2, Color32, FontId, Layout, Margin, Rect, RichText, Rounding, Sense, Stroke};
use radio_api::{RadioBrowserClient, SearchFilters, Station};
use radio_core::RadioAppCore;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use radio_player as radio_player_crate;
use radio_tray::TrayHotkeyManager;

#[derive(Parser, Debug)]
#[command(name = "ham-radio-gui", about = "Ham Radio - internet radio player")]
struct Args {
    #[arg(long)]
    country: Option<String>,
    #[arg(long)]
    genre: Option<String>,
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

const COUNTRIES: &[(&str, &str)] = &[
    ("NL", "Netherlands"), ("TR", "Turkey"), ("US", "United States"), ("GB", "United Kingdom"),
    ("DE", "Germany"), ("FR", "France"), ("IT", "Italy"), ("ES", "Spain"),
    ("BE", "Belgium"), ("SE", "Sweden"), ("NO", "Norway"), ("DK", "Denmark"),
    ("PL", "Poland"), ("GR", "Greece"), ("PT", "Portugal"), ("CH", "Switzerland"),
    ("AT", "Austria"), ("IE", "Ireland"), ("CA", "Canada"), ("AU", "Australia"),
    ("BR", "Brazil"), ("JP", "Japan"), ("IN", "India"), ("RU", "Russia"),
    ("UA", "Ukraine"), ("CZ", "Czech Republic"), ("HU", "Hungary"), ("RO", "Romania"),
    ("FI", "Finland"), ("NZ", "New Zealand"), ("MX", "Mexico"), ("AR", "Argentina"),
    ("KR", "South Korea"), ("CN", "China"), ("ZA", "South Africa"), ("IL", "Israel"),
];

const GENRES: &[&str] = &[
    "rock", "classical", "jazz", "pop", "electronic", "hiphop", "dance", "folk",
    "country", "blues", "metal", "reggae", "talk", "news", "sports", "oldies",
    "80s", "90s", "ambient", "chillout", "house", "techno", "trance", "disco",
    "soul", "funk", "rap", "indie", "alternative", "punk", "latin", "world",
];

// ------------------------------------------------------------
// Theme
// ------------------------------------------------------------
mod theme {
    use eframe::egui::Color32;
    pub const BG: Color32 = Color32::from_rgb(17, 18, 21);
    pub const PANEL: Color32 = Color32::from_rgb(22, 24, 28);
    pub const CARD: Color32 = Color32::from_rgb(30, 33, 38);
    pub const CARD_HI: Color32 = Color32::from_rgb(41, 45, 52);
    pub const STROKE: Color32 = Color32::from_rgb(50, 55, 63);
    pub const TEXT: Color32 = Color32::from_rgb(226, 229, 234);
    pub const MUTED: Color32 = Color32::from_rgb(137, 144, 155);
    pub const DIM: Color32 = Color32::from_rgb(80, 86, 96);
    pub const ACCENT: Color32 = Color32::from_rgb(240, 178, 68);
    pub const GREEN: Color32 = Color32::from_rgb(104, 204, 132);
    pub const RED: Color32 = Color32::from_rgb(232, 96, 96);
    pub const INK: Color32 = Color32::from_rgb(28, 22, 10);
}
use theme::*;

fn apply_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();

    style.spacing.item_spacing = vec2(8.0, 6.0);
    style.spacing.button_padding = vec2(12.0, 6.0);
    style.spacing.interact_size.y = 28.0;
    style.spacing.slider_width = 150.0;

    use egui::TextStyle::*;
    style.text_styles.insert(Heading, FontId::proportional(22.0));
    style.text_styles.insert(Body, FontId::proportional(14.0));
    style.text_styles.insert(Button, FontId::proportional(14.0));
    style.text_styles.insert(Small, FontId::proportional(11.0));
    style.text_styles.insert(Monospace, FontId::monospace(13.0));

    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.extreme_bg_color = PANEL; // text edit background
    v.faint_bg_color = CARD;
    v.window_stroke = Stroke::new(1.0, STROKE);
    v.window_rounding = Rounding::same(10.0);
    v.menu_rounding = Rounding::same(8.0);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(240, 178, 68, 80);
    v.selection.stroke = Stroke::new(1.0, ACCENT);

    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, STROKE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);

    v.widgets.inactive.bg_fill = CARD_HI;
    v.widgets.inactive.weak_bg_fill = CARD_HI;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, STROKE);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);

    v.widgets.hovered.bg_fill = Color32::from_rgb(54, 59, 68);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(54, 59, 68);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(84, 91, 104));
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    v.widgets.active.bg_fill = Color32::from_rgb(64, 70, 81);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(64, 70, 81);
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    v.widgets.open.bg_fill = CARD_HI;
    v.widgets.open.weak_bg_fill = CARD_HI;

    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = Rounding::same(6.0);
    }

    ctx.set_style(style);
}

// ------------------------------------------------------------
// Small UI helpers
// ------------------------------------------------------------
fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0, STROKE))
        .rounding(10.0)
        .inner_margin(Margin::symmetric(16.0, 14.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 28))
        .stroke(Stroke::new(1.0, Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 110)))
        .rounding(10.0)
        .inner_margin(Margin::symmetric(9.0, 2.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(10.5).strong().color(color));
        });
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(MUTED));
}

/// Cut `text` with an ellipsis so it fits in `max_w` pixels.
fn fit_text(ui: &egui::Ui, text: &str, font: &FontId, max_w: f32) -> String {
    ui.fonts(|f| {
        let total: f32 = text.chars().map(|c| f.glyph_width(font, c)).sum();
        if total <= max_w {
            return text.to_string();
        }
        let ell = f.glyph_width(font, '…');
        let mut w = 0.0;
        let mut out = String::new();
        for c in text.chars() {
            let cw = f.glyph_width(font, c);
            if w + cw + ell > max_w {
                break;
            }
            w += cw;
            out.push(c);
        }
        out.push('…');
        out
    })
}

fn icon_button(ui: &mut egui::Ui, glyph: &str, tip: &str, color: Color32, enabled: bool) -> bool {
    let btn = egui::Button::new(RichText::new(glyph).size(16.0).color(color)).min_size(vec2(40.0, 36.0));
    ui.add_enabled(enabled, btn).on_hover_text(tip).clicked()
}

/// Segmented LED-style level meter with peak marker and S-meter scale.
fn draw_meter(ui: &mut egui::Ui, level: f32, peak: f32) {
    let w = ui.available_width();
    let h = 16.0;
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let n = 48usize;
    let gap = 2.0;
    let seg_w = ((w - gap * (n as f32 - 1.0)) / n as f32).max(1.0);
    let lit = (level.clamp(0.0, 1.0) * n as f32).round() as usize;
    let peak_i = if peak > 0.0 { Some(((peak.clamp(0.0, 1.0) * n as f32).ceil() as usize).clamp(1, n) - 1) } else { None };
    let painter = ui.painter();
    for i in 0..n {
        let frac = i as f32 / n as f32;
        let base = if frac < 0.60 { GREEN } else if frac < 0.85 { ACCENT } else { RED };
        let color = if Some(i) == peak_i {
            Color32::from_rgb(245, 245, 245)
        } else if i < lit {
            base
        } else {
            Color32::from_rgb(36, 39, 45)
        };
        let x = rect.left() + i as f32 * (seg_w + gap);
        painter.rect_filled(Rect::from_min_size(pos2(x, rect.top()), vec2(seg_w, h)), 1.5, color);
    }

    let (scale, _) = ui.allocate_exact_size(vec2(w, 13.0), Sense::hover());
    let ticks = ["S1", "S3", "S5", "S7", "S9", "+20", "+40", "+60"];
    let painter = ui.painter();
    for (i, t) in ticks.iter().enumerate() {
        let x = scale.left() + w * (i as f32 / (ticks.len() as f32 - 1.0));
        let anchor = if i == 0 { Align2::LEFT_TOP } else if i == ticks.len() - 1 { Align2::RIGHT_TOP } else { Align2::CENTER_TOP };
        painter.text(pos2(x, scale.top() + 1.0), anchor, *t, FontId::monospace(9.5), DIM);
    }
}

// ------------------------------------------------------------
// Favorites order (GUI-side; radio_core keeps no order of its own)
// ------------------------------------------------------------
fn order_file() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    let dir = base.join("ham-radio");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("favorites_order.txt"))
}

fn load_fav_order() -> Vec<String> {
    order_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| t.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

/// Stable sort: known ids in saved order, unknown (new) favorites keep their order at the end.
fn apply_order(favs: &mut [Station], order: &[String]) {
    if order.is_empty() {
        return;
    }
    favs.sort_by_key(|s| order.iter().position(|u| *u == s.stationuuid).unwrap_or(usize::MAX));
}

enum RowAction {
    None,
    Play,
    ToggleFav,
}

struct RowOut {
    action: RowAction,
    rect: Rect,
    drag_started: bool,
}

/// One station row. With `handle` set, a grip on the left lets the user drag it (favorites tab).
fn station_row(ui: &mut egui::Ui, s: &Station, current: bool, on_air: bool, fav: bool, handle: bool, dragged: bool) -> RowOut {
    let row_h = 44.0;
    let star_w = 40.0;
    let handle_w = if handle { 24.0 } else { 0.0 };
    let total_w = ui.available_width();
    let mut out = RowOut { action: RowAction::None, rect: Rect::NOTHING, drag_started: false };

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let handle_part = if handle { Some(ui.allocate_exact_size(vec2(handle_w, row_h), Sense::drag())) } else { None };
        let (main, main_resp) = ui.allocate_exact_size(vec2((total_w - star_w - handle_w).max(10.0), row_h), Sense::click());
        let (star, star_resp) = ui.allocate_exact_size(vec2(star_w, row_h), Sense::click());
        let main_resp = main_resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        let star_resp = star_resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(if fav { "Remove from favorites" } else { "Add to favorites" });

        let mut row = main.union(star);
        let mut handle_hover = false;
        let mut handle_rect: Option<Rect> = None;
        if let Some((hr, hresp)) = handle_part {
            row = row.union(hr);
            handle_rect = Some(hr);
            let hresp = hresp.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text("Drag to reorder");
            handle_hover = hresp.hovered();
            if hresp.drag_started() {
                out.drag_started = true;
            }
        }
        out.rect = row;
        let hovered = main_resp.hovered() || star_resp.hovered() || handle_hover;

        // text prep (needs &Ui for font metrics)
        let base_x = main.left();
        let text_x = base_x + 30.0;
        let badge_w = 46.0;
        let max_w = (star.left() - badge_w - 16.0 - text_x).max(40.0);
        let name_font = FontId::proportional(14.5);
        let meta_font = FontId::proportional(11.5);
        let name = fit_text(ui, &s.name, &name_font, max_w);

        let mut meta: Vec<String> = Vec::new();
        let br = s.bitrate.to_string();
        if !br.is_empty() && br != "0" {
            meta.push(format!("{} kbps", br));
        }
        if !s.country.is_empty() {
            meta.push(s.country.clone());
        }
        if !s.tags.is_empty() {
            meta.push(s.tags.replace(',', ", "));
        }
        let meta = fit_text(ui, &meta.join("  ·  "), &meta_font, max_w);

        let codec = s.codec.to_uppercase();
        let codec: String = if codec.is_empty() { "—".to_string() } else { codec.chars().take(5).collect() };
        let badge_color = match codec.as_str() {
            "AAC" | "AAC+" => ACCENT,
            "MP3" => Color32::from_rgb(122, 170, 230),
            _ => MUTED,
        };

        if dragged {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }

        let painter = ui.painter();
        let bg = if dragged {
            Color32::from_rgba_unmultiplied(240, 178, 68, 46)
        } else if current {
            Color32::from_rgba_unmultiplied(240, 178, 68, 22)
        } else if hovered {
            CARD
        } else {
            Color32::TRANSPARENT
        };
        painter.rect_filled(row, 6.0, bg);
        if current {
            painter.rect_filled(
                Rect::from_min_size(pos2(base_x, row.top() + 8.0), vec2(3.0, row_h - 16.0)),
                1.5,
                if on_air { GREEN } else { ACCENT },
            );
        }
        if let Some(hr) = handle_rect {
            let c = hr.center();
            let col = if dragged || handle_hover { MUTED } else { DIM };
            for dx in [-3.0_f32, 3.0] {
                for dy in [-6.0_f32, 0.0, 6.0] {
                    painter.circle_filled(pos2(c.x + dx, c.y + dy), 1.4, col);
                }
            }
        }
        if on_air {
            painter.text(pos2(base_x + 16.0, row.center().y), Align2::CENTER_CENTER, "▶", FontId::proportional(11.0), GREEN);
        }
        painter.text(
            pos2(text_x, row.top() + 15.0),
            Align2::LEFT_CENTER,
            name,
            name_font,
            if current { Color32::WHITE } else { TEXT },
        );
        painter.text(pos2(text_x, row.top() + 32.0), Align2::LEFT_CENTER, meta, meta_font, MUTED);

        // codec badge
        let badge = Rect::from_center_size(pos2(star.left() - badge_w / 2.0 - 8.0, row.center().y), vec2(badge_w, 18.0));
        painter.rect_filled(badge, 4.0, Color32::from_rgba_unmultiplied(badge_color.r(), badge_color.g(), badge_color.b(), 30));
        painter.text(badge.center(), Align2::CENTER_CENTER, codec, FontId::monospace(10.0), badge_color);

        // star
        let star_color = if fav { ACCENT } else if star_resp.hovered() { MUTED } else { DIM };
        painter.text(star.center(), Align2::CENTER_CENTER, "★", FontId::proportional(17.0), star_color);

        if star_resp.clicked() {
            out.action = RowAction::ToggleFav;
        } else if main_resp.clicked() {
            out.action = RowAction::Play;
        }
    });
    out
}

// ------------------------------------------------------------
// App
// ------------------------------------------------------------
enum PlayMsg {
    ProbedOk(Station),
    ProbedFailed(String),
}

#[derive(Clone, Copy, PartialEq)]
enum StatusKind {
    Info,
    Ok,
    Error,
}

struct HamRadioApp {
    core: RadioAppCore,
    stations: Vec<Station>,
    favorites: Vec<Station>,
    fav_order: Vec<String>,
    fav_drag: Option<usize>,
    limit: usize,
    last_station: Option<Station>,
    volume: f32,
    search_country: String,
    search_genre: String,
    search_name: String,
    status: String,
    status_kind: StatusKind,
    vu_level: f32,
    vu_peak: f32,
    search_rx: Option<mpsc::Receiver<Vec<Station>>>,
    play_rx: Option<mpsc::Receiver<PlayMsg>>,
    pending_since: Option<Instant>,
    pending_name: String,
    is_playing: bool,
    show_favorites: bool,
    hotkey_manager: Option<TrayHotkeyManager>,
    country_filter: String,
    genre_filter: String,
    show_name_dropdown: bool,
    current_song: String,
    song_rx: Option<mpsc::Receiver<String>>,
    song_cancel: Option<Arc<AtomicBool>>,
    recording_file: String,
    last_recorded_song: String,
}

impl HamRadioApp {
    fn new(cc: &eframe::CreationContext<'_>, args: Args) -> Self {
        apply_theme(&cc.egui_ctx);
        let core = RadioAppCore::new().expect("core");
        let fav_order = load_fav_order();
        let mut favorites = core.list_favorites().unwrap_or_default();
        apply_order(&mut favorites, &fav_order);
        let hotkey_manager = match TrayHotkeyManager::new() {
            Ok(m) => {
                println!("[gui] Hotkeys active: Ctrl+Alt+P play/pause, N next, B prev");
                Some(m)
            }
            Err(e) => {
                println!("[gui] Hotkey init failed (optional): {}", e);
                None
            }
        };
        let initial_country = args.country.unwrap_or_else(|| "NL".to_string());
        let initial_genre = args.genre.unwrap_or_else(|| "classical".to_string());
        Self {
            core,
            stations: vec![],
            favorites,
            fav_order,
            fav_drag: None,
            limit: args.limit,
            last_station: None,
            volume: 0.7,
            search_country: initial_country,
            search_genre: initial_genre,
            search_name: String::new(),
            status: "Ready. Press Tune, or search a station by name.".to_string(),
            status_kind: StatusKind::Info,
            vu_level: 0.0,
            vu_peak: 0.0,
            search_rx: None,
            play_rx: None,
            pending_since: None,
            pending_name: String::new(),
            is_playing: false,
            show_favorites: false,
            hotkey_manager,
            country_filter: String::new(),
            genre_filter: String::new(),
            show_name_dropdown: false,
            current_song: String::new(),
            song_rx: None,
            song_cancel: None,
            recording_file: String::new(),
            last_recorded_song: String::new(),
        }
    }

    fn say(&mut self, kind: StatusKind, msg: impl Into<String>) {
        self.status = msg.into();
        self.status_kind = kind;
    }

    fn format_country_display(code: &str) -> String {
        for (c, name) in COUNTRIES {
            if c.eq_ignore_ascii_case(code) {
                return format!("{} - {}", name, c);
            }
        }
        if code.len() == 2 {
            format!("{} - {}", code.to_uppercase(), code.to_uppercase())
        } else {
            code.to_string()
        }
    }

    // ---------------- playback ----------------

    fn stop_song_poller(&mut self) {
        if let Some(flag) = self.song_cancel.take() {
            flag.store(true, Ordering::Relaxed);
        }
        self.song_rx = None;
        self.current_song = String::new();
    }

    fn play_station_nonblocking(&mut self, station: Station) {
        self.core.stop();
        self.is_playing = false;
        self.pending_name = station.name.clone();
        self.pending_since = Some(Instant::now());
        self.say(StatusKind::Info, format!("Connecting to {}…", station.name));
        self.last_station = Some(station.clone());
        self.show_name_dropdown = false;

        // ICY "now playing" poller for this station; the previous one is cancelled.
        self.stop_song_poller();
        let stream_url = if !station.url_resolved.is_empty() { station.url_resolved.clone() } else { station.url.clone() };
        let (song_tx, song_rx) = mpsc::channel();
        self.song_rx = Some(song_rx);
        let cancel = Arc::new(AtomicBool::new(false));
        self.song_cancel = Some(cancel.clone());
        let poll_url = stream_url.clone();
        std::thread::spawn(move || loop {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if let Some(title) = radio_player_crate::fetch_icy_title(&poll_url) {
                if song_tx.send(title).is_err() {
                    break;
                }
            }
            std::thread::sleep(Duration::from_secs(5));
        });

        // Probe off the GUI thread; the result comes back over a channel.
        let (tx, rx) = mpsc::channel();
        self.play_rx = Some(rx);
        std::thread::spawn(move || match radio_player_crate::probe_url(&stream_url) {
            Ok(_) => {
                let _ = tx.send(PlayMsg::ProbedOk(station));
            }
            Err(pe) => {
                let msg = pe.msg().to_string();
                let generic = if msg.contains("offline") || msg.contains("reached") {
                    format!("{} can't be reached. It may be offline. Try another station.", station.name)
                } else {
                    format!("{} can't be played. Try another station.", station.name)
                };
                let _ = tx.send(PlayMsg::ProbedFailed(generic));
            }
        });
    }

    fn play_station_direct(&mut self, station: Station) {
        self.core.stop();
        match self.core.play_station(station) {
            Ok(_) => {
                let name = self.core.current_station_name();
                self.say(StatusKind::Ok, format!("Playing {}", name));
                self.is_playing = true;
            }
            Err(_) => {
                self.say(StatusKind::Error, "This station can't be played. Try another station.");
                self.is_playing = false;
            }
        }
    }

    fn toggle_play_stop(&mut self) {
        if self.is_playing || self.pending_since.is_some() {
            self.core.stop();
            self.is_playing = false;
            self.pending_since = None;
            self.play_rx = None;
            self.stop_song_poller();
            self.say(StatusKind::Info, "Stopped");
        } else if let Some(last) = self.last_station.clone() {
            self.play_station_nonblocking(last);
        } else if let Some(first) = self.stations.first().cloned() {
            self.play_station_nonblocking(first);
        } else {
            self.say(StatusKind::Info, "No station selected. Tune or search first.");
        }
    }

    /// Step through favorites relative to the station that is currently selected.
    /// Done here (not in radio_core) so the position always follows what is playing,
    /// and so the poller, highlight and star stay in sync.
    fn play_favorite_step(&mut self, next: bool) {
        let n = self.favorites.len();
        if n == 0 {
            self.say(StatusKind::Info, "No favorites yet. Click ★ on a station to add one.");
            return;
        }
        let cur = self
            .last_station
            .as_ref()
            .and_then(|s| self.favorites.iter().position(|f| f.stationuuid == s.stationuuid));
        let idx = match (cur, next) {
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
            (None, true) => 0,
            (None, false) => n - 1,
        };
        let station = self.favorites[idx].clone();
        self.play_station_nonblocking(station);
    }

    fn change_volume(&mut self, delta: f32) {
        self.volume = (self.volume + delta).clamp(0.0, 1.0);
        self.core.set_volume(self.volume);
    }

    // ---------------- recording ----------------

    fn sanitize_filename(s: &str) -> String {
        let cleaned: String = s
            .chars()
            .map(|c| match c {
                '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
                c if (c as u32) < 32 => '-',
                c => c,
            })
            .collect();
        // limit by characters (byte truncation panics on non-ASCII names)
        let mut out: String = cleaned.chars().take(80).collect();
        while out.ends_with('.') || out.ends_with(' ') {
            out.pop();
        }
        out.trim().to_string()
    }

    fn recording_filename(&self) -> String {
        let station = self.core.current_station_name();
        let base = if !self.current_song.is_empty() {
            Self::sanitize_filename(&self.current_song)
        } else if !station.is_empty() && station != "No station" && station != "Stopped" {
            Self::sanitize_filename(&station)
        } else {
            String::new()
        };
        let base = if base.is_empty() { "ham_recording".to_string() } else { base };
        let ext = match &self.last_station {
            Some(s) if s.codec.to_lowercase().contains("aac") => "aac",
            _ => "mp3",
        };
        format!("{}.{}", base, ext)
    }

    fn start_recording_named(&mut self) {
        let fname = self.recording_filename();
        match self.core.start_recording(std::path::PathBuf::from(fname.clone())) {
            Ok(_) => {
                self.recording_file = fname.clone();
                self.last_recorded_song = self.current_song.clone();
                self.say(StatusKind::Ok, format!("Recording to {}", fname));
            }
            Err(e) => self.say(StatusKind::Error, format!("Recording failed: {}", e)),
        }
    }

    fn stop_recording_named(&mut self) {
        self.core.stop_recording();
        let f = std::mem::take(&mut self.recording_file);
        self.last_recorded_song = String::new();
        self.say(StatusKind::Info, format!("Recording saved: {}", f));
    }

    // ---------------- favorites ----------------

    fn refresh_favorites(&mut self) {
        let mut favs = self.core.list_favorites().unwrap_or_default();
        apply_order(&mut favs, &self.fav_order);
        self.favorites = favs;
    }

    fn save_fav_order(&mut self) {
        self.fav_order = self.favorites.iter().map(|f| f.stationuuid.clone()).collect();
        if let Some(path) = order_file() {
            let _ = std::fs::write(path, self.fav_order.join("\n"));
        }
    }

    /// Move favorite `src` so it sits before index `target` (target is a gap index 0..=len).
    fn move_favorite(&mut self, src: usize, target: usize) {
        let n = self.favorites.len();
        if src >= n {
            return;
        }
        let mut t = target.min(n);
        if t == src || t == src + 1 {
            return;
        }
        let item = self.favorites.remove(src);
        if t > src {
            t -= 1;
        }
        self.favorites.insert(t, item);
        self.save_fav_order();
    }

    fn is_favorite(&self, s: &Station) -> bool {
        self.favorites.iter().any(|f| f.stationuuid == s.stationuuid)
    }

    fn toggle_favorite(&mut self, s: &Station) {
        if self.is_favorite(s) {
            self.remove_favorite_by_uuid(&s.stationuuid, &s.name);
        } else {
            match self.core.add_favorite(s) {
                Ok(_) => {
                    self.say(StatusKind::Ok, format!("Added {} to favorites", s.name));
                    self.refresh_favorites();
                }
                Err(e) => self.say(StatusKind::Error, format!("Couldn't add favorite: {}", e)),
            }
        }
    }

    fn remove_favorite_by_uuid(&mut self, uuid: &str, name: &str) {
        match self.core.remove_favorite(uuid) {
            Ok(_) => {
                self.say(StatusKind::Info, format!("Removed {} from favorites", name));
                self.refresh_favorites();
            }
            Err(e) => self.say(StatusKind::Error, format!("Couldn't remove favorite: {}", e)),
        }
    }

    // ---------------- search ----------------

    /// Tune: Country + Genre only (ignores the name field).
    fn search_nonblocking(&mut self) {
        let country_input = self.search_country.trim().to_string();
        let genre = self.search_genre.trim().to_string();
        let country = if country_input.contains(" - ") {
            country_input.split(" - ").last().unwrap_or(&country_input).trim().to_string()
        } else {
            let mut found = country_input.clone();
            for (code, name) in COUNTRIES {
                if name.eq_ignore_ascii_case(&country_input) {
                    found = code.to_string();
                    break;
                }
            }
            found
        };
        let limit = self.limit;
        let (tx, rx) = mpsc::channel();
        self.search_rx = Some(rx);
        self.say(StatusKind::Info, format!("Tuning {} / {}…", Self::format_country_display(&country), genre));
        self.show_favorites = false;
        self.show_name_dropdown = false;
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            let result = rt.block_on(async {
                let client = RadioBrowserClient::new();
                let filters = SearchFilters {
                    countrycode: if country.is_empty() { None } else { Some(country) },
                    genre: if genre.is_empty() { None } else { Some(genre) },
                    limit,
                    ..Default::default()
                };
                client.search_filtered(filters).await.unwrap_or_default()
            });
            let _ = tx.send(result);
        });
    }

    /// Global name search: ignores Country and Genre.
    fn search_name_only(&mut self) {
        let name_query = self.search_name.trim().to_string();
        if name_query.len() < 2 {
            return;
        }
        let limit = self.limit;
        let (tx, rx) = mpsc::channel();
        self.search_rx = Some(rx);
        self.say(StatusKind::Info, format!("Searching for “{}”…", name_query));
        self.show_favorites = false;
        self.show_name_dropdown = false;
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            let result = rt.block_on(async {
                let client = RadioBrowserClient::new();
                let filters = SearchFilters { query: name_query, limit, ..Default::default() };
                client.search_filtered(filters).await.unwrap_or_default()
            });
            let _ = tx.send(result);
        });
    }

    fn get_name_autocomplete(&self) -> Vec<Station> {
        let query = self.search_name.trim().to_lowercase();
        if query.len() < 2 {
            return vec![];
        }
        let mut matches: Vec<Station> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for s in self.favorites.iter().chain(self.stations.iter()) {
            if matches.len() >= 15 {
                break;
            }
            if seen.contains(&s.stationuuid) {
                continue;
            }
            if s.name.to_lowercase().contains(&query) {
                matches.push(s.clone());
                seen.insert(s.stationuuid.clone());
            }
        }
        matches
    }

    // ---------------- input & background work ----------------

    fn handle_hotkeys(&mut self) {
        let action = self.hotkey_manager.as_ref().and_then(|hm| hm.poll_action());
        if let Some(action) = action {
            match action.as_str() {
                "play_pause" => self.toggle_play_stop(),
                "next" => self.play_favorite_step(true),
                "prev" => self.play_favorite_step(false),
                "vol_up" => self.change_volume(0.05),
                "vol_down" => self.change_volume(-0.05),
                _ => {}
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // Don't steal keys while the user is typing.
        if ctx.wants_keyboard_input() {
            return;
        }
        let (space, up, down) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::ArrowUp) || i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals),
                i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::Minus),
            )
        });
        if space {
            self.toggle_play_stop();
        }
        if up {
            self.change_volume(0.05);
        }
        if down {
            self.change_volume(-0.05);
        }
    }

    fn poll_background(&mut self) {
        // search results
        if let Some(rx) = &self.search_rx {
            if let Ok(stations) = rx.try_recv() {
                let count = stations.len();
                self.stations = stations;
                self.search_rx = None;
                self.show_favorites = false;
                if count == 0 {
                    self.say(StatusKind::Info, "No stations found. Try another country, genre or name.");
                } else {
                    self.say(StatusKind::Ok, format!("Found {} stations. Click one to play.", count));
                }
            }
        }

        // now playing; split the recording when the song changes
        let mut titles: Vec<String> = Vec::new();
        if let Some(rx) = &self.song_rx {
            while let Ok(t) = rx.try_recv() {
                titles.push(t);
            }
        }
        for title in titles {
            if title != self.current_song {
                self.current_song = title.clone();
                if self.core.is_recording() && title != self.last_recorded_song {
                    self.core.stop_recording();
                    let fname = self.recording_filename();
                    if self.core.start_recording(std::path::PathBuf::from(fname.clone())).is_ok() {
                        self.recording_file = fname.clone();
                        self.last_recorded_song = title.clone();
                        self.say(StatusKind::Ok, format!("Recording to {}", fname));
                    }
                }
            }
        }

        // probe result / timeout
        let mut msg: Option<PlayMsg> = None;
        if let Some(rx) = &self.play_rx {
            if let Ok(m) = rx.try_recv() {
                msg = Some(m);
            }
        }
        match msg {
            Some(PlayMsg::ProbedOk(station)) => {
                self.pending_since = None;
                self.play_rx = None;
                self.play_station_direct(station);
            }
            Some(PlayMsg::ProbedFailed(err)) => {
                self.pending_since = None;
                self.play_rx = None;
                self.is_playing = false;
                self.say(StatusKind::Error, err);
            }
            None => {
                if let Some(since) = self.pending_since {
                    if since.elapsed() > Duration::from_secs(8) {
                        let name = self.pending_name.clone();
                        self.pending_since = None;
                        self.play_rx = None;
                        self.is_playing = false;
                        self.say(StatusKind::Error, format!("{} can't be played. Try another station.", name));
                    }
                }
            }
        }
    }

    // ---------------- UI sections ----------------

    fn ui_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            ui.label(RichText::new("HAM").size(22.0).strong().color(ACCENT));
            ui.label(RichText::new("RADIO").size(22.0).color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.hotkey_manager.is_some() {
                    chip(ui, "HOTKEYS", MUTED);
                }
                if self.core.is_recording() {
                    chip(ui, "● REC", RED);
                }
            });
        });
    }

    fn ui_now_playing(&mut self, ui: &mut egui::Ui) {
        let station = self.core.current_station_name();
        let idle = station.is_empty() || station == "No station" || station == "Stopped";
        let connecting = self.pending_since.is_some();
        let last_name = self.last_station.as_ref().map(|s| s.name.clone());

        let (title, title_color) = if connecting {
            (self.pending_name.clone(), MUTED)
        } else if self.is_playing && !idle {
            (station.clone(), TEXT)
        } else if let Some(n) = last_name {
            (n, MUTED)
        } else {
            ("No station selected".to_string(), DIM)
        };

        let level = self.vu_level;
        let peak = self.vu_peak;

        card(ui, |ui| {
            ui.horizontal(|ui| {
                let (label, color) = if connecting {
                    ("CONNECTING", ACCENT)
                } else if self.is_playing {
                    ("ON AIR", GREEN)
                } else {
                    ("STANDBY", MUTED)
                };
                chip(ui, label, color);
                if connecting {
                    ui.add(egui::Spinner::new().size(14.0).color(ACCENT));
                }
            });

            ui.label(RichText::new(title).size(22.0).strong().color(title_color));

            let (song, song_color) = if !self.is_playing {
                (" ".to_string(), DIM)
            } else if self.current_song.is_empty() {
                ("♪  Waiting for track info…".to_string(), DIM)
            } else {
                (format!("♪  {}", self.current_song), ACCENT)
            };
            ui.label(RichText::new(song).size(14.0).color(song_color));

            ui.add_space(6.0);
            draw_meter(ui, level, peak);

            ui.horizontal(|ui| {
                if self.core.is_recording() && !self.recording_file.is_empty() {
                    ui.label(RichText::new(format!("● {}", self.recording_file)).size(11.0).color(RED));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let s = (level * 9.0) as u32 + 1;
                    let s = if s > 9 { "S9+".to_string() } else { format!("S{}", s) };
                    ui.label(RichText::new(format!("{}  ·  {:.0}%", s, level * 100.0)).size(11.0).monospace().color(MUTED));
                });
            });
        });
    }

    fn ui_transport(&mut self, ui: &mut egui::Ui) {
        let connecting = self.pending_since.is_some();
        let is_rec = self.core.is_recording();
        let current_fav = self.last_station.as_ref().map(|s| self.is_favorite(s)).unwrap_or(false);

        ui.horizontal(|ui| {
            let (label, fill, fg, tip) = if connecting {
                ("✕  Cancel", CARD_HI, TEXT, "Cancel (Space)")
            } else if self.is_playing {
                ("⏹  Stop", Color32::from_rgb(70, 38, 38), RED, "Stop (Space)")
            } else {
                ("▶  Play", ACCENT, INK, "Play (Space)")
            };
            let main_btn = egui::Button::new(RichText::new(label).size(15.0).strong().color(fg))
                .fill(fill)
                .min_size(vec2(124.0, 36.0));
            if ui.add(main_btn).on_hover_text(tip).clicked() {
                self.toggle_play_stop();
            }

            if icon_button(ui, "⏮", "Previous favorite (Ctrl+Alt+B)", TEXT, true) {
                self.play_favorite_step(false);
            }
            if icon_button(ui, "⏭", "Next favorite (Ctrl+Alt+N)", TEXT, true) {
                self.play_favorite_step(true);
            }

            let fav_tip = if current_fav { "Remove current station from favorites" } else { "Add current station to favorites" };
            if icon_button(ui, "★", fav_tip, if current_fav { ACCENT } else { MUTED }, self.last_station.is_some()) {
                if let Some(s) = self.last_station.clone() {
                    self.toggle_favorite(&s);
                }
            }

            let rec_label = if is_rec { "■  Stop rec" } else { "●  Rec" };
            let rec_btn = egui::Button::new(RichText::new(rec_label).color(RED))
                .fill(if is_rec { Color32::from_rgb(70, 38, 38) } else { CARD_HI })
                .min_size(vec2(96.0, 36.0));
            let rec_tip = if is_rec { "Stop recording" } else { "Record the current station" };
            if ui.add_enabled(self.is_playing || is_rec, rec_btn).on_hover_text(rec_tip).clicked() {
                if is_rec {
                    self.stop_recording_named();
                } else {
                    self.start_recording_named();
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(format!("{:>3.0}%", self.volume * 100.0)).monospace().color(MUTED));
                let mut vol = self.volume;
                if ui.add(egui::Slider::new(&mut vol, 0.0..=1.0).show_value(false)).changed() {
                    self.volume = vol;
                    self.core.set_volume(vol);
                }
                field_label(ui, "VOL");
            });
        });
    }

    fn ui_search(&mut self, ui: &mut egui::Ui) {
        let mut play_from_dropdown: Option<Station> = None;

        card(ui, |ui| {
            // --- name search ---
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 96.0).max(120.0);
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.search_name)
                        .hint_text("Search stations worldwide by name…")
                        .desired_width(w),
                );
                if edit.changed() {
                    self.show_name_dropdown = self.search_name.trim().len() >= 2;
                }
                if edit.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.show_name_dropdown = false;
                }
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter {
                    self.search_name_only();
                }
                if ui.add(egui::Button::new("Search").min_size(vec2(84.0, 28.0))).on_hover_text("Search all countries and genres").clicked() {
                    self.search_name_only();
                }
            });

            // --- autocomplete (favorites + current results) ---
            if self.show_name_dropdown && self.search_name.trim().len() >= 2 {
                let matches = self.get_name_autocomplete();
                egui::Frame::none()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, STROKE))
                    .rounding(6.0)
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if matches.is_empty() {
                            ui.label(RichText::new("No local matches. Press Search to look online.").size(12.0).color(MUTED));
                        } else {
                            ui.label(RichText::new(format!("{} matching stations", matches.len())).size(11.0).color(MUTED));
                            egui::ScrollArea::vertical().max_height(130.0).show(ui, |ui| {
                                for s in &matches {
                                    let label = format!("{}   ·   {}   ·   {} kbps {}", s.name, s.country, s.bitrate, s.codec);
                                    if ui.selectable_label(false, label).clicked() {
                                        play_from_dropdown = Some(s.clone());
                                    }
                                }
                            });
                        }
                    });
            }

            ui.add_space(2.0);

            // --- country / genre / tune ---
            ui.horizontal_wrapped(|ui| {
                field_label(ui, "Country");
                ui.add(egui::TextEdit::singleline(&mut self.search_country).hint_text("NL or Netherlands").desired_width(110.0));
                egui::ComboBox::from_id_source("country_dropdown")
                    .selected_text(Self::format_country_display(&self.search_country))
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.country_filter).hint_text("Filter…"));
                        let filter = self.country_filter.to_lowercase();
                        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                            for (code, name) in COUNTRIES {
                                if !filter.is_empty() && !name.to_lowercase().contains(&filter) && !code.to_lowercase().contains(&filter) {
                                    continue;
                                }
                                let selected = self.search_country.eq_ignore_ascii_case(code) || self.search_country.eq_ignore_ascii_case(name);
                                if ui.selectable_label(selected, format!("{} - {}", name, code)).clicked() {
                                    self.search_country = code.to_string();
                                    ui.close_menu();
                                }
                            }
                        });
                    });

                ui.add_space(10.0);
                field_label(ui, "Genre");
                ui.add(egui::TextEdit::singleline(&mut self.search_genre).hint_text("rock, jazz…").desired_width(100.0));
                egui::ComboBox::from_id_source("genre_dropdown")
                    .selected_text(self.search_genre.clone())
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.genre_filter).hint_text("Filter…"));
                        let filter = self.genre_filter.to_lowercase();
                        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                            for genre in GENRES {
                                if !filter.is_empty() && !genre.to_lowercase().contains(&filter) {
                                    continue;
                                }
                                if ui.selectable_label(self.search_genre == *genre, *genre).clicked() {
                                    self.search_genre = genre.to_string();
                                    ui.close_menu();
                                }
                            }
                        });
                    });

                ui.add_space(10.0);
                let tune = egui::Button::new(RichText::new("Tune").strong().color(INK)).fill(ACCENT).min_size(vec2(72.0, 28.0));
                if ui.add(tune).on_hover_text("List stations for this country and genre").clicked() {
                    self.search_nonblocking();
                }
            });
        });

        if let Some(st) = play_from_dropdown {
            self.play_station_nonblocking(st);
            self.search_name = String::new();
            self.show_name_dropdown = false;
        }
    }

    fn ui_list(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let n_res = self.stations.len();
            let n_fav = self.favorites.len();
            if ui.selectable_label(!self.show_favorites, RichText::new(format!("Results  {}", n_res)).strong()).clicked() {
                self.show_favorites = false;
            }
            if ui.selectable_label(self.show_favorites, RichText::new(format!("★ Favorites  {}", n_fav)).strong()).clicked() {
                self.show_favorites = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let hint = if self.show_favorites {
                    "Drag the grip to reorder  ·  ★ removes"
                } else {
                    "Click a row to play  ·  ★ toggles favorite"
                };
                ui.label(RichText::new(hint).size(11.0).color(DIM));
            });
        });
        ui.add_space(2.0);

        if !self.show_favorites {
            self.fav_drag = None;
        }
        let list: Vec<Station> = if self.show_favorites { self.favorites.clone() } else { self.stations.clone() };
        let current_uuid = self.last_station.as_ref().map(|s| s.stationuuid.clone());
        let handle = self.show_favorites;
        let mut play: Option<Station> = None;
        let mut fav: Option<Station> = None;
        let mut drag_start: Option<usize> = None;
        let mut drop_target: Option<usize> = None;

        if list.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                let msg = if self.show_favorites {
                    "No favorites yet. Click ★ on any station to keep it here."
                } else {
                    "No stations yet. Press Tune, or search by name."
                };
                ui.label(RichText::new(msg).color(MUTED));
            });
            return;
        }

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let mut rects: Vec<Rect> = Vec::with_capacity(list.len());
            for (i, s) in list.iter().enumerate() {
                let is_cur = current_uuid.as_deref() == Some(s.stationuuid.as_str());
                let is_fav = self.favorites.iter().any(|f| f.stationuuid == s.stationuuid);
                let dragged = handle && self.fav_drag == Some(i);
                let out = station_row(ui, s, is_cur, is_cur && self.is_playing, is_fav, handle, dragged);
                rects.push(out.rect);
                if out.drag_started {
                    drag_start = Some(i);
                }
                match out.action {
                    RowAction::Play => play = Some(s.clone()),
                    RowAction::ToggleFav => fav = Some(s.clone()),
                    RowAction::None => {}
                }
            }

            // While dragging: auto-scroll near the edges and show where the row will land.
            if handle && (self.fav_drag.is_some() || drag_start.is_some()) {
                if let Some(pos) = ui.input(|i| i.pointer.latest_pos()) {
                    let clip = ui.clip_rect();
                    if pos.y < clip.top() + 28.0 {
                        ui.scroll_with_delta(vec2(0.0, 12.0));
                    } else if pos.y > clip.bottom() - 28.0 {
                        ui.scroll_with_delta(vec2(0.0, -12.0));
                    }
                    let target = rects.iter().filter(|r| r.center().y < pos.y).count();
                    drop_target = Some(target);
                    let y = if target == 0 {
                        rects[0].top() - 1.0
                    } else if target >= rects.len() {
                        rects[rects.len() - 1].bottom() + 1.0
                    } else {
                        (rects[target - 1].bottom() + rects[target].top()) / 2.0
                    };
                    let (x0, x1) = (rects[0].left() + 4.0, rects[0].right() - 4.0);
                    ui.painter().line_segment([pos2(x0, y), pos2(x1, y)], Stroke::new(2.0, ACCENT));
                }
            }
        });

        if let Some(i) = drag_start {
            self.fav_drag = Some(i);
        }
        if let Some(src) = self.fav_drag {
            if ui.input(|i| i.pointer.any_released() || !i.pointer.any_down()) {
                if let Some(t) = drop_target {
                    self.move_favorite(src, t);
                }
                self.fav_drag = None;
            }
        }

        if let Some(s) = fav {
            self.toggle_favorite(&s);
        } else if let Some(s) = play {
            self.play_station_nonblocking(s);
        }
    }

    fn ui_status_bar(&mut self, ui: &mut egui::Ui) {
        let busy = self.search_rx.is_some() || self.pending_since.is_some();
        ui.horizontal(|ui| {
            if busy {
                ui.add(egui::Spinner::new().size(12.0).color(ACCENT));
            } else {
                let color = match self.status_kind {
                    StatusKind::Info => DIM,
                    StatusKind::Ok => GREEN,
                    StatusKind::Error => RED,
                };
                let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, color);
            }
            let text_color = if self.status_kind == StatusKind::Error && !busy { RED } else { MUTED };
            ui.label(RichText::new(&self.status).size(12.5).color(text_color));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new("Space play/stop  ·  ↑↓ or +/- volume  ·  Ctrl+Alt+P global").size(11.0).color(DIM));
            });
        });
    }
}

impl eframe::App for HamRadioApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Faster repaint while audio is running so the meter looks smooth.
        let wait = if self.is_playing { 40 } else { 200 };
        ctx.request_repaint_after(Duration::from_millis(wait));

        self.vu_level = self.core.vu_level();
        self.vu_peak = self.core.vu_peak();
        self.handle_hotkeys();
        self.handle_shortcuts(ctx);
        self.poll_background();

        egui::TopBottomPanel::bottom("status_bar")
            .frame(egui::Frame::none().fill(PANEL).stroke(Stroke::new(1.0, STROKE)).inner_margin(Margin::symmetric(16.0, 7.0)))
            .show(ctx, |ui| self.ui_status_bar(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(Margin::symmetric(16.0, 12.0)))
            .show(ctx, |ui| {
                self.ui_header(ui);
                ui.add_space(10.0);
                self.ui_now_playing(ui);
                ui.add_space(10.0);
                self.ui_transport(ui);
                ui.add_space(12.0);
                self.ui_search(ui);
                ui.add_space(10.0);
                self.ui_list(ui);
            });
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    println!("Starting Ham Radio GUI - Country: {:?} Genre: {:?} Limit: {}", args.country, args.genre, args.limit);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 820.0])
            .with_min_inner_size([720.0, 600.0])
            .with_title("Ham Radio"),
        ..Default::default()
    };
    eframe::run_native("Ham Radio", options, Box::new(|cc| Box::new(HamRadioApp::new(cc, args))))
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(())
}
