//! What the previewer shows and what a click does: the whole picture is drawn again for every frame from the state
//! here, and the places that can be clicked are noted while drawing. Nothing in this file talks to a window.

use crate::audio::Player;
use crate::canvas::{mix, Rect};
use crate::doc::{Doc, NEUTRAL};
use crate::surface::{Font, Surface};

const BG: u32 = 0x1A1B1E;
const PANEL: u32 = 0x202124;
const RAISED: u32 = 0x2A2B30;
const SUNK: u32 = 0x18191C;
const BORDER: u32 = 0x36383E;
const TEXT: u32 = 0xF1F2F4;
const DIM: u32 = 0xB4B7BF;
const MUTED: u32 = 0x82868F;
const TEAL: u32 = 0x2FE3C4;
const TEAL_DEEP: u32 = 0x12362F;
const PURPLE: u32 = 0xA970FF;
const PURPLE_DEEP: u32 = 0x2B2145;
const RED: u32 = 0xFF6B7A;
const CARD: u32 = 0x2C2D33;

// Segoe Fluent Icons
const I_PLAY: char = '\u{E768}';
const I_PAUSE: char = '\u{E769}';
const I_PREV: char = '\u{E892}';
const I_NEXT: char = '\u{E893}';
const I_VOLUME: char = '\u{E767}';
const I_MUTE: char = '\u{E74F}';
const I_OPEN: char = '\u{E838}';
const I_SAVE: char = '\u{E74E}';
const I_INFO: char = '\u{E946}';
const I_MORE: char = '\u{E712}';
const I_FILTER: char = '\u{E71C}';
const I_SEARCH: char = '\u{E721}';
const I_PAGE: char = '\u{E8A5}';
const I_DONE: char = '\u{E930}';
const I_PERSON: char = '\u{E77B}';
const I_CHIP: char = '\u{E950}';
const I_TAG: char = '\u{E8EC}';
const I_BUBBLE: char = '\u{E8BD}';
const I_SMILE: char = '\u{E76E}';
const I_NOTE: char = '\u{E8D6}';
const I_CHART: char = '\u{E9D9}';
/// drawn by hand: a few bars, the sign of sound
const I_WAVE: char = '\0';

/// A place that can be clicked.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    None,
    Open,
    Save,
    Info,
    More,
    ViewWave,
    ViewSpec,
    Wave,
    Prev,
    Back,
    Play,
    Fwd,
    Next,
    Volume,
    Live,
    Filter,
    Search,
    Kebab,
    Row(usize),
    Word(usize, usize),
    Tag(usize),
}

/// What only the window can do for the view.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Request {
    None,
    Open,
    Save,
    /// the menus, with the point to open them at
    MoreMenu(f32, f32),
    FilterMenu(f32, f32),
    KebabMenu(f32, f32),
}

pub struct View {
    pub doc: Option<Doc>,
    pub player: Option<Player>,
    pub error: Option<String>,
    pub spectro: bool,
    /// the transcript follows the sound
    pub live: bool,
    pub query: String,
    pub search_focus: bool,
    pub tag_filter: Option<String>,
    /// the lower panel shows what is known of the file in place of the transcript
    pub details: bool,
    pub volume: f32,
    /// a fixed position instead of the player's, for snapshots
    pub frozen: Option<u64>,
    mute: bool,
    mouse: (f32, f32),
    drag: Hit,
    hits: Vec<(Rect, Hit)>,
    scroll: f32,
    details_scroll: f32,
    /// where the voice names are on the screen, with their speaker ids: what the mouse can rest on
    voices: Vec<(Rect, u32)>,
    // what the last frame worked out
    plot: Rect,
    slider: Rect,
    list: Rect,
    content_h: f32,
    last_row: Option<usize>,
    // kept between frames
    peaks: Vec<(i16, i16)>,
    spec: (usize, usize, Vec<u32>),
    widths: (f32, Vec<Vec<f32>>),
}

pub fn clock(frames: u64, rate: u32) -> String {
    let cs = (frames as f64 / rate.max(1) as f64 * 100.0).floor() as u64;
    format!("{}:{:02}.{:02}", cs / 6000, cs / 100 % 60, cs % 100)
}

/// A part of the details sheet.
enum Item {
    Head(&'static str),
    Pair(String, String),
}

/// A value as text.
fn show(v: &bac::Value) -> String {
    match v {
        bac::Value::Text(t) => t.clone(),
        bac::Value::Bytes(b) => format!("<{} bytes>", b.len()),
    }
}

/// A key as a label: "voice_split" -> "Voice split".
fn title(key: &str) -> String {
    let t = key.replace(['_', '-'], " ");
    let mut c = t.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
}

/// A number with its thousands apart: 713,280.
fn group(n: u64) -> String {
    let d = n.to_string();
    let mut out = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn size_text(bytes: usize) -> String {
    match bytes {
        0..=999 => format!("{bytes} bytes"),
        1000..=999_999 => format!("{:.0} kB", bytes as f64 / 1e3),
        _ => format!("{:.2} MB", bytes as f64 / 1e6),
    }
}

/// The colours and the sign of a tag: edge, fill, glyph.
fn tag_style(label: &str) -> (u32, u32, char) {
    let l = label.to_lowercase();
    if l == NEUTRAL {
        (0x6A6E78, RAISED, I_WAVE)
    } else if l.contains("whisper") {
        (TEAL, TEAL_DEEP, I_BUBBLE)
    } else if l.contains("laugh") {
        (PURPLE, PURPLE_DEEP, I_SMILE)
    } else if l.contains("sing") || l.contains("music") {
        (PURPLE, PURPLE_DEEP, I_NOTE)
    } else {
        const MORE: [(u32, u32); 5] = [(0x5AA9FF, 0x1B2A42), (0xF2B84B, 0x3A2E14), (0xFF7A90, 0x40202A), (0x7BD88F, 0x1D3524), (0xE08CE0, 0x3A2140)];
        let h = l.bytes().fold(7usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize));
        let (edge, fill) = MORE[h % MORE.len()];
        (edge, fill, I_TAG)
    }
}

impl View {
    pub fn new(mute: bool) -> View {
        View {
            doc: None,
            player: None,
            error: None,
            spectro: false,
            live: true,
            query: String::new(),
            search_focus: false,
            tag_filter: None,
            details: false,
            volume: 1.0,
            frozen: None,
            mute,
            mouse: (-1.0, -1.0),
            drag: Hit::None,
            hits: Vec::new(),
            scroll: 0.0,
            details_scroll: 0.0,
            voices: Vec::new(),
            plot: Rect::default(),
            slider: Rect::default(),
            list: Rect::default(),
            content_h: 0.0,
            last_row: None,
            peaks: Vec::new(),
            spec: (0, 0, Vec::new()),
            widths: (0.0, Vec::new()),
        }
    }

    /// Opens a file in place of the one shown. On failure the old one stays and the reason is shown.
    pub fn load(&mut self, path: &str) -> bool {
        match Doc::open(path) {
            Ok(mut doc) => {
                self.player = None;
                let p = Player::new(std::mem::take(&mut doc.pcm), doc.channels, doc.rate, self.mute);
                p.set_volume(self.volume);
                self.player = Some(p);
                self.doc = Some(doc);
                self.error = None;
                self.scroll = 0.0;
                self.details_scroll = 0.0;
                self.query.clear();
                self.tag_filter = None;
                self.last_row = None;
                self.peaks.clear();
                self.spec = (0, 0, Vec::new());
                self.widths = (0.0, Vec::new());
                true
            }
            Err(e) => {
                self.error = Some(e);
                false
            }
        }
    }

    pub fn position(&self) -> u64 {
        self.frozen.or(self.player.as_ref().map(|p| p.position())).unwrap_or(0)
    }

    pub fn playing(&self) -> bool {
        self.player.as_ref().is_some_and(|p| p.playing())
    }

    pub fn toggle_play(&mut self) {
        if let Some(p) = &self.player {
            p.set_playing(!p.playing());
        }
    }

    pub fn seek(&mut self, frame: u64) {
        if let Some(p) = &self.player {
            p.seek(frame);
        }
        if self.frozen.is_some() {
            self.frozen = Some(frame);
        }
    }

    pub fn seek_by(&mut self, secs: f64) {
        if let Some(d) = &self.doc {
            let to = (self.position() as f64 + secs * d.rate as f64).clamp(0.0, d.frames as f64) as u64;
            self.seek(to);
        }
    }

    /// To the start of the row before (-1) or after (+1). Back first goes to the start of the row being heard.
    pub fn jump_row(&mut self, dir: i32) {
        let Some(d) = &self.doc else { return };
        let pos = self.position();
        let started = d.segments.partition_point(|s| s.start <= pos);
        let to = if dir > 0 {
            d.segments.get(started).map(|s| s.start)
        } else {
            let cur = started.checked_sub(1);
            match cur {
                Some(i) if pos - d.segments[i].start > d.rate as u64 => Some(d.segments[i].start),
                Some(i) if i > 0 => Some(d.segments[i - 1].start),
                _ => Some(0),
            }
        };
        if let Some(t) = to {
            self.seek(t);
            self.live = true;
        }
    }

    pub fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.0);
        if let Some(p) = &self.player {
            p.set_volume(self.volume);
        }
    }

    /// The rows that pass the search and the tag filter.
    fn shown(&self) -> Vec<usize> {
        let Some(d) = &self.doc else { return Vec::new() };
        let q = self.query.to_lowercase();
        (0..d.segments.len())
            .filter(|&i| {
                let s = &d.segments[i];
                let tag = match &self.tag_filter {
                    None => true,
                    Some(t) if t == NEUTRAL => s.tags.is_empty(),
                    Some(t) => s.tags.contains(t),
                };
                tag && (q.is_empty() || s.lower.contains(&q))
            })
            .collect()
    }

    /// The transcript as plain text, one row per line.
    pub fn transcript_text(&self) -> String {
        let Some(d) = &self.doc else { return String::new() };
        let mut out = String::new();
        for s in &d.segments {
            let who = s.speaker.and_then(|id| d.labels.speaker(id)).map_or(String::new(), |sp| format!("{}: ", sp.name));
            out.push_str(&format!("[{} - {}] {who}{}\r\n", clock(s.start, d.rate), clock(s.end, d.rate), s.text));
        }
        out
    }

    // -----------------------------------------------------------------------------------------------------------------
    // input

    #[cfg(test)]
    fn place(&self, hit: Hit) -> Rect {
        self.hits.iter().find(|h| h.1 == hit).unwrap_or_else(|| panic!("{hit:?} is not on the screen")).0
    }

    fn hit_at(&self, x: f32, y: f32) -> Hit {
        self.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map_or(Hit::None, |h| h.1)
    }

    /// 0 = arrow, 1 = hand, 2 = text cursor.
    pub fn cursor(&self) -> u8 {
        match self.hit_at(self.mouse.0, self.mouse.1) {
            Hit::None => 0,
            Hit::Search => 2,
            _ => 1,
        }
    }

    fn drag_to(&mut self, x: f32) {
        match self.drag {
            Hit::Wave => {
                if let Some(d) = &self.doc {
                    let t = ((x - self.plot.x) / self.plot.w.max(1.0)).clamp(0.0, 1.0);
                    let to = (t as f64 * d.frames as f64) as u64;
                    self.seek(to);
                }
            }
            Hit::Volume => self.set_volume((x - self.slider.x) / self.slider.w.max(1.0)),
            _ => {}
        }
    }

    pub fn mouse_down(&mut self, x: f32, y: f32) -> Request {
        self.mouse = (x, y);
        let hit = self.hit_at(x, y);
        self.search_focus = hit == Hit::Search;
        match hit {
            Hit::Open => return Request::Open,
            Hit::Save => return Request::Save,
            Hit::Info => self.details = !self.details,
            Hit::More => return Request::MoreMenu(x, y),
            Hit::Filter => return Request::FilterMenu(x, y),
            Hit::Kebab => return Request::KebabMenu(x, y),
            Hit::ViewWave => self.spectro = false,
            Hit::ViewSpec => self.spectro = true,
            Hit::Wave | Hit::Volume => {
                self.drag = hit;
                self.drag_to(x);
                if hit == Hit::Wave {
                    self.live = true;
                }
            }
            Hit::Prev => self.jump_row(-1),
            Hit::Next => self.jump_row(1),
            Hit::Back => self.seek_by(-5.0),
            Hit::Fwd => self.seek_by(5.0),
            Hit::Play => self.toggle_play(),
            Hit::Live => self.live = !self.live,
            Hit::Row(i) | Hit::Word(i, _) => {
                let to = self.doc.as_ref().and_then(|d| {
                    let s = d.segments.get(i)?;
                    Some(match hit {
                        Hit::Word(_, w) => s.words.get(w).map_or(s.start, |w| w.start),
                        _ => s.start,
                    })
                });
                if let Some(t) = to {
                    self.seek(t);
                    self.live = true;
                }
            }
            Hit::Tag(i) => {
                let tag = self.doc.as_ref().and_then(|d| d.tags.get(i).cloned());
                self.tag_filter = if self.tag_filter == tag { None } else { tag };
                self.scroll = 0.0;
            }
            Hit::Search | Hit::None => {}
        }
        Request::None
    }

    pub fn mouse_move(&mut self, x: f32, y: f32) {
        self.mouse = (x, y);
        self.drag_to(x);
    }

    pub fn mouse_up(&mut self) {
        self.drag = Hit::None;
    }

    pub fn dragging(&self) -> bool {
        self.drag != Hit::None
    }

    /// `notches`: wheel steps, up is positive.
    pub fn wheel(&mut self, notches: f32, scale: f32) {
        let most = (self.content_h - self.list.h).max(0.0);
        if self.list.contains(self.mouse.0, self.mouse.1) && self.details {
            self.details_scroll = (self.details_scroll - notches * 80.0 * scale).clamp(0.0, most);
        } else if self.list.contains(self.mouse.0, self.mouse.1) {
            self.scroll = (self.scroll - notches * 80.0 * scale).clamp(0.0, most);
            self.live = false;
        } else if self.slider.contains(self.mouse.0, self.mouse.1) {
            self.set_volume(self.volume + notches * 0.05);
        }
    }

    /// A key that is not a character. `vk` as Windows names it.
    pub fn key(&mut self, vk: usize, ctrl: bool) -> Request {
        const BACK: usize = 8;
        const RETURN: usize = 13;
        const ESC: usize = 27;
        const SPACE: usize = 32;
        const END: usize = 35;
        const HOME: usize = 36;
        const LEFT: usize = 37;
        const UP: usize = 38;
        const RIGHT: usize = 39;
        const DOWN: usize = 40;
        if ctrl {
            match vk as u8 {
                b'O' => return Request::Open,
                b'S' => return Request::Save,
                b'I' => self.details = !self.details,
                b'F' => self.search_focus = true,
                _ => {}
            }
            return Request::None;
        }
        if self.search_focus {
            match vk {
                BACK => {
                    self.query.pop();
                    self.scroll = 0.0;
                }
                ESC => {
                    self.query.clear();
                    self.search_focus = false;
                }
                RETURN => self.search_focus = false,
                _ => {}
            }
            return Request::None;
        }
        match vk {
            SPACE => self.toggle_play(),
            LEFT => self.seek_by(-5.0),
            RIGHT => self.seek_by(5.0),
            UP => self.jump_row(-1),
            DOWN => self.jump_row(1),
            HOME => self.seek(0),
            END => {
                let end = self.doc.as_ref().map_or(0, |d| d.frames);
                self.seek(end);
            }
            ESC if self.details => self.details = false,
            ESC => {
                self.tag_filter = None;
                self.query.clear();
            }
            _ => {}
        }
        Request::None
    }

    pub fn typed(&mut self, c: char) {
        if self.search_focus && !c.is_control() && self.query.chars().count() < 80 {
            self.query.push(c);
            self.scroll = 0.0;
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // drawing

    fn hover(&self, r: Rect) -> bool {
        self.drag == Hit::None && r.contains(self.mouse.0, self.mouse.1)
    }

    fn panel(s: &mut Surface, r: Rect) {
        let sc = s.scale;
        let mut c = s.canvas();
        c.rrect(r, 7.0 * sc, PANEL, 1.0);
        c.stroke(r, 7.0 * sc, 1.0, BORDER, 1.0);
    }

    /// A glyph of the icon font in the middle of `r`; the hand-drawn wave sign for `I_WAVE`.
    fn icon(s: &mut Surface, r: Rect, glyph: char, big: bool, color: u32) {
        if glyph == I_WAVE {
            let sc = s.scale;
            let mut c = s.canvas();
            let h = if big { 16.0 } else { 12.0 } * sc;
            for (i, part) in [0.35f32, 0.8, 1.0, 0.55, 0.3].iter().enumerate() {
                let x = r.cx() + (i as f32 - 2.0) * 3.0 * sc;
                c.line(x, r.cy() - h * part * 0.5, x, r.cy() + h * part * 0.5, 1.4 * sc, color);
            }
            return;
        }
        let mut b = [0u8; 4];
        s.text_in(r, glyph.encode_utf8(&mut b), if big { Font::IconBig } else { Font::Icon }, color, 1);
    }

    /// A square button with a glyph.
    fn button(&mut self, s: &mut Surface, r: Rect, hit: Hit, glyph: char, on: bool) {
        let sc = s.scale;
        let fill = if on { TEAL_DEEP } else if self.hover(r) { 0x34363C } else { RAISED };
        let mut c = s.canvas();
        c.rrect(r, 6.0 * sc, fill, 1.0);
        c.stroke(r, 6.0 * sc, 1.0, if on { TEAL } else { 0x44474E }, 1.0);
        Self::icon(s, r, glyph, false, if on { TEAL } else { TEXT });
        self.hits.push((r, hit));
    }

    /// A tag's chip at (x, y). `lit`: the tag sounds right now; `picked`: the filter is on it. Returns its width.
    fn chip(s: &mut Surface, x: f32, y: f32, label: &str, lit: bool, picked: bool) -> f32 {
        let sc = s.scale;
        let (edge, fill, glyph) = tag_style(label);
        let w = (s.measure(label, Font::Body) + 44.0 * sc).round();
        let r = Rect::new(x, y, w, 25.0 * sc);
        let mut c = s.canvas();
        c.rrect(r, 7.0 * sc, if lit { mix(fill, edge, 0.35) } else { fill }, 1.0);
        c.stroke(r, 7.0 * sc, if picked { 2.0 * sc } else { 1.0 }, edge, 1.0);
        Self::icon(s, Rect::new(x + 6.0 * sc, y, 26.0 * sc, r.h), glyph, false, if glyph == I_WAVE { DIM } else { edge });
        s.text_in(Rect::new(x + 33.0 * sc, y - sc, w, r.h), label, Font::Body, TEXT, 0);
        w
    }

    /// Draws everything.
    pub fn draw(&mut self, s: &mut Surface) {
        let sc = s.scale;
        let u = |v: f32| (v * sc).round();
        let (w, h) = (s.w as f32, s.h as f32);
        self.hits.clear();
        self.voices.clear();
        s.set_clip(None);
        s.canvas().fill(Rect::new(0.0, 0.0, w, h), BG);
        let pos = self.position();
        let cur = self.doc.as_ref().and_then(|d| d.segment_at(pos));

        // ---- the file
        let tile = Rect::new(u(22.0), u(14.0), u(52.0), u(52.0));
        s.canvas().rrect(tile, u(9.0), 0x35333F, 1.0);
        Self::icon(s, tile, I_WAVE, true, 0xC3A1FF);
        let buttons = [(Hit::Open, I_OPEN), (Hit::Save, I_SAVE), (Hit::Info, I_INFO), (Hit::More, I_MORE)];
        let bx = w - u(16.0) - buttons.len() as f32 * u(50.0) + u(10.0);
        for (i, (hit, glyph)) in buttons.into_iter().enumerate() {
            self.button(s, Rect::new(bx + i as f32 * u(50.0), u(23.0), u(40.0), u(34.0)), hit, glyph, hit == Hit::Info && self.details);
        }
        let room = bx - u(110.0);
        let (name, sub) = match &self.doc {
            Some(d) => {
                let kind = if d.head.is_some() { "BantamAudioCodec (BAC)".to_string() } else { d.codec() };
                (d.name.clone(), format!("{kind}  ·  {}  ·  {:.1} kbit/s", size_text(d.file_bytes), d.file_bytes as f64 * 8.0 / d.seconds().max(1e-9) / 1000.0))
            }
            None => ("No file".into(), "Drop a BAC, WAV or FLAC file here, or press Ctrl+O".into()),
        };
        let name = s.fit(&name, Font::Title, room);
        s.text(u(92.0), u(15.0), &name, Font::Title, TEXT);
        let sub = s.fit(&sub, Font::Body, room);
        s.text(u(92.0), u(43.0), &sub, Font::Body, DIM);

        // ---- the three areas
        let side_w = if w < u(980.0) { u(290.0) } else { u(370.0) };
        let top = u(80.0);
        let wave = Rect::new(u(16.0), top, w - u(44.0) - side_w, u(262.0));
        let meta = Rect::new(wave.right() + u(12.0), top, side_w, u(152.0));
        let tags = Rect::new(meta.x, meta.bottom() + u(12.0), side_w, wave.h - meta.h - u(12.0));
        let status_h = u(38.0);
        let script = Rect::new(u(16.0), wave.bottom() + u(14.0), w - u(32.0), (h - status_h - wave.bottom() - u(26.0)).max(u(60.0)));
        self.draw_wave(s, wave, pos);
        self.draw_meta(s, meta, cur);
        self.draw_tags(s, tags, cur);
        self.draw_script(s, script, pos, cur);

        // ---- the status line
        s.set_clip(None);
        s.canvas().fill(Rect::new(0.0, h - status_h, w, 1.0), BORDER);
        let bar = Rect::new(u(16.0), h - status_h + 1.0, w - u(32.0), status_h - 1.0);
        let (state, color) = match (&self.error, &self.player) {
            (Some(e), _) => (e.clone(), RED),
            (None, Some(p)) if !p.has_device() && self.frozen.is_none() => ("No sound device: the time runs without sound".to_string(), RED),
            (None, Some(p)) if p.playing() => ("Playing".to_string(), TEXT),
            (None, Some(_)) if pos > 0 => ("Paused".to_string(), TEXT),
            _ => ("Ready".to_string(), TEXT),
        };
        Self::icon(s, Rect::new(bar.x, bar.y, u(22.0), bar.h), I_DONE, false, if color == RED { RED } else { TEAL });
        let right = match &self.doc {
            Some(d) => format!("Rows: {}    |    Duration: {}    |    Codec: {}", d.segments.len(), clock(d.frames, d.rate), d.codec()),
            None => String::new(),
        };
        let rw = s.text_in(bar, &right, Font::Body, DIM, 2);
        let state = s.fit(&state, Font::Body, bar.w - rw - u(60.0));
        s.text_in(Rect::new(bar.x + u(32.0), bar.y, bar.w, bar.h), &state, Font::Body, color, 0);

        // ---- over everything: what is known of the voice the mouse rests on
        let (mx, my) = self.mouse;
        if let Some(&(spot, id)) = self.voices.iter().find(|v| v.0.contains(mx, my)).filter(|_| self.drag == Hit::None) {
            self.draw_voice_card(s, spot, id);
        }
    }

    /// A card under (or over) the voice name at `spot`: the name, a description if the file gives one, every
    /// attribute of the voice, and how much it says in this file.
    fn draw_voice_card(&self, s: &mut Surface, spot: Rect, id: u32) {
        let Some(d) = &self.doc else { return };
        let Some(sp) = d.labels.speaker(id) else { return };
        let sc = s.scale;
        let u = |v: f32| (v * sc).round();
        let (w, pad) = (u(336.0).min(s.w as f32 - u(24.0)), u(14.0));
        let inner = w - 2.0 * pad;
        let is_about = |k: &str| matches!(k, "description" | "about" | "notes" | "note");
        let about: Vec<String> = match sp.attrs.iter().find(|a| is_about(&a.key)) {
            Some(a) => s.wrap(&show(&a.value), Font::Body, inner),
            None => Vec::new(),
        };
        let keys: Vec<String> = sp.attrs.iter().filter(|a| !is_about(&a.key)).map(|a| title(&a.key)).collect();
        let key_w = (keys.iter().map(|k| s.measure(k, Font::Body)).fold(0.0, f32::max) + u(16.0)).min(inner * 0.45);
        let rows: Vec<(String, Vec<String>)> = sp.attrs.iter().filter(|a| !is_about(&a.key)).zip(keys).map(|(a, k)| (k, s.wrap(&show(&a.value), Font::Body, inner - key_w))).collect();
        let mine = d.segments.iter().filter(|g| g.speaker == Some(id));
        let (count, spoken) = mine.fold((0u64, 0u64), |(n, t), g| (n + 1, t + (g.end - g.start)));
        let share = format!("{count} row{} in this file, {} of speech", if count == 1 { "" } else { "s" }, clock(spoken, d.rate));
        let line = s.line(Font::Body) + u(4.0);
        let head_h = u(48.0);
        let body: f32 = about.len() as f32 * line + if about.is_empty() { 0.0 } else { u(8.0) } + rows.iter().map(|r| r.1.len().max(1) as f32 * line).sum::<f32>();
        let nothing = about.is_empty() && rows.is_empty();
        let h = head_h + u(10.0) + if nothing { line } else { body } + u(8.0) + u(30.0);
        // under the name; over it when there is no room under
        let x = spot.x.min(s.w as f32 - w - u(10.0)).max(u(10.0));
        let y = if spot.bottom() + u(8.0) + h < s.h as f32 - u(8.0) { spot.bottom() + u(8.0) } else { (spot.y - u(8.0) - h).max(u(8.0)) };
        let card = Rect::new(x, y, w, h);
        {
            let mut c = s.canvas();
            c.rrect(Rect::new(x - u(2.0), y + u(3.0), w + u(4.0), h + u(4.0)), u(11.0), 0x000000, 0.4);
            c.rrect(card, u(9.0), CARD, 1.0);
            c.stroke(card, u(9.0), 1.0, 0x565A63, 1.0);
            c.circle(x + pad + u(14.0), y + head_h * 0.5 + u(2.0), 14.0 * sc, TEAL_DEEP);
            c.fill(Rect::new(x + pad, y + head_h + u(2.0), inner, 1.0), 0x45484F);
        }
        Self::icon(s, Rect::new(x + pad, y + u(2.0), u(28.0), head_h), I_PERSON, false, TEAL);
        let tag = format!("voice {}", sp.id);
        let tag_w = s.text_in(Rect::new(x + pad, y + u(2.0), inner, head_h), &tag, Font::Small, MUTED, 2);
        let name = s.fit(&sp.name, Font::Head, inner - u(48.0) - tag_w);
        s.text_in(Rect::new(x + pad + u(38.0), y + u(1.0), inner, head_h), &name, Font::Head, TEXT, 0);
        let mut ty = y + head_h + u(12.0);
        if nothing {
            s.text(x + pad, ty, "The file says nothing more about this voice", Font::Body, MUTED);
            ty += line;
        }
        for l in &about {
            s.text(x + pad, ty, l, Font::Body, DIM);
            ty += line;
        }
        if !about.is_empty() {
            ty += u(8.0);
        }
        for (key, lines) in &rows {
            s.text(x + pad, ty, key, Font::Body, MUTED);
            for l in lines {
                s.text(x + pad + key_w, ty, l, Font::Body, TEXT);
                ty += line;
            }
            if lines.is_empty() {
                ty += line;
            }
        }
        s.canvas().fill(Rect::new(x + pad, ty + u(6.0), inner, 1.0), 0x45484F);
        s.text_in(Rect::new(x + pad, ty + u(7.0), inner, u(30.0)), &share, Font::Small, DIM, 0);
    }

    /// The lower panel as a sheet of facts: the file, its attributes, its voices.
    fn draw_details(&mut self, s: &mut Surface, r: Rect, head: Rect) {
        let sc = s.scale;
        let u = |v: f32| (v * sc).round();
        Self::icon(s, Rect::new(r.x + u(20.0), head.y, u(26.0), head.h), I_INFO, true, TEXT);
        s.text_in(Rect::new(r.x + u(60.0), head.y - sc, r.w, head.h), "File details", Font::Title, TEXT, 0);
        // the way back
        let label = "Transcript";
        let bw = s.measure(label, Font::Body) + u(48.0);
        let back = Rect::new(r.right() - u(14.0) - bw, head.y + u(9.0), bw, u(32.0));
        {
            let over = self.hover(back);
            let mut c = s.canvas();
            c.rrect(back, u(6.0), if over { 0x34363C } else { RAISED }, 1.0);
            c.stroke(back, u(6.0), 1.0, 0x44474E, 1.0);
        }
        Self::icon(s, Rect::new(back.x + u(8.0), back.y, u(24.0), back.h), I_PAGE, false, TEXT);
        s.text_in(Rect::new(back.x + u(36.0), back.y - sc, bw, back.h), label, Font::Body, TEXT, 0);
        self.hits.push((back, Hit::Info));
        s.canvas().fill(Rect::new(r.x + 1.0, head.bottom(), r.w - 2.0, 1.0), BORDER);
        let area = Rect::new(r.x + 1.0, head.bottom() + 1.0, r.w - 2.0, r.bottom() - head.bottom() - u(5.0));
        self.list = area;
        let Some(d) = &self.doc else {
            s.text_in(area, "Open a file to see what it is", Font::Body, MUTED, 1);
            self.content_h = 0.0;
            return;
        };

        // ---- what there is to say
        let secs = d.seconds().max(1e-9);
        let mut file = vec![Item::Head("File"), Item::Pair("Path".into(), d.path.clone())];
        let pair = |k: &str, v: String| Item::Pair(k.into(), v);
        match &d.head {
            Some(h) => {
                file.push(pair("Format", format!("BantamAudioCodec (BAC), format version {}", bac::container::VERSION)));
                file.push(pair("Coding", if h.params.lossless { "lossless: decodes to the exact samples of the source".into() } else { h.quality.map_or("lossy".into(), |q| format!("lossy, quality {q} of 10")) }));
            }
            None => file.push(pair("Format", d.codec())),
        }
        file.push(pair("Sound", format!("{} Hz, {} channel{}, {} bits", group(d.rate as u64), d.channels, if d.channels == 1 { "" } else { "s" }, d.bits)));
        file.push(pair("Duration", format!("{}  ({} samples)", clock(d.frames, d.rate), group(d.frames))));
        file.push(pair("Size", format!("{} bytes, {:.1} kbit/s", group(d.file_bytes as u64), d.file_bytes as f64 * 8.0 / secs / 1000.0)));
        if let Some(st) = &d.stats {
            let every = if st.keys > 0 { format!(", one every {:.1} s", secs / st.keys as f64) } else { String::new() };
            file.push(pair("Packets", format!("{}, of which {} can be decoded on their own{every}", group(st.packets as u64), group(st.keys as u64))));
            // the events of every tier, in the order the tiers first appear
            let mut tiers: Vec<(&str, usize)> = Vec::new();
            for e in &d.labels.events {
                match tiers.iter_mut().find(|t| t.0 == e.tier) {
                    Some(t) => t.1 += 1,
                    None => tiers.push((&e.tier, 1)),
                }
            }
            let tiers: Vec<String> = tiers.iter().map(|(t, n)| format!("{} {t}", group(*n as u64))).collect();
            file.push(pair("Labels", if st.label_bytes == 0 { "none".into() } else { format!("{} bytes{}{}", group(st.label_bytes), if tiers.is_empty() { "" } else { ": events " }, tiers.join(", ")) }));
        }
        if !d.segments.is_empty() {
            file.push(pair("Word times", if d.word_times { "from the file's word tier".into() } else { "estimated from each row's time (the file has no word tier)".into() }));
        }
        let mut more = Vec::new();
        if !d.labels.attrs.is_empty() {
            more.push(Item::Head("Attributes"));
            more.extend(d.labels.attrs.iter().map(|a| Item::Pair(title(&a.key), show(&a.value))));
        }
        if !d.labels.speakers.is_empty() {
            more.push(Item::Head("Voices"));
            for sp in &d.labels.speakers {
                let attrs: Vec<String> = sp.attrs.iter().map(|a| format!("{}: {}", a.key, show(&a.value))).collect();
                let mine = d.segments.iter().filter(|g| g.speaker == Some(sp.id));
                let (count, spoken) = mine.fold((0u64, 0u64), |(n, t), g| (n + 1, t + (g.end - g.start)));
                let said = format!("{count} row{}, {}", if count == 1 { "" } else { "s" }, clock(spoken, d.rate));
                more.push(Item::Pair(sp.name.clone(), if attrs.is_empty() { said } else { format!("{}\n{said}", attrs.join(", ")) }));
            }
        }

        // ---- two columns side by side when there is room, one under the other when not
        let two = area.w >= u(820.0) && !more.is_empty();
        let col_w = if two { (area.w - u(76.0)) * 0.5 } else { area.w - u(48.0) };
        let left = area.x + u(22.0);
        let (h1, h2) = (Self::flow(s, &file, left, 0.0, col_w, false), Self::flow(s, &more, left, 0.0, col_w, false));
        self.content_h = if two { h1.max(h2) } else { h1 + h2 + u(8.0) } + u(26.0);
        let most = (self.content_h - area.h).max(0.0);
        self.details_scroll = self.details_scroll.clamp(0.0, most).round();
        s.set_clip(Some(area));
        let top = area.y + u(14.0) - self.details_scroll;
        Self::flow(s, &file, left, top, col_w, true);
        if two {
            Self::flow(s, &more, left + col_w + u(32.0), top, col_w, true);
        } else {
            Self::flow(s, &more, left, top + h1 + u(8.0), col_w, true);
        }
        if most > 0.0 {
            let track = area.h - u(8.0);
            let len = (track * area.h / self.content_h).max(u(24.0));
            s.canvas().rrect(Rect::new(area.right() - u(7.0), area.y + u(4.0) + (track - len) * self.details_scroll / most, u(4.0), len), u(2.0), 0x5A5D66, 1.0);
        }
        s.set_clip(None);
    }

    /// Lays `items` out downwards from (x, y) in a column `w` wide, drawing them if `draw`. Returns the y after
    /// the last one.
    fn flow(s: &mut Surface, items: &[Item], x: f32, y: f32, w: f32, draw: bool) -> f32 {
        let sc = s.scale;
        let u = |v: f32| (v * sc).round();
        let line = s.line(Font::Body) + u(4.0);
        let key_w = u(112.0).min(w * 0.4);
        let mut y = y;
        for (i, item) in items.iter().enumerate() {
            match item {
                Item::Head(t) => {
                    if i > 0 {
                        y += u(16.0);
                    }
                    if draw {
                        s.text(x, y, t, Font::Bold, TEXT);
                        let under = y + s.line(Font::Bold) + u(5.0);
                        s.canvas().fill(Rect::new(x, under, w, 1.0), BORDER);
                    }
                    y += s.line(Font::Bold) + u(14.0);
                }
                Item::Pair(k, v) => {
                    let lines = s.wrap(v, Font::Body, w - key_w);
                    if draw {
                        let k = s.fit(k, Font::Body, key_w - u(10.0));
                        s.text(x, y, &k, Font::Body, MUTED);
                        for (j, l) in lines.iter().enumerate() {
                            s.text(x + key_w, y + j as f32 * line, l, Font::Body, TEXT);
                        }
                    }
                    y += lines.len().max(1) as f32 * line + u(4.0);
                }
            }
        }
        y
    }

    fn draw_wave(&mut self, s: &mut Surface, r: Rect, pos: u64) {
        let sc = s.scale;
        let u = |v: f32| (v * sc).round();
        Self::panel(s, r);
        // the switch between the two pictures
        let pill = Rect::new(r.cx() - u(134.0), r.y + u(10.0), u(268.0), u(26.0));
        {
            let mut c = s.canvas();
            c.rrect(pill, u(13.0), SUNK, 1.0);
            c.stroke(pill, u(13.0), 1.0, 0x44474E, 1.0);
        }
        for (i, (label, glyph, hit)) in [("Waveform", I_WAVE, Hit::ViewWave), ("Spectrogram", I_CHART, Hit::ViewSpec)].into_iter().enumerate() {
            let half = Rect::new(pill.x + i as f32 * pill.w * 0.5, pill.y, pill.w * 0.5, pill.h);
            let on = self.spectro == (i == 1);
            if on {
                let mut c = s.canvas();
                c.rrect(half, u(13.0), TEAL_DEEP, 1.0);
                c.stroke(half, u(13.0), 1.2 * sc, TEAL, 1.0);
            }
            let tw = s.measure(label, Font::Body);
            let x = half.cx() - (tw + u(24.0)) * 0.5;
            Self::icon(s, Rect::new(x, half.y, u(18.0), half.h), glyph, false, if on { TEAL } else { DIM });
            s.text_in(Rect::new(x + u(24.0), half.y - sc, tw, half.h), label, Font::Body, if on { TEXT } else { DIM }, 0);
            self.hits.push((half, hit));
        }

        // the picture
        let plot = Rect::new(r.x + u(18.0), r.y + u(46.0), r.w - u(36.0), u(122.0));
        self.plot = plot;
        s.canvas().rrect(plot, u(5.0), SUNK, 1.0);
        let Some(d) = &self.doc else {
            s.canvas().stroke(plot, u(5.0), 1.0, BORDER, 1.0);
            s.text_in(plot, "Nothing to show yet", Font::Body, MUTED, 1);
            self.draw_transport(s, r, 0, 0, 1);
            return;
        };
        let inner = plot.inset(1.0);
        let (cols, rows) = (inner.w as usize, inner.h as usize);
        // the marks of the time axis: the smallest round step that leaves room for the numbers
        let secs = d.seconds().max(1e-6);
        let most = (plot.w / u(84.0)).max(2.0) as f64;
        let step = [0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0].into_iter().find(|st| secs / st <= most).unwrap_or(7200.0);
        if self.spectro {
            if (self.spec.0, self.spec.1) != (cols, rows) {
                let levels = d.spectrogram(cols, rows);
                let color = |v: u8| {
                    let t = v as f32 / 255.0;
                    if t < 0.45 {
                        mix(SUNK, 0x14705F, t / 0.45)
                    } else if t < 0.85 {
                        mix(0x14705F, TEAL, (t - 0.45) / 0.4)
                    } else {
                        mix(TEAL, 0xFFFFFF, (t - 0.85) / 0.15)
                    }
                };
                self.spec = (cols, rows, levels.into_iter().map(color).collect());
            }
            let c = s.canvas();
            let (x0, y0) = (inner.x as usize, inner.y as usize);
            for y in 0..rows {
                let dst = (y0 + y) * c.w as usize + x0;
                c.px[dst..dst + cols].copy_from_slice(&self.spec.2[y * cols..(y + 1) * cols]);
            }
        } else {
            if self.peaks.len() != cols {
                self.peaks = d.peaks(cols);
            }
            // quiet files are drawn larger, up to four times
            let top = self.peaks.iter().map(|p| (p.0 as i32).abs().max((p.1 as i32).abs())).max().unwrap_or(1).max(8192) as f32;
            let half = inner.h * 0.5 - u(5.0);
            let mut c = s.canvas();
            c.clip = (inner.x as i32, inner.y as i32, inner.right() as i32, inner.bottom() as i32);
            let mut t = step;
            while t < secs {
                c.vline(inner.x + (t / secs) as f32 * inner.w, inner.y, inner.bottom(), 0x7A7E88, 0.55, (3.0 * sc) as i32);
                t += step;
            }
            for (i, &(lo, hi)) in self.peaks.iter().enumerate() {
                let (a, b) = (inner.cy() - hi as f32 / top * half, inner.cy() - lo as f32 / top * half);
                c.fill(Rect::new(inner.x + i as f32, a.floor(), 1.0, (b - a).max(1.0).ceil()), TEAL);
            }
        }
        s.canvas().stroke(plot, u(5.0), 1.0, 0x3E4047, 1.0);
        // the numbers under it
        let mut t = 0.0;
        while t < secs + 1e-9 {
            let label = format!("{}:{:02}", (t as u64) / 60, (t as u64) % 60);
            let label = if step < 1.0 { format!("{label}.{}", ((t * 10.0).round() as u64) % 10) } else { label };
            let x = plot.x + (t / secs) as f32 * plot.w;
            let tw = s.measure(&label, Font::Small);
            s.text((x - tw * 0.5).clamp(plot.x - u(6.0), plot.right() - tw + u(6.0)), plot.bottom() + u(5.0), &label, Font::Small, DIM);
            t += step;
        }
        // where the sound is
        let x = (plot.x + (pos as f64 / d.frames.max(1) as f64) as f32 * plot.w).round();
        {
            let mut c = s.canvas();
            c.rrect(Rect::new(x - sc, plot.y - u(4.0), 2.0 * sc, plot.h + u(4.0)), sc, PURPLE, 1.0);
            c.circle(x, plot.y - u(4.0), 5.0 * sc, PURPLE);
        }
        self.hits.push((Rect::new(plot.x - u(6.0), plot.y - u(8.0), plot.w + u(12.0), plot.h + u(12.0)), Hit::Wave));
        let (frames, rate) = (d.frames, d.rate);
        self.draw_transport(s, r, pos, frames, rate);
    }

    /// The row of buttons under the picture.
    fn draw_transport(&mut self, s: &mut Surface, r: Rect, pos: u64, frames: u64, rate: u32) {
        let sc = s.scale;
        let u = |v: f32| (v * sc).round();
        let row = Rect::new(r.x, r.bottom() - u(60.0), r.w, u(60.0));
        s.canvas().fill(Rect::new(r.x + 1.0, row.y, r.w - 2.0, 1.0), BORDER);
        let mid = row.cy();
        let mut x = row.x + u(26.0);
        let playing = self.playing();
        for (hit, glyph) in [(Hit::Prev, I_PREV), (Hit::Back, '5'), (Hit::Play, if playing { I_PAUSE } else { I_PLAY }), (Hit::Fwd, '5'), (Hit::Next, I_NEXT)] {
            let (bw, bh) = if hit == Hit::Play { (u(42.0), u(34.0)) } else { (u(36.0), u(34.0)) };
            let b = Rect::new(x, mid - bh * 0.5, bw, bh);
            let over = self.hover(b);
            match hit {
                Hit::Play => {
                    let mut c = s.canvas();
                    c.rrect(b, u(7.0), if over { 0x2AD3B5 } else { 0x1FB89C }, 1.0);
                    c.stroke(b, u(7.0), 1.0, TEAL, 1.0);
                    Self::icon(s, b, glyph, false, 0x06221D);
                }
                Hit::Back | Hit::Fwd => {
                    // a round arrow with the number of seconds in it
                    if over {
                        s.canvas().rrect(b, u(7.0), RAISED, 1.0);
                    }
                    let (cx, cy, rad) = (b.cx(), b.cy() + sc, 9.5 * sc);
                    let mut c = s.canvas();
                    let fwd = hit == Hit::Fwd;
                    if fwd {
                        c.arc(cx, cy, rad, 1.5 * sc, 35.0, 335.0, TEXT);
                        c.poly(&[(cx + 1.0 * sc, cy - rad - 4.0 * sc), (cx + 7.5 * sc, cy - rad + 1.0 * sc), (cx + 0.5 * sc, cy - rad + 4.5 * sc)], TEXT);
                    } else {
                        c.arc(cx, cy, rad, 1.5 * sc, 25.0, 325.0, TEXT);
                        c.poly(&[(cx - 1.0 * sc, cy - rad - 4.0 * sc), (cx - 7.5 * sc, cy - rad + 1.0 * sc), (cx - 0.5 * sc, cy - rad + 4.5 * sc)], TEXT);
                    }
                    s.text_in(Rect::new(b.x, b.y + 1.5 * sc, b.w, b.h), "5", Font::Small, TEXT, 1);
                }
                _ => {
                    if over {
                        s.canvas().rrect(b, u(7.0), RAISED, 1.0);
                    }
                    Self::icon(s, b, glyph, true, TEXT);
                }
            }
            self.hits.push((b, hit));
            x += bw + u(14.0);
        }
        x += u(4.0);
        s.canvas().fill(Rect::new(x, mid - u(14.0), 1.0, u(28.0)), 0x4A4D55);
        x += u(22.0);
        let now = clock(pos, rate);
        let tw = s.text(x, mid - s.line(Font::Head) * 0.5, &now, Font::Head, TEXT);
        let total = format!("  /  {}", clock(frames, rate));
        let tw2 = s.text(x + tw, mid - s.line(Font::Body) * 0.5 + sc, &total, Font::Body, DIM);
        // the width of the widest time, so that nothing after it moves while playing
        x += s.measure("00:00.00", Font::Head).max(tw) + tw2 + u(20.0);
        s.canvas().fill(Rect::new(x, mid - u(14.0), 1.0, u(28.0)), 0x4A4D55);
        x += u(18.0);
        if row.right() - x < u(150.0) {
            self.slider = Rect::default();
            return;
        }
        Self::icon(s, Rect::new(x, row.y, u(26.0), row.h), if self.volume < 0.005 { I_MUTE } else { I_VOLUME }, true, TEXT);
        x += u(40.0);
        let percent = format!("{:.0}%", self.volume * 100.0);
        let pw = s.measure("100%", Font::Body);
        let track = Rect::new(x, mid - u(2.0), (row.right() - x - pw - u(40.0)).clamp(u(50.0), u(110.0)), u(4.0));
        self.slider = Rect::new(track.x - u(8.0), mid - u(14.0), track.w + u(16.0), u(28.0));
        {
            let mut c = s.canvas();
            c.rrect(track, u(2.0), 0x4A4D55, 1.0);
            c.rrect(Rect::new(track.x, track.y, track.w * self.volume, track.h), u(2.0), TEAL, 1.0);
            c.circle(track.x + track.w * self.volume, mid, 7.0 * sc, TEAL);
        }
        self.hits.push((self.slider, Hit::Volume));
        s.text_in(Rect::new(track.right() + u(18.0), row.y, pw, row.h), &percent, Font::Body, TEXT, 0);
    }

    fn draw_meta(&mut self, s: &mut Surface, r: Rect, cur: Option<usize>) {
        let sc = s.scale;
        let u = |v: f32| (v * sc).round();
        Self::panel(s, r);
        s.text(r.x + u(18.0), r.y + u(10.0), "Metadata", Font::Head, TEXT);
        s.canvas().fill(Rect::new(r.x + u(16.0), r.y + u(40.0), r.w - u(32.0), 1.0), BORDER);
        let mut rows: Vec<(char, &str, String)> = Vec::new();
        if let Some(d) = &self.doc {
            // who speaks now; before anything is said, everyone in the file
            let speaker = cur.and_then(|i| d.segments[i].speaker).and_then(|id| d.labels.speaker(id));
            let who = match speaker {
                Some(sp) => sp.name.clone(),
                None => d.labels.speakers.iter().map(|sp| sp.name.as_str()).collect::<Vec<_>>().join(", "),
            };
            let text = |sp: &bac::Speaker, key: &str| sp.attrs.iter().find(|a| a.key == key).and_then(|a| if let bac::Value::Text(t) = &a.value { Some(t.clone()) } else { None });
            let profile = speaker.or(if d.labels.speakers.len() == 1 { d.labels.speakers.first() } else { None }).map(|sp| {
                text(sp, "profile").or(text(sp, "voice")).unwrap_or_else(|| {
                    let known: Vec<String> = ["gender", "age", "accent", "style", "personality"].iter().filter_map(|k| text(sp, k)).collect();
                    if known.is_empty() {
                        // whatever else is known of the voice; where it sits in the file is not part of it
                        let other = sp.attrs.iter().filter(|a| a.key != "role" && a.key != "channel");
                        other.filter_map(|a| text(sp, &a.key).map(|v| format!("{}: {v}", a.key))).collect::<Vec<_>>().join(", ")
                    } else {
                        known.join(", ")
                    }
                })
            });
            let dash = |v: String| if v.is_empty() { "–".to_string() } else { v };
            let khz = d.rate as f64 / 1000.0;
            let rate = format!("{} kHz{}", if khz.fract() == 0.0 { format!("{khz:.0}") } else { format!("{khz:.2}").trim_end_matches('0').to_string() }, if d.channels > 1 { format!(", {} channels", d.channels) } else { String::new() });
            rows.push((I_PERSON, "Speaker:", dash(who)));
            rows.push((I_WAVE, "Voice Profile:", dash(profile.unwrap_or_default())));
            rows.push((I_CHIP, "Codec:", d.codec()));
            rows.push((I_CHART, "Sample Rate:", rate));
        }
        let label_w = s.measure("Voice Profile:", Font::Body) + u(18.0);
        for (i, (glyph, label, value)) in rows.iter().enumerate() {
            let row = Rect::new(r.x + u(18.0), r.y + u(47.0) + i as f32 * u(24.5), r.w - u(36.0), u(24.5));
            Self::icon(s, Rect::new(row.x, row.y, u(22.0), row.h), *glyph, false, DIM);
            s.text_in(Rect::new(row.x + u(36.0), row.y, label_w, row.h), label, Font::Body, DIM, 0);
            let vx = row.x + u(36.0) + label_w;
            let value = s.fit(value, Font::Body, row.right() - vx);
            s.text_in(Rect::new(vx, row.y, row.right() - vx, row.h), &value, Font::Body, TEXT, 0);
        }
    }

    fn draw_tags(&mut self, s: &mut Surface, r: Rect, cur: Option<usize>) {
        let sc = s.scale;
        let u = |v: f32| (v * sc).round();
        Self::panel(s, r);
        s.text(r.x + u(18.0), r.y + u(9.0), "Tags", Font::Head, TEXT);
        s.canvas().fill(Rect::new(r.x + u(16.0), r.y + u(38.0), r.w - u(32.0), 1.0), BORDER);
        let Some(d) = &self.doc else { return };
        if d.tags.is_empty() {
            s.text(r.x + u(18.0), r.y + u(48.0), "No tags in this file", Font::Body, MUTED);
            return;
        }
        s.set_clip(Some(r.inset(2.0)));
        let (mut x, mut y) = (r.x + u(16.0), r.y + u(48.0));
        for (i, tag) in d.tags.iter().enumerate() {
            let lit = cur.is_some_and(|c| if tag == NEUTRAL { d.segments[c].tags.is_empty() } else { d.segments[c].tags.contains(tag) });
            let w = s.measure(tag, Font::Body) + u(44.0);
            if x + w > r.right() - u(12.0) && x > r.x + u(16.0) {
                x = r.x + u(16.0);
                y += u(31.0);
            }
            let w = Self::chip(s, x, y, tag, lit, self.tag_filter.as_ref() == Some(tag));
            self.hits.push((Rect::new(x, y, w, u(25.0)), Hit::Tag(i)));
            x += w + u(8.0);
        }
        s.set_clip(None);
    }

    fn draw_script(&mut self, s: &mut Surface, r: Rect, pos: u64, cur: Option<usize>) {
        let sc = s.scale;
        let u = |v: f32| (v * sc).round();
        Self::panel(s, r);
        let head = Rect::new(r.x, r.y, r.w, u(50.0));
        if self.details {
            self.draw_details(s, r, head);
            return;
        }
        // ---- its head: title, live, filter, search, menu
        Self::icon(s, Rect::new(r.x + u(20.0), head.y, u(26.0), head.h), I_PAGE, true, TEXT);
        let title = if self.live { "Transcript (live)" } else { "Transcript" };
        let tw = s.text_in(Rect::new(r.x + u(60.0), head.y - sc, r.w, head.h), title, Font::Title, TEXT, 0);
        let mut right = r.right() - u(14.0);
        let kebab = Rect::new(right - u(24.0), head.y + u(9.0), u(24.0), u(32.0));
        {
            let over = self.hover(kebab);
            let mut c = s.canvas();
            if over {
                c.rrect(kebab, u(6.0), RAISED, 1.0);
            }
            for i in -1..=1 {
                c.circle(kebab.cx(), kebab.cy() + i as f32 * 6.5 * sc, 1.7 * sc, TEXT);
            }
        }
        self.hits.push((kebab, Hit::Kebab));
        right = kebab.x - u(10.0);
        let search_w = (r.w - tw - u(330.0)).clamp(u(120.0), u(250.0));
        let search = Rect::new(right - search_w, head.y + u(9.0), search_w, u(32.0));
        {
            let mut c = s.canvas();
            c.rrect(search, u(6.0), SUNK, 1.0);
            c.stroke(search, u(6.0), 1.0, if self.search_focus { TEAL } else { 0x4A4D55 }, 1.0);
        }
        Self::icon(s, Rect::new(search.x + u(8.0), search.y, u(26.0), search.h), I_SEARCH, false, TEXT);
        let field = Rect::new(search.x + u(42.0), search.y - sc, search.w - u(50.0), search.h);
        s.set_clip(Some(Rect::new(field.x, search.y, field.w, search.h)));
        let qw = if self.query.is_empty() && !self.search_focus {
            s.text_in(field, "Search transcript...", Font::Body, MUTED, 0);
            0.0
        } else {
            let q = self.query.clone();
            s.text_in(field, &q, Font::Body, TEXT, 0)
        };
        if self.search_focus {
            s.canvas().fill(Rect::new(field.x + qw + sc, search.y + u(8.0), (1.5 * sc).max(1.0), search.h - u(16.0)), TEAL);
        }
        s.set_clip(None);
        self.hits.push((search, Hit::Search));
        right = search.x - u(10.0);
        let filter = Rect::new(right - u(40.0), head.y + u(9.0), u(40.0), u(32.0));
        self.button(s, filter, Hit::Filter, I_FILTER, self.tag_filter.is_some());
        right = filter.x - u(12.0);
        let live = Rect::new(right - u(62.0), head.y + u(12.0), u(62.0), u(26.0));
        {
            let over = self.hover(live);
            let mut c = s.canvas();
            c.rrect(live, u(6.0), if self.live { TEAL_DEEP } else if over { RAISED } else { PANEL }, 1.0);
            if !self.live {
                c.stroke(live, u(6.0), 1.0, 0x4A4D55, 1.0);
            }
            c.circle(live.x + u(14.0), live.cy(), 4.0 * sc, if self.live { TEAL } else { MUTED });
        }
        s.text_in(Rect::new(live.x + u(25.0), live.y - sc, live.w, live.h), "Live", Font::Body, if self.live { TEAL } else { DIM }, 0);
        self.hits.push((live, Hit::Live));
        if let Some(d) = self.doc.as_ref().filter(|d| !d.word_times && !d.segments.is_empty()) {
            let _ = d;
            let note = "word times are estimates";
            if r.x + u(76.0) + tw + s.measure(note, Font::Small) < live.x - u(10.0) {
                s.text_in(Rect::new(r.x + u(76.0) + tw, head.y + sc, r.w, head.h), note, Font::Small, MUTED, 0);
            }
        }

        // ---- the column heads
        let cols = Rect::new(r.x + 1.0, head.bottom(), r.w - 2.0, u(34.0));
        s.canvas().fill(cols, RAISED);
        let (c_num, c_time, c_voice) = (r.x + u(56.0), r.x + u(96.0), r.x + u(244.0));
        // the voice column is as wide as the longest name, within bounds; a file without speakers has none
        let names: Vec<String> = self.doc.as_ref().map_or(Vec::new(), |d| d.labels.speakers.iter().map(|sp| sp.name.clone()).collect());
        let widest = s.measure("Voice", Font::Bold);
        let widest = names.iter().fold(widest, |w, n| w.max(s.measure(n, Font::Bold)));
        let voice_w = if names.is_empty() { 0.0 } else { widest.min(u(140.0)) + u(28.0) };
        let c_tag = c_voice + voice_w;
        // a narrow window gives the text some of the room of the tags
        let c_text = c_tag + if r.w < u(760.0) { u(112.0) } else { u(144.0) };
        s.text_in(Rect::new(c_num - u(20.0), cols.y, u(40.0), cols.h), "#", Font::Bold, TEXT, 1);
        for (x, label) in [(c_time, "Time"), (c_voice, "Voice"), (c_tag, "Tag"), (c_text, "Text")] {
            if label != "Voice" || voice_w > 0.0 {
                s.text_in(Rect::new(x, cols.y - sc, u(100.0), cols.h), label, Font::Bold, TEXT, 0);
            }
        }
        let list = Rect::new(r.x + 1.0, cols.bottom(), r.w - 2.0, r.bottom() - cols.bottom() - u(4.0));
        self.list = list;
        let Some(d) = &self.doc else {
            s.text_in(list, "Open a file to see its transcript", Font::Body, MUTED, 1);
            return;
        };
        let shown = self.shown();
        if shown.is_empty() {
            let why = if d.segments.is_empty() { "This file has no transcript" } else { "No row matches the search and the filter" };
            s.text_in(Rect::new(list.x, list.y, list.w, list.h.min(u(120.0))), why, Font::Body, MUTED, 1);
            self.content_h = 0.0;
            return;
        }

        // ---- the widths of all words, measured once per scale
        if self.widths.0 != sc || self.widths.1.len() != d.segments.len() {
            self.widths = (sc, d.segments.iter().map(|seg| seg.words.iter().map(|w| s.measure(&w.text, Font::Body)).collect()).collect());
        }
        let space = s.measure(" ", Font::Body).max(3.0 * sc) + 1.5 * sc;
        let line_h = s.line(Font::Body) + u(5.0);
        let text_w = (list.right() - u(18.0) - c_text).max(u(80.0));
        // where every shown row lies: (row, top, height), with the words wrapped into the text column
        let mut lay: Vec<(usize, f32, f32)> = Vec::with_capacity(shown.len());
        let mut y = 0.0;
        for &i in &shown {
            let (mut x, mut lines) = (0.0f32, 1);
            for &ww in &self.widths.1[i] {
                if x > 0.0 && x + ww > text_w {
                    x = 0.0;
                    lines += 1;
                }
                x += ww + space;
            }
            let hh = (lines as f32 * line_h + u(16.0)).max(u(42.0));
            lay.push((i, y, hh));
            y += hh;
        }
        self.content_h = y;
        // ---- follow the sound: when it enters another row, bring that row into view
        let max_scroll = (self.content_h - list.h).max(0.0);
        if self.live && cur != self.last_row {
            if let Some(&(_, top, hh)) = cur.and_then(|c| lay.iter().find(|l| l.0 == c)) {
                if top < self.scroll || top + hh > self.scroll + list.h {
                    self.scroll = top - (list.h - hh).max(0.0) * 0.3;
                }
            }
        }
        self.last_row = cur;
        self.scroll = self.scroll.clamp(0.0, max_scroll).round();

        s.set_clip(Some(list));
        for &(i, top, hh) in &lay {
            let row = Rect::new(list.x, list.y + top - self.scroll, list.w, hh);
            if row.bottom() < list.y || row.y > list.bottom() {
                continue;
            }
            let seg = &d.segments[i];
            let active = cur == Some(i);
            let over = self.drag == Hit::None && row.contains(self.mouse.0, self.mouse.1) && list.contains(self.mouse.0, self.mouse.1);
            {
                let mut c = s.canvas();
                if active {
                    let box_ = Rect::new(row.x, row.y + 1.0, row.w, row.h - 1.0);
                    c.rrect(box_, u(6.0), TEAL_DEEP, 1.0);
                    c.stroke(box_, u(6.0), 1.2 * sc, TEAL, 1.0);
                } else {
                    if over {
                        c.fill(row, 0x26272B);
                    }
                    c.fill(Rect::new(row.x, row.bottom().round(), row.w, 1.0), 0x303238);
                }
            }
            self.hits.push((Rect::new(row.x, row.y.max(list.y), row.w, row.bottom().min(list.bottom()) - row.y.max(list.y)), Hit::Row(i)));
            let first = Rect::new(row.x, row.y + u(8.0), row.w, line_h);
            if active {
                s.canvas().poly(&[(row.x + u(22.0), first.cy() - u(7.0)), (row.x + u(34.0), first.cy()), (row.x + u(22.0), first.cy() + u(7.0))], TEAL);
            }
            s.text_in(Rect::new(c_num - u(20.0), first.y, u(40.0), first.h), &format!("{}", i + 1), Font::Body, TEXT, 1);
            s.text_in(Rect::new(c_time, first.y, u(140.0), first.h), &format!("{} – {}", clock(seg.start, d.rate), clock(seg.end, d.rate)), Font::Body, TEXT, 0);
            // the voice: its name, which the mouse can rest on for more
            if voice_w > 0.0 {
                let cell = Rect::new(c_voice, first.y, voice_w - u(16.0), first.h);
                match seg.speaker.and_then(|id| d.labels.speaker(id)) {
                    Some(sp) => {
                        let name = s.fit(&sp.name, Font::Bold, cell.w);
                        let nw = s.measure(&name, Font::Bold);
                        let spot = Rect::new(cell.x - u(4.0), cell.y, nw + u(8.0), cell.h);
                        let on = self.drag == Hit::None && spot.contains(self.mouse.0, self.mouse.1) && list.contains(self.mouse.0, self.mouse.1);
                        s.text_in(cell, &name, Font::Bold, if active || on { TEAL } else { 0xD5D8DE }, 0);
                        if on {
                            s.canvas().fill(Rect::new(cell.x, cell.bottom() - u(2.0), nw, 1.0), TEAL);
                        }
                        if spot.bottom() > list.y && spot.y < list.bottom() {
                            self.voices.push((spot, sp.id));
                        }
                    }
                    None => {
                        s.text_in(cell, "–", Font::Body, MUTED, 0);
                    }
                }
            }
            // the tag: the first one, and how many more
            let tag = seg.tags.first().map_or(NEUTRAL, |t| t.as_str());
            s.set_clip(Some(Rect::new(c_tag, row.y.max(list.y), c_text - c_tag - u(8.0), row.bottom().min(list.bottom()) - row.y.max(list.y))));
            let cw = Self::chip(s, c_tag, first.cy() - u(12.5), tag, active && !seg.tags.is_empty(), false);
            if seg.tags.len() > 1 {
                s.text_in(Rect::new(c_tag + cw + u(6.0), first.y, u(40.0), first.h), &format!("+{}", seg.tags.len() - 1), Font::Small, DIM, 0);
            }
            s.set_clip(Some(list));
            // the words
            let mut x = 0.0;
            let mut ly = first.y;
            for (wi, word) in seg.words.iter().enumerate() {
                let ww = self.widths.1[i][wi];
                if x > 0.0 && x + ww > text_w {
                    x = 0.0;
                    ly += line_h;
                }
                let cell = Rect::new(c_text + x, ly, ww, line_h);
                let said = active && pos >= word.start && pos < word.end;
                if said {
                    s.canvas().rrect(Rect::new(cell.x - 2.5 * sc, cell.y + u(1.0), ww + 5.0 * sc, line_h - u(2.0)), u(4.0), TEAL, 1.0);
                }
                // in the row being heard, the words still to come are dimmer
                let color = if said { 0x06221D } else if active && pos < word.start { 0xA9C9C3 } else { TEXT };
                s.text_in(cell, &word.text, Font::Body, color, 0);
                if cell.bottom() > list.y && cell.y < list.bottom() {
                    self.hits.push((Rect::new(cell.x - space * 0.5, cell.y, ww + space, line_h), Hit::Word(i, wi)));
                }
                x += ww + space;
            }
        }
        // ---- how far the list is scrolled
        if max_scroll > 0.0 {
            let track = list.h - u(8.0);
            let len = (track * list.h / self.content_h).max(u(24.0));
            let at = list.y + u(4.0) + (track - len) * self.scroll / max_scroll;
            s.canvas().rrect(Rect::new(list.right() - u(7.0), at, u(4.0), len), u(2.0), 0x5A5D66, 1.0);
        }
        s.set_clip(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bac::labels::{Event, Labels};

    /// Six seconds with three rows: the first has timed words, the second is whispered.
    fn file() -> String {
        let rate = 16_000u64;
        let pcm = bac::Pcm { rate: rate as u32, bits: 16, ch: vec![(0..6 * rate).map(|i| ((i % 160) as i32 - 80) * 150).collect()] };
        let mut l = Labels::default();
        let ev = |tier: &str, a: u64, b: u64, text: &str| Event { tier: tier.into(), start: a, end: b, speaker: None, weight: None, text: text.into(), attrs: vec![] };
        let ann = l.speaker_id("ann");
        l.speakers[0].attrs = vec![
            bac::Attr { key: "description".into(), value: bac::Value::Text("A calm reader with a low voice, recorded in a quiet room for this test.".into()) },
            bac::Attr { key: "accent".into(), value: bac::Value::Text("General American".into()) },
        ];
        l.add(Event { speaker: Some(ann), ..ev("text", 0, 2 * rate, "one two three four") });
        for (i, w) in ["one", "two", "three", "four"].iter().enumerate() {
            l.add(ev("word", i as u64 * 8000, i as u64 * 8000 + 6000, w));
        }
        l.add(ev("text", 2 * rate, 4 * rate, "a quiet second row"));
        l.add(ev("tag", 2 * rate, 4 * rate, "whisper"));
        l.add(ev("text", 4 * rate, 6 * rate, "and the third"));
        let path = std::env::temp_dir().join(format!("bac-view-test-{}.bac", std::process::id()));
        std::fs::write(&path, bac::encode(&pcm, &bac::EncOpts::default(), &l).unwrap()).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn clicks_and_keys_do_what_they_show() {
        let path = file();
        let mut v = View::new(true);
        assert!(v.load(&path), "{:?}", v.error);
        std::fs::remove_file(&path).ok();
        let mut s = Surface::new(1090, 820, 1.0);
        v.draw(&mut s);
        let click = |v: &mut View, x: f32, y: f32| {
            let r = v.mouse_down(x, y);
            v.mouse_up();
            r
        };

        // a word: the sound goes to its start, and the word is the one lit
        let w = v.place(Hit::Word(0, 2));
        click(&mut v, w.cx(), w.cy());
        assert_eq!(v.position(), 16_000);
        v.draw(&mut s);
        let lit = |s: &Surface, r: Rect| s.pixels()[(r.y as usize + 3) * s.w as usize + r.x as usize + 3] == TEAL;
        assert!(lit(&s, v.place(Hit::Word(0, 2))) && !lit(&s, v.place(Hit::Word(0, 1))) && !lit(&s, v.place(Hit::Word(0, 3))));

        // the waveform: the middle of it is the middle of the file
        let p = v.plot;
        click(&mut v, p.cx(), p.cy());
        assert!((v.position() as i64 - 48_000).abs() < 600, "{}", v.position());

        // keys
        v.key(37, false);
        assert!(v.position() < 600, "five seconds back from the middle of six is the start");
        v.key(40, false);
        assert_eq!(v.position(), 32_000);
        v.key(40, false);
        assert_eq!(v.position(), 64_000);
        v.key(38, false);
        assert_eq!(v.position(), 32_000);
        v.key(36, false);
        assert_eq!(v.position(), 0);

        // a row
        v.draw(&mut s);
        let r = v.place(Hit::Row(2));
        click(&mut v, r.x + 60.0, r.cy());
        assert_eq!(v.position(), 64_000);

        // the search box takes what is typed, and the rows follow
        let b = v.place(Hit::Search);
        click(&mut v, b.cx(), b.cy());
        "QUIET".chars().for_each(|c| v.typed(c));
        assert_eq!(v.shown(), [1]);
        v.key(32, false);
        assert!(!v.playing(), "a space in the search box is not play");
        v.key(27, false);
        assert_eq!(v.shown(), [0, 1, 2]);

        // a tag: only the rows that have it; again: all rows
        v.draw(&mut s);
        let t = v.place(Hit::Tag(0));
        click(&mut v, t.cx(), t.cy());
        assert_eq!((v.tag_filter.as_deref(), v.shown()), (Some("whisper"), vec![1]));
        v.draw(&mut s);
        let t = v.place(Hit::Tag(1));
        click(&mut v, t.cx(), t.cy());
        assert_eq!((v.tag_filter.as_deref(), v.shown()), (Some("neutral"), vec![0, 2]));
        click(&mut v, t.cx(), t.cy());
        assert_eq!(v.shown().len(), 3);

        // the voice: its name is in the table, and resting on it shows the card
        v.draw(&mut s);
        let (spot, id) = v.voices[0];
        assert_eq!((v.voices.len(), id), (1, 1));
        let under = |s: &Surface| s.pixels()[(spot.bottom() as usize + 14) * s.w as usize + spot.x as usize + 60];
        assert_ne!(under(&s), CARD);
        v.mouse_move(spot.cx(), spot.cy());
        v.draw(&mut s);
        assert_eq!(under(&s), CARD);
        v.mouse_move(5.0, 5.0);
        v.draw(&mut s);
        assert_ne!(under(&s), CARD);

        // the details take the transcript's place and give it back
        let b = v.place(Hit::Info);
        click(&mut v, b.cx(), b.cy());
        v.draw(&mut s);
        assert!(v.details && !v.hits.iter().any(|h| matches!(h.1, Hit::Row(_) | Hit::Search)));
        assert!(v.content_h > 100.0);
        v.key(27, false);
        assert!(!v.details);
        assert_eq!(v.key(b'I' as usize, true), Request::None);
        assert!(v.details);
        v.key(b'I' as usize, true);
        v.draw(&mut s);
        assert!(!v.details && v.hits.iter().any(|h| h.1 == Hit::Row(0)));

        // the buttons that need the window ask for it
        v.draw(&mut s);
        let o = v.place(Hit::Open);
        assert_eq!(click(&mut v, o.cx(), o.cy()), Request::Open);
        assert_eq!(v.key(b'S' as usize, true), Request::Save);

        // play: the position moves (in silence), pause: it stays
        v.key(36, false);
        let b = v.place(Hit::Play);
        click(&mut v, b.cx(), b.cy());
        assert!(v.playing());
        std::thread::sleep(std::time::Duration::from_millis(600));
        let at = v.position();
        assert!(at > 4_000 && at < 16_000, "after 0.6 s of play the position is {at}");
        click(&mut v, b.cx(), b.cy());
        std::thread::sleep(std::time::Duration::from_millis(150));
        let stopped = v.position();
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!v.playing() && v.position() == stopped);

        // near the end: it plays to the end and stops there; play then starts over
        v.seek(96_000 - 3_200);
        v.toggle_play();
        std::thread::sleep(std::time::Duration::from_millis(700));
        assert!(!v.playing() && v.position() == 96_000, "playing {}, at {}", v.playing(), v.position());
        v.toggle_play();
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(v.playing() && v.position() < 16_000);
    }

    #[test]
    fn the_times_read_as_minutes_and_seconds() {
        assert_eq!(clock(0, 16_000), "0:00.00");
        assert_eq!(clock(16_000 * 75 + 8_000, 16_000), "1:15.50");
        assert_eq!(size_text(1_280_000), "1.28 MB");
        assert_eq!((group(713_280), group(999), group(1_000)), ("713,280".to_string(), "999".to_string(), "1,000".to_string()));
        assert_eq!(title("voice_split"), "Voice split");
    }
}
