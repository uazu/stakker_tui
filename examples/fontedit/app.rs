use crate::Font;
use stakker::{ActorOwn, CX, StopCause, actor, call, fail, fwd_to, ret_some_to, stop};
use stakker_tui::{Key, Region, TermShare, Terminal, Tile, sizer::SimpleSizer};
use std::collections::VecDeque;
use std::io::Write as IoWrite;

pub struct App {
    term: ActorOwn<Terminal>,
    tsh: Option<TermShare>,
    param_tile: Tile,
    cells_tile: Tile,
    mini_tile: Tile,
    font: Font,
    cx: usize, // Cursor position
    cy: usize,
    index: usize,  // Current character index
    unsaved: bool, // Needs saving
    save_error: Option<String>,
    mode: Mode,
    clip: Vec<bool>,
    undo: VecDeque<Font>,
}

#[derive(Copy, Clone)]
enum Mode {
    Move,
    Draw,
    Roll,
}

impl Mode {
    fn caps(self) -> &'static str {
        match self {
            Self::Move => "MOVE",
            Self::Draw => "DRAW",
            Self::Roll => "ROLL",
        }
    }
    fn cursor(self) -> &'static str {
        match self {
            Self::Move => "<>",
            Self::Draw => "[]",
            Self::Roll => "><",
        }
    }
}

impl App {
    pub fn init(cx: CX![], font: Font) -> Option<Self> {
        let term = actor!(
            cx,
            Terminal::init(
                SimpleSizer::new(),
                fwd_to!([cx], resize() as (Option<TermShare>)),
                fwd_to!([cx], input() as (Key))
            ),
            ret_some_to!([cx], |_, cx, cause: StopCause| {
                println!("Terminal actor failed: {cause}");
                stop!(cx);
            })
        );
        Some(Self {
            term,
            tsh: None,
            param_tile: Tile::new(),
            cells_tile: Tile::new(),
            mini_tile: Tile::new(),
            font,
            cx: 0,
            cy: 0,
            index: 0,
            unsaved: false,
            save_error: None,
            mode: Mode::Move,
            clip: Vec::new(),
            undo: VecDeque::new(),
        })
    }

    fn resize(&mut self, cx: CX![], tsh: Option<TermShare>) {
        self.tsh = tsh;
        self.redraw(cx);
    }

    fn redraw(&mut self, cx: CX![]) {
        if let Some(ref tsh) = self.tsh {
            let out = tsh.output(cx);
            out.attr_99().cursor_show().scroll_up().save_cleanup();
            out.cursor_hide().clear_all_99_and_remote();

            // Draw fixed stuff here, then break out and store
            // sub-tiles for areas that need to be updated later
            let tile = tsh.tile(cx);
            let sx = tile.sx();
            let sy = tile.sy();
            let split_y1 = sy - self.font.sy as i32;
            let split_y0 = split_y1.min(10);
            let (top, _, cells_tile) = tile.split_yy(split_y0, split_y1);
            let spare = sx.saturating_sub(80);
            let (ox, mini_sx) = if (spare as usize) < self.font.sx / 2 {
                (spare / 2, 0)
            } else {
                (0, spare)
            };
            let (_, rest) = top.split_x(ox);
            let (param_tile, mini_tile, mut keys_tile) = rest.split_xx(40, 40 + mini_sx);

            if let Some(mut r) = keys_tile.region(cx, 0, 0, split_y0, 40) {
                r.hfb(172).clear_all();
                let text = "\
Keys: Arrows move, [Space] change pixel
Mode on/off: [d] draw, [r] roll
Fill: [f] flood, [v] vert, [h] horiz
[n] new glyph, [u] undo, [l] ruler line
Clip: [c] copy, [P] paste.
Index: [PgUp] [PgDn] [Home] [End].
[S] save, [Q] save+quit, [^C] hard quit
Edit the font file directly to change
font size or codepoint numbers.";
                r.text(&text.replace('[', "\0171 ").replace(']', " \0172"));
            }
            self.cells_tile = cells_tile;
            self.param_tile = param_tile;
            self.mini_tile = mini_tile;

            self.draw_cells(cx);
            self.draw_mini(cx);
            self.draw_param(cx);
        }
    }

    fn draw_param(&mut self, cx: CX![]) {
        if let Some(mut r) = self.param_tile.full(cx) {
            r.hfb(172).clear_all();

            // Use sub-region to clip the filename if it's too long
            r.region(0, 0, 1, 39)
                .hfb(172)
                .text("File:  ")
                .text(&self.font.path.to_string_lossy());

            let count = self.font.index_limit();
            let index = self.index;

            let state = if let Some(ref err) = self.save_error {
                err
            } else if self.unsaved {
                "UNSAVED"
            } else {
                ""
            };
            let cp = self.font.chars[index].cp;
            let ch = match cp {
                32..=126 | 160.. => char::from_u32(cp).unwrap_or('?'),
                _ => '?',
            };
            let sx = self.font.sx;
            let sy = self.font.sy;

            r.at(1, 0);
            let _ = writeln!(r, "Size:  {sx} x {sy}, {count} glyphs");
            let _ = writeln!(r, "Index: {index} U+{cp:04x} '{ch}'");
            let _ = writeln!(r, "Mode:  {}", self.mode.caps());
            let _ = writeln!(r, "State: {}", state);
        }
    }

    fn draw_cells(&mut self, cx: CX![]) {
        if let Some(mut full) = self.cells_tile.full(cx) {
            let sx = full.sx();
            full.clear_all_99();

            let font_sx = self.font.sx;
            let font_sy = self.font.sy;
            let cell_wid = font_sx * 2;
            let r = full.region(
                0,
                (sx - cell_wid as i32) / 2,
                font_sy as i32,
                cell_wid as i32,
            );
            draw_cell(
                &self.font,
                r,
                self.index,
                171,
                7,
                Some((self.cy, self.cx, self.mode)),
            );
            let max = self.font.index_limit();
            for i in 1..=self.index {
                let r = full.region(
                    0,
                    (sx - (cell_wid * (1 + i * 2)) as i32) / 2,
                    font_sy as i32,
                    cell_wid as i32,
                );
                if !r.is_visible() {
                    break;
                }
                draw_cell(&self.font, r, self.index - i, 70, 6, None);
            }
            for i in 1..(max - self.index) {
                let r = full.region(
                    0,
                    (sx + (cell_wid * (i * 2 - 1)) as i32) / 2,
                    font_sy as i32,
                    cell_wid as i32,
                );
                if !r.is_visible() {
                    break;
                }
                draw_cell(&self.font, r, self.index + i, 70, 6, None);
            }
        }
    }

    fn draw_mini(&mut self, cx: CX![]) {
        if let Some(mut full) = self.mini_tile.full(cx) {
            full.clear_all_99();
            let sx = full.sx() * 2;
            let sy = full.sy() * 4;
            let font_sx = self.font.sx as i32;
            let font_sy = self.font.sy as i32;
            let oy = sy.saturating_sub(font_sy) / 2;
            let mut ox = (sx - font_sx) / 2;
            let mut index = self.index;
            while ox > 0 && index > 0 {
                ox -= font_sx;
                index -= 1;
            }
            while ox < sx {
                draw_mini_cell(&self.font, full.full(), oy, ox, index, 170);
                ox += font_sx;
                index += 1;
            }
        }
    }

    fn draw_all(&mut self, cx: CX![]) {
        self.draw_cells(cx);
        self.draw_mini(cx);
        self.draw_param(cx);
    }

    fn input(&mut self, cx: CX![], key: Key) {
        match key {
            Key::Ctrl('L') => self.redraw(cx),
            Key::Ctrl('C') => stop!(cx),
            Key::Left => {
                if self.cx > 0 {
                    self.move_or_draw(cx, 0, -1)
                }
            }
            Key::Right => {
                if self.cx + 1 < self.font.sx {
                    self.move_or_draw(cx, 0, 1)
                }
            }
            Key::Up => {
                if self.cy > 0 {
                    self.move_or_draw(cx, -1, 0)
                }
            }
            Key::Down => {
                if self.cy + 1 < self.font.sy {
                    self.move_or_draw(cx, 1, 0)
                }
            }
            Key::PgUp => {
                if self.index > 0 {
                    self.index -= 1;
                    self.draw_all(cx);
                }
            }
            Key::PgDn => {
                if self.index + 1 < self.font.index_limit() {
                    self.index += 1;
                    self.draw_all(cx);
                }
            }
            Key::Home => {
                self.index = 0;
                self.draw_all(cx);
            }
            Key::End => {
                self.index = self.font.index_limit() - 1;
                self.draw_all(cx);
            }
            Key::Pr(' ') => {
                self.undo_push();
                self.unsaved = true;
                let curr = self.font.get(self.index, self.cy, self.cx);
                self.font.set(self.index, self.cy, self.cx, !curr);
                self.draw_all(cx);
            }
            Key::Pr('d') => {
                self.mode = match self.mode {
                    Mode::Draw => Mode::Move,
                    _ => Mode::Draw,
                };
                self.draw_all(cx);
            }
            Key::Pr('r') => {
                self.mode = match self.mode {
                    Mode::Roll => Mode::Move,
                    _ => Mode::Roll,
                };
                self.draw_all(cx);
            }
            Key::Pr('f') => {
                self.undo_push();
                self.unsaved = true;
                self.font.fill(self.index, self.cy, self.cx, true, true);
                self.draw_all(cx);
            }
            Key::Pr('h') => {
                self.undo_push();
                self.unsaved = true;
                self.font.fill(self.index, self.cy, self.cx, false, true);
                self.draw_all(cx);
            }
            Key::Pr('v') => {
                self.undo_push();
                self.unsaved = true;
                self.font.fill(self.index, self.cy, self.cx, true, false);
                self.draw_all(cx);
            }
            Key::Pr('u') => {
                if let Some(font) = self.undo.pop_front() {
                    self.font = font;
                    let limit = self.font.index_limit();
                    self.index = self.index.min(limit - 1);
                    self.draw_all(cx);
                }
            }
            Key::Pr('l') => {
                self.font.rule_bm ^= 1 << self.cy;
                self.unsaved = true;
                self.draw_all(cx);
            }
            Key::Pr('n') => {
                self.undo_push();
                self.unsaved = true;
                self.index = self.font.index_limit();
                self.font.add_glyph();
                self.draw_param(cx);
                self.draw_cells(cx);
            }
            Key::Pr('c') => {
                self.clip = self.font.get_glyph(self.index);
            }
            Key::Pr('P') => {
                if !self.clip.is_empty() {
                    self.undo_push();
                    self.unsaved = true;
                    self.font.set_glyph(self.index, &self.clip);
                    self.draw_all(cx);
                }
            }
            Key::Pr('S') => {
                if let Err(e) = self.font.save() {
                    self.save_error = Some(e.to_string());
                } else {
                    self.save_error = None;
                    self.unsaved = false;
                }
                self.draw_param(cx);
            }
            Key::Pr('Q') => {
                if let Err(e) = self.font.save() {
                    fail!(cx, e);
                } else {
                    stop!(cx);
                }
            }
            _ => call!([self.term], bell()),
        }
    }

    fn move_or_draw(&mut self, cx: CX![], dy: i32, dx: i32) {
        match self.mode {
            Mode::Move => {
                self.cx = (self.cx as i32 + dx) as usize;
                self.cy = (self.cy as i32 + dy) as usize;
                self.draw_cells(cx);
            }
            Mode::Draw => {
                self.undo_push();
                self.unsaved = true;
                let curr = self.font.get(self.index, self.cy, self.cx);
                self.cx = (self.cx as i32 + dx) as usize;
                self.cy = (self.cy as i32 + dy) as usize;
                self.font.set(self.index, self.cy, self.cx, curr);
                self.draw_all(cx);
            }
            Mode::Roll => {
                self.undo_push();
                if dx != 0 {
                    self.font.roll_horiz(self.index, dx);
                }
                if dy != 0 {
                    self.font.roll_vert(self.index, dy);
                }
                self.draw_all(cx);
            }
        }
    }

    fn undo_push(&mut self) {
        self.undo.push_front(self.font.clone());
        while self.undo.len() > 50 {
            self.undo.pop_back();
        }
    }
}

fn draw_cell(
    font: &Font,
    mut r: Region,
    index: usize,
    black: u16,
    white: u16,
    curs: Option<(usize, usize, Mode)>,
) {
    let form = &font.chars[index].form;
    for y in 0..font.sy {
        let rule = ((font.rule_bm >> y) & 1) != 0;
        for x in 0..font.sx {
            r.hfb(if form[x + y * font.sx] { white } else { black });
            let mut text = if rule { "--" } else { "  " };
            if let Some((cy, cx, mode)) = curs
                && cy == y
                && cx == x
            {
                text = mode.cursor();
            }
            r.text(text);
        }
    }
}

fn draw_mini_cell(font: &Font, mut r: Region, oy: i32, ox: i32, index: usize, hfb: u16) {
    if index < font.chars.len() {
        let form = &font.chars[index].form;
        for y in 0..font.sy {
            for x in 0..font.sx {
                if form[x + y * font.sx] {
                    r.braille_plot(oy + y as i32, ox + x as i32, hfb, true);
                }
            }
        }
    }
}
