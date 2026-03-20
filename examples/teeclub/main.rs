//! Teeclub patience card game.  Uses tile-based output

#![allow(clippy::octal_escapes)] // Allow \0HFB sequences

use stakker::{ActorOwn, CX, Core, Ret, Stakker, StopCause};
use stakker::{actor, after, fwd_to, ret, ret_shutdown, ret_some_to, stop};
use stakker_mio::MioPoll;
use stakker_mio::mio::{Events, Poll};
use stakker_tui::{Key, Region, TermShare, Terminal, Tile, sizer::SimpleSizer};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

mod cardgame;
mod draw;
pub use cardgame::{Card, Hand, Rand};
use draw::CARD_SY;

// Main loop dumps setup I/O errors to console.  You could catch them
// instead if you want.
fn main() -> std::io::Result<()> {
    let mut stakker = Stakker::new(Instant::now());
    let s = &mut stakker;
    let miopoll = MioPoll::new(s, Poll::new()?, Events::with_capacity(1024), 0)?;

    let _app = actor!(s, App::init(), ret_shutdown!(s));

    const MAXDELAY: Duration = Duration::from_secs(60);
    let mut idle_pending = s.run(Instant::now(), false);
    let mut io_pending = false;
    let mut activity;
    while s.not_shutdown() {
        let maxdur = s.next_wait_max(Instant::now(), MAXDELAY, idle_pending || io_pending);
        (activity, io_pending) = miopoll.poll(maxdur)?;
        idle_pending = s.run(Instant::now(), !activity);
    }
    if let Some(reason) = s.shutdown_reason()
        && reason.has_error()
    {
        println!("{reason}");
    }

    Ok(())
}

/// App main actor
pub struct App {
    _terminal: ActorOwn<Terminal>,
    tsh: Option<TermShare>,
    logo_hfb: Option<Vec<u16>>, // Logo HFB if 256-colour is available
    game: Option<Game>,
    scores_tile: Tile,
    scores: Vec<i32>,
    score: Option<i32>,
    last_score: Option<i32>,
    adjust: i32,
}

impl App {
    fn init(cx: CX![]) -> Option<Self> {
        let _terminal = actor!(
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

        let mut this = Self {
            _terminal,
            tsh: None,
            logo_hfb: None,
            game: None,
            scores_tile: Default::default(),
            scores: Vec::new(),
            score: None,
            last_score: None,
            adjust: 0,
        };
        let _ = this.load_scores();
        Some(this)
    }

    fn resize(&mut self, cx: CX![], tsh: Option<TermShare>) {
        self.tsh = tsh;

        if let Some(ref tsh) = self.tsh {
            let out = tsh.output(cx);
            out.attr_99().cursor_show().scroll_up().save_cleanup();
            out.cursor_hide().clear_all_99();

            if self.logo_hfb.is_none() && out.features().colour_256 {
                const LOGO_RGB: [(f64, u32); 7] = [
                    (0.0, 0xFFFF00),
                    (1.0, 0x00FF00),
                    (2.0, 0x22FFFF),
                    (3.0, 0x4444FF),
                    (4.0, 0xFF22FF),
                    (5.0, 0xFF0000),
                    (6.0, 0xFFFF00),
                ];
                self.logo_hfb = Some(
                    (0..30)
                        .map(|i| {
                            let rgb = out.rgb_interpolate(&LOGO_RGB, i as f64 * 0.2);
                            out.hfb_alloc(false, false, rgb, 0x000000)
                        })
                        .collect(),
                );
            }

            self.layout(cx);
        }
    }

    /// Redraw, with intentional visual flash (for ^L)
    fn redraw(&mut self, cx: CX![]) {
        let Some(ref tsh) = self.tsh else {
            return;
        };
        let mut tile = tsh.tile(cx);
        if let Some(mut r) = tile.full(cx) {
            r.clear_all_99();
        }
        after!(Duration::from_millis(10), [cx], layout());
    }

    fn layout(&mut self, cx: CX![]) {
        let Some(ref tsh) = self.tsh else {
            return;
        };

        let mut tile = tsh.tile(cx);
        if let Some(mut r) = tile.full(cx) {
            r.clear_all_99();
        }
        let left_gap = 0.max(tile.sx() - 80) / 2;
        let right_off = (left_gap + 80).min(tile.sx());
        let right_gap = tile.sx() - right_off;
        self.adjust = self.adjust.max(-left_gap).min(right_gap);
        let left_gap = left_gap + self.adjust;
        let right_off = (left_gap + 80).min(tile.sx());

        let (_, mid, _) = tile.split_xx(left_gap, right_off);
        let (main, scores) = mid.split_x(74);
        self.scores_tile = scores;
        let sy = main.sy();
        if let Some(game) = &mut self.game {
            game.layout(cx, main);
        } else {
            let logo_sy = 9;
            let instr_sy = 16;
            let gap = (sy - logo_sy - instr_sy) / 3;
            let (_, mut logo, rest) = main.split_yy(gap, gap + logo_sy);
            let (_, mut text, _) = rest.split_yy(gap, gap + instr_sy);
            if let Some(r) = logo.full(cx) {
                self.draw_logo(r);
            }
            if let Some(r) = text.full(cx) {
                Self::draw_instructions(r);
            }
        }

        self.draw_scores(cx);
    }

    fn draw_scores(&mut self, cx: CX![]) {
        let mut score = self.score;
        let mut last_score = self.last_score;
        if let Some(mut r) = self.scores_tile.full(cx) {
            r.hfb(176).clear_all();
            r.hfb(7).text("SCORES");
            let mut y = 1;
            let last_y = r.sy() - 1;
            let mut draw = move |hfb, sc| {
                if y > 0 {
                    r.at(y, 0).hfb(hfb);
                    write!(r, "{:3}:{:02}", sc / 60, sc % 60).unwrap();
                    y += 1;
                }
            };

            for &s0 in &self.scores {
                if let Some(curr) = score
                    && (curr < s0 || y == last_y)
                {
                    draw(7, curr);
                    score = None;
                }
                if Some(s0) == last_score {
                    draw(173, s0);
                    last_score = None;
                } else {
                    draw(176, s0);
                }
            }
            if let Some(s1) = score {
                draw(173, s1);
            }
        }
    }

    fn draw_logo(&self, mut r: Region<'_>) {
        const LOGO_HFB: [u16; 30] = [
            160, 160, 160, 140, 140, 140, 140, 140, 150, 150, 150, 150, 150, 110, 110, 110, 110,
            110, 130, 130, 130, 130, 130, 120, 120, 120, 120, 120, 160, 160,
        ];
        const LOGO: [&str; 9] = [
            r"  ___________________________________      ",
            r" / ____  ____________________________\     ",
            r"/ /   / /                __      __  __    ",
            r"\ \  / / ___  ___  _____/ /_  __/ /_ \ \   ",
            r" \/ / / / _ \/ _ \/ ___/ / / / / __ \ \ \  ",
            r"   / / /  __/  __/ /__/ / /_/ / /_/ /  \ \ ",
            r"   \_\ \___/\___/\___/_/\__,_/_.___/ /_ \ \",
            r"     ___________________________________/ /",
            r"     \___________________________________/ ",
        ];
        let logo_hfb = match self.logo_hfb.as_ref() {
            Some(v) => v.as_slice(),
            None => LOGO_HFB.as_slice(),
        };
        let ox = (r.sx() - 43) / 2;
        let oy = (r.sy() - 9) / 2;
        for (y, s) in LOGO.iter().enumerate() {
            r.at(oy + y as i32, ox);
            for (x, c) in s.chars().enumerate() {
                r.hfb(logo_hfb[(10 + x + y) % 30]);
                r.char(c);
            }
        }
    }

    fn draw_instructions(mut r: Region<'_>) {
        const TEXT: &str = r"

        Keys: [N] New game; [Q] Quit; [1] to [9] Select cards in
        corresponding column; [+]/[=] or [-] Increase or decrease
        number of selected cards; [1] to [9] If the same digit as the
        selecting keypress, move selected cards to top, if another
        digit, move as many of selected cards as possible to that
        column; [Space] Bring a new card down from the stock pile;
        [BackSp] Go back one move; [<]/[>] Adjust display left/right.

        Rules: The aim is to move all the cards to the top area, the
        spaces the the right of the stock pile.  Each top pile must
        consist of one suit only, stacked in order from Ace up to
        King.  The cards in the main area can be moved around:
        sequences of one or more cards of the same suit may be moved
        on top of a card with the next-higher number, of any suit.
        The bulk of the cards are in the stock pile in the top-left.
        Cards may be brought down from there using Space.

        ";
        let lines: Vec<_> = TEXT.trim().split('\n').map(|s| s.to_string()).collect();
        let n_lines = lines.len();
        let oy = (r.sy() - n_lines as i32) / 2;
        for (i, line) in lines.iter().enumerate() {
            r.at(i as i32 + oy, 7);
            let line = line.trim().replace("[", "\0171 ").replace("]", " \0099");
            r.text(&line);
        }
    }

    fn input(&mut self, cx: CX![], key: Key) {
        match key {
            Key::Ctrl('L') => self.redraw(cx),
            Key::Ctrl('C') => stop!(cx),
            Key::Pr('q' | 'Q') => {
                if self.game.is_some() {
                    self.game = None;
                    self.score = None;
                    self.layout(cx);
                } else {
                    stop!(cx);
                }
            }
            Key::Pr('n' | 'N') => {
                self.game = Some(Game::new(
                    cx.now(),
                    ret_some_to!([cx], game_finished() as (Duration)),
                ));
                self.last_score = None;
                self.update_score(cx);
                self.layout(cx);
            }
            Key::Pr('<') => {
                self.adjust -= 1;
                self.save_scores().expect("Failed to save scores");
                self.layout(cx);
            }
            Key::Pr('>') => {
                self.adjust += 1;
                self.save_scores().expect("Failed to save scores");
                self.layout(cx);
            }
            _ => {
                if let Some(game) = &mut self.game {
                    game.input(cx, key);
                }
            }
        }
    }

    fn update_score(&mut self, cx: CX![]) {
        if let Some(game) = &mut self.game {
            let dur = game.score(cx);
            let score = dur.as_secs() as i32;
            let to_wait = if Some(score) != self.score {
                self.score = Some(score);
                self.draw_scores(cx);

                // Update 100ms after next change
                1_000_000_000 - dur.subsec_nanos() + 100_000_000
            } else {
                1_000_000_000
            };
            after!(Duration::from_nanos(to_wait as u64), [cx], update_score());
        }
    }

    fn game_finished(&mut self, cx: CX![], dur: Duration) {
        let score = dur.as_secs() as i32;
        self.scores.push(score);
        self.scores.sort_unstable();
        self.save_scores().expect("Failed to save scores");
        self.last_score = Some(score);
        self.score = None;
        self.game = None;
        self.layout(cx);
    }

    fn score_path() -> PathBuf {
        let mut path = std::env::home_dir().unwrap_or_default();
        path.push(".teeclub-scores");
        path
    }

    fn load_scores(&mut self) -> Option<()> {
        let data = std::fs::read_to_string(Self::score_path()).ok()?;
        let mut it = data.split(char::is_whitespace);
        while let Some(tok) = it.next() {
            match tok {
                "adjust:" => {
                    self.adjust = it.next()?.parse::<i32>().ok()?;
                }
                "scores[" => {
                    self.scores = Vec::new();
                    loop {
                        match it.next()? {
                            "]" => break,
                            v => self.scores.push(v.parse::<i32>().ok()?),
                        }
                    }
                    self.scores.sort_unstable();
                }
                "" => (),
                _ => return None,
            }
        }
        Some(())
    }

    fn save_scores(&mut self) -> std::io::Result<()> {
        let mut out = Vec::new();
        let _ = writeln!(out, "adjust: {}", self.adjust);
        let _ = writeln!(out, "scores[");
        for sc in self.scores.iter().take(200) {
            let _ = writeln!(out, "{sc}");
        }
        let _ = writeln!(out, "]");
        std::fs::write(Self::score_path(), out)
    }
}

/// Game state
#[derive(Eq, PartialEq)]
struct State {
    pile: [Hand; 9],
    stack: [Hand; 9],
}

impl State {
    fn pack(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for p in &self.pile {
            p.pack(&mut out);
        }
        for s in &self.stack {
            s.pack(&mut out);
        }
        out
    }

    fn unpack(mut data: &[u8]) -> Self {
        let mut pile: [Hand; 9] = Default::default();
        let mut stack: [Hand; 9] = Default::default();
        for p in &mut pile {
            *p = Hand::unpack(&mut data);
        }
        for s in &mut stack {
            *s = Hand::unpack(&mut data);
        }
        Self { pile, stack }
    }
}

/// Game state and gameplay handling
struct Game {
    tile: Tile,
    history: Vec<Vec<u8>>,
    state: State,
    select: Option<(usize, usize)>, // (index, count)
    dur: Duration,
    activity: Instant,
    ret: Option<Ret<Duration>>, // Report score for game completion
}

impl Game {
    fn new(start: Instant, ret: Ret<Duration>) -> Self {
        let seed = std::time::SystemTime::UNIX_EPOCH
            .elapsed()
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);

        let mut rand = Rand32::new(seed);
        let mut pile: [Hand; 9] = Default::default();
        let mut stack: [Hand; 9] = Default::default();

        let p0 = &mut pile[0];
        p0.add_deck();
        p0.add_deck();
        p0.shuffle(&mut rand);
        p0.shuffle(&mut rand);

        for _ in 0..5 {
            for s in &mut stack {
                if let Some(card) = p0.pick_last() {
                    s.add(card);
                }
            }
        }

        Self {
            tile: Tile::default(),
            history: Vec::new(),
            state: State { pile, stack },
            select: None,
            dur: Duration::from_secs(0),
            activity: start,
            ret: Some(ret),
        }
    }

    fn select(&mut self, core: &mut Core, i: usize) {
        let s = &self.state.stack[i];
        let len = s.len();
        if len == 0 {
            self.select = None;
        } else {
            let mut count = 1;
            while count < len && s[len - count].inc == Some(s[len - count - 1]) {
                count += 1;
            }
            self.select = Some((i, count));
        }
        self.draw(core);
    }

    fn sel_inc(&mut self, core: &mut Core) {
        if let Some((i, count)) = self.select {
            let s = &self.state.stack[i];
            let len = s.len();
            if count < len && s[len - count].inc == Some(s[len - count - 1]) {
                self.select = Some((i, count + 1));
                self.draw(core);
            }
        }
    }

    fn sel_dec(&mut self, core: &mut Core) {
        if let Some(sel) = &mut self.select {
            let i = sel.0;
            let count = sel.1;
            if count <= 1 {
                self.select = None;
            } else {
                self.select = Some((i, count - 1));
            }
            self.draw(core);
        }
    }

    /// Mark this state by saving it in history, available to undo
    /// later
    fn mark(&mut self) {
        let v = self.state.pack();
        if let Some(last) = self.history.last()
            && *last == v
        {
            return;
        }
        self.history.push(v);
    }

    /// Restore most recent marked state and remove it from history
    fn undo(&mut self, core: &mut Core) {
        if let Some(data) = self.history.pop() {
            self.state = State::unpack(&data);
            self.select = None;
            self.draw(core);
        }
    }

    /// Move selected cards to the given stack or to top if the stack
    /// is the selected stack
    fn move_to(&mut self, core: &mut Core, to: usize) {
        let Some((fr, cnt)) = self.select else {
            return;
        };
        self.select = None;
        self.mark();

        let s = &mut self.state;
        let Some(fr_last) = s.stack[fr].peek_last() else {
            return;
        };

        if fr == to {
            // Move to top
            for pi in (1..9).rev() {
                let do_move = if let Some(to_last) = s.pile[pi].peek_last() {
                    to_last.inc == Some(fr_last)
                } else {
                    fr_last.num == 1
                };
                if do_move {
                    for _ in 0..cnt {
                        if let Some(card) = s.stack[fr].pick_last() {
                            s.pile[pi].add(card);
                        }
                    }
                    break;
                }
            }
            if self.is_finished()
                && let Some(ret) = self.ret.take()
            {
                ret!([ret], self.score(core));
            }
        } else {
            // Move to another pile
            let mut copy_cnt = cnt;
            if let Some(to_last) = s.stack[to].peek_last() {
                copy_cnt = copy_cnt.min(to_last.num.saturating_sub(fr_last.num) as usize);
                if to_last.num != fr_last.num + copy_cnt as u8 {
                    copy_cnt = 0;
                }
            }

            let mut tmp = Vec::new();
            for _ in 0..copy_cnt {
                tmp.push(s.stack[fr].pick_last().unwrap());
            }
            while let Some(card) = tmp.pop() {
                s.stack[to].add(card);
            }
        }
        self.draw(core);
    }

    /// Move card down from stock pile
    fn move_down(&mut self, core: &mut Core) {
        self.mark();
        if let Some(card) = self.state.pile[0].pick_last() {
            self.state.stack[0].add(card);
            self.select = None;
            self.draw(core);
        }
    }

    fn is_finished(&mut self) -> bool {
        for i in 1..9 {
            if self.state.pile[i].len() != 13 {
                return false;
            }
        }
        true
    }

    /// Set the tile to use for drawing, and redraw
    fn layout(&mut self, core: &mut Core, tile: Tile) {
        self.tile = tile;
        self.draw(core);
    }

    /// Draw the playing area
    fn draw(&mut self, core: &mut Core) {
        if let Some(mut r) = self.tile.full(core) {
            r.clear_all_99();

            for col in 0..9 {
                let x = 1 + col as i32 * 8;

                // Pile
                if let Some(card) = self.state.pile[col].peek_last() {
                    if col == 0 {
                        // Facedown pile
                        draw::card_back(&mut r, 0, x);
                    } else {
                        draw::card(&mut r, 0, x, card);
                    }
                } else {
                    draw::card_space(&mut r, 0, x);
                }

                // Index number
                r.hfb(7)
                    .at(CARD_SY, x)
                    .hfb(70)
                    .char((49 + col as u8) as char);

                // Stack
                let y0 = CARD_SY + 1;
                let y1 = r.sy();
                let hand = &self.state.stack[col];
                draw::card_space(&mut r, y0, x);

                let selected = self
                    .select
                    .map(|(index, count)| if col == index { count } else { 0 })
                    .unwrap_or(0);
                let len = hand.len();
                let sel_i = len.saturating_sub(selected);
                let space = y1 - y0 - 1;
                let mut y = y0;
                let mut i = 0;
                if len as i32 + CARD_SY > space {
                    draw::card_dots(&mut r, y, x);
                    y += 1;
                    i = len - (space - CARD_SY) as usize;
                }
                while i < hand.len() {
                    let card = hand[i];
                    if i >= sel_i {
                        draw::card(&mut r, y + 1, x + 1, card);
                    } else {
                        draw::card(&mut r, y, x, card);
                    }
                    i += 1;
                    y += 1;
                }
            }
        }
    }

    /// Handle input
    fn input(&mut self, core: &mut Core, key: Key) {
        self.activity(core);
        match key {
            Key::Pr(dig @ '1'..='9') => {
                let i = (dig as u8 - b'1') as usize;
                if self.select.is_some() {
                    self.move_to(core, i);
                } else {
                    self.select(core, i);
                }
            }
            Key::Pr('+' | '=') => self.sel_inc(core),
            Key::Pr('-') => self.sel_dec(core),
            Key::BackSp => self.undo(core),
            Key::Pr(' ') => self.move_down(core),
            _ => (),
        }
    }

    fn activity(&mut self, core: &mut Core) {
        self.dur = self.score(core);
        self.activity = core.now();
    }

    fn score(&mut self, core: &mut Core) -> Duration {
        let now = core.now();
        let dur = now
            .saturating_duration_since(self.activity)
            .min(Duration::from_secs(5));
        self.dur + dur
    }
}

// Adapted from `oorandom` crate, MIT license
struct Rand32(u64);

impl Rand32 {
    const INC: u64 = 1442695040888963407;
    const MUL: u64 = 6364136223846793005;

    fn new(seed: u64) -> Self {
        let mut this = Self(0);
        let _ = this.get(0..1);
        this.0 = this.0.wrapping_add(seed);
        let _ = this.get(0..1);
        this
    }
}

impl Rand for Rand32 {
    fn get(&mut self, range: std::ops::Range<u32>) -> u32 {
        let state = self.0;
        self.0 = state.wrapping_mul(Self::MUL).wrapping_add(Self::INC);
        let xorshifted = (((state >> 18) ^ state) >> 27) as u32;
        let rot = (state >> 59) as u32;
        let v = xorshifted.rotate_right(rot);
        v % (range.end - range.start) + range.start
    }
}
