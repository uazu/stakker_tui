use crate::{Page, Region, Tile, sizer::Sizer};
use stakker::{Core, Fwd, Share, fwd};
use std::collections::HashMap;
use std::io::{Result, Write};

/// Shared access for terminal output
///
/// Clone to get more references to the same shared [`Output`] buffer.
///
/// [`Output`]: struct.Output.html
#[derive(Clone)]
pub struct TermShare(Share<Output>);

impl TermShare {
    pub(crate) fn new(share: Share<Output>) -> Self {
        Self(share)
    }

    /// Get access to the output buffer for direct ANSI output
    pub fn output<'a>(&'a self, core: &'a mut Core) -> &'a mut Output {
        self.0.rw(core)
    }

    /// Create a new page with the same size as the terminal.  This
    /// can be sent to the screen using [`TermShare::update`].
    ///
    /// [`TermShare::update`]: struct.TermShare.html#method.update
    pub fn page(&self, core: &mut Core) -> Page {
        self.0.rw(core).new_page()
    }

    /// Update the terminal to match the given [`Page`].  If `redraw`
    /// is false then sends the minimum updates to change the current
    /// terminal contents to match the page.  Otherwise does a full
    /// redraw.
    ///
    /// [`Page`]: struct.Page.html
    pub fn update(&self, core: &mut Core, page: &mut Page, redraw: bool) {
        self.0.rw(core).update_to(page, redraw);
    }

    /// Create a new top-level tile and clear the local page to
    /// default colours (99).  (The local page is the page that backs
    /// all [`Tile`] operations.)  All previous tiles are
    /// automatically invalidated, meaning that it's no longer
    /// possible to write to the terminal through them.  After calling
    /// this, the caller should redraw static text and break up the
    /// tile into subtiles as necessary and pass the new tiles to the
    /// code that needs them in order to redraw them and to allow them
    /// to continue to be updated.  See [`Tile`].
    ///
    /// [`Tile`]: struct.Tile.html
    pub fn tile(&self, core: &mut Core) -> Tile {
        let o = self.0.rw(core);
        let (sy, sx) = o.size();
        let genr = o.tile_new_generation();

        Tile {
            share: Some(self.clone()),
            genr,
            oy: 0,
            ox: 0,
            sy,
            sx,
        }
    }
}

/// Output buffer for the terminal
///
/// This just buffers byte data on the way to the terminal.  It allows
/// batching up a whole screen update into a single write, to try to
/// avoid tearing.  This is shared between the [`Terminal`] actor and
/// the actor(s) that will be writing data to the terminal whilst
/// output is enabled via [`TermShare`] shared references.
///
/// Note that coordinates and sizes are passed as `i32` here, because
/// that is more convenient when relative offsets might be negative.
///
/// Calls that add to the buffer return the same `self` value,
/// allowing the calls to be chained.  This implements `Write` so
/// macro calls in the form of `write!(termout, ...)` may be used to
/// do formatted output.  Nothing will be send to the output until the
/// `flush()` call is made.
///
/// Not all ANSI sequences are covered here, just the basic ones
/// required to implement a full-screen application and that are
/// commonly supported everywhere, i.e. the lowest common denominator.
/// If you need another ANSI sequence, it is easy to create, for
/// example `termout.csi().num(5).asc(';').num(10).asc('r')` to set
/// the scroll region to lines 5-10.
///
/// [`TermShare`]: struct.TermShare.html
/// [`Terminal`]: struct.Terminal.html
pub struct Output {
    sizer: Sizer,
    buf: Vec<u8>,
    flush_to: usize,
    features: Features,
    size: (i32, i32),
    fwd_flush: Fwd<()>,
    fwd_lazy_upd: Fwd<()>,
    remote_page: Option<Page>,
    local_page: Option<Page>,
    local_page_update_queued: bool,
    hfb_arr: Vec<String>,          // For HFB values `hfb >= 400`
    hfb_map: HashMap<String, u16>, // Reverse lookup
    pub(crate) tile_generation: u64,
    pub(crate) new_cleanup: Option<Vec<u8>>,
}

impl Output {
    pub(crate) fn new(
        features: Features,
        sizer: Sizer,
        fwd_flush: Fwd<()>,
        fwd_lazy_upd: Fwd<()>,
    ) -> Self {
        Self {
            buf: Vec::new(),
            sizer,
            flush_to: 0,
            features,
            size: (0, 0),
            fwd_flush,
            fwd_lazy_upd,
            remote_page: None,
            local_page: None,
            local_page_update_queued: false,
            hfb_arr: Vec::new(),
            hfb_map: HashMap::new(),
            tile_generation: 0,
            new_cleanup: None,
        }
    }

    /// Get the features supported by the terminal
    #[inline]
    pub fn features(&self) -> &Features {
        &self.features
    }

    /// Get the active [`Sizer`]
    ///
    /// [`Sizer`]: sizer/struct.Sizer.html
    #[inline]
    pub fn sizer(&self) -> Sizer {
        self.sizer.clone()
    }

    /// Get current terminal size: (rows, columns)
    #[inline]
    pub fn size(&self) -> (i32, i32) {
        self.size
    }

    /// Get current terminal size-Y, i.e. rows
    #[inline]
    pub fn sy(&self) -> i32 {
        self.size.0
    }

    /// Get current terminal size-X, i.e. columns
    #[inline]
    pub fn sx(&self) -> i32 {
        self.size.1
    }

    /// Mark all the data from the start of the buffer to the current
    /// end of the buffer as ready for flushing and send a message to
    /// the [`Terminal`] to do the flush.  Any data added after this
    /// call won't be flushed unless another call to this method is
    /// made.  This gives precise control of how much data is flushed
    /// despite flushing being an asynchronous operation.
    ///
    /// [`Terminal`]: struct.Terminal.html
    #[inline]
    pub fn flush(&mut self) {
        self.flush_to = self.buf.len();
        fwd!([self.fwd_flush]);
    }

    /// Add a chunk of UTF-8 string data to the output buffer.
    ///
    /// See also the `Write` implementation, which allows use of
    /// `write!` and `writeln!` to add data to the buffer.
    #[inline]
    pub fn text(&mut self, data: &str) -> &mut Self {
        self.buf.extend_from_slice(data.as_bytes());
        self
    }

    /// Add a chunk of byte data to the output buffer.
    ///
    /// See also the `Write` implementation, which allows use of
    /// `write!` and `writeln!` to add data to the buffer.
    #[inline]
    pub fn bytes(&mut self, data: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(data);
        self
    }

    /// Add a `char` to the output buffer as 1-4 bytes of UTF-8
    #[inline]
    pub fn char(&mut self, ch: char) -> &mut Self {
        let mut buf = [0; 4];
        self.buf
            .extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        self
    }

    /// Add a single byte to the output buffer.
    #[inline]
    pub fn byt(&mut self, v1: u8) -> &mut Self {
        self.buf.push(v1);
        self
    }

    /// Add a single ASCII byte to the output buffer.
    #[inline]
    pub fn asc(&mut self, c: char) -> &mut Self {
        self.buf.push(c as u8);
        self
    }

    /// Add N spaces.  The spaces are written with the current
    /// colour-pair.  This is the only portable way to write areas of
    /// coloured empty cells.
    #[inline]
    pub fn spaces(&mut self, n: i32) -> &mut Self {
        // Note that we can't use `CSI @` because that doesn't advance
        // the cursor, and in some terminals doesn't use the current
        // colours either.
        for _ in 0..n {
            self.asc(' ');
        }
        self
    }

    /// Add an ESC byte (27) + ASCII byte to the output buffer.
    #[inline]
    pub fn esc(&mut self, c: char) -> &mut Self {
        self.buf.push(27);
        self.buf.push(c as u8);
        self
    }

    /// Add `ESC [`, which is the CSI sequence
    #[inline]
    pub fn csi(&mut self) -> &mut Self {
        self.esc('[')
    }

    /// Add the unicode replacement character U+FFFD
    pub fn repl_char(&mut self) -> &mut Self {
        self.text("\u{FFFD}")
    }

    /// Add a 1-3 digit decimal number (0..=999) to the output buffer,
    /// as used in control sequences.  If number is out of range, then
    /// nearest valid number is used.
    pub fn num(&mut self, v: i32) -> &mut Self {
        if v <= 0 {
            self.asc('0');
        } else if v <= 9 {
            self.byt(v as u8 + b'0');
        } else if v <= 99 {
            self.byt((v / 10) as u8 + b'0').byt((v % 10) as u8 + b'0');
        } else if v <= 999 {
            self.byt((v / 100) as u8 + b'0')
                .byt((v / 10 % 10) as u8 + b'0')
                .byt((v % 10) as u8 + b'0');
        } else {
            self.asc('9').asc('9').asc('9');
        }
        self
    }

    /// Add ANSI sequence to move to origin (top-left)
    #[inline]
    pub fn origin(&mut self) -> &mut Self {
        self.text("\x1B[H")
    }

    /// Add ANSI sequence to move cursor to the given coordinates.
    /// Note that coordinates are row-first, with (0,0) as top-left.
    /// Coordinates are taken modulo the screen dimensions, so for
    /// example -1,-1 is bottom-right, and (0, -10) is 10 from the
    /// right on the top line.
    #[inline]
    pub fn at(&mut self, y: i32, x: i32) -> &mut Self {
        let (sy, sx) = self.size;
        self.csi()
            .num(y.rem_euclid(sy) + 1)
            .asc(';')
            .num(x.rem_euclid(sx) + 1)
            .asc('H')
    }

    /// Add ANSI sequence to skip the cursor forwards (rightwards) N
    /// cells, without overwriting them.
    pub fn skip(&mut self, n: i32) -> &mut Self {
        // Use `CSI C` because `CSI a` doesn't work everywhere.  With
        // no argument, `CSI C` skips one character.
        if n >= 1 {
            self.csi();
            if n > 1 {
                self.num(n);
            }
            self.asc('C');
        }
        self
    }

    /// Move cursor to the start of the next row.  Note that writing a
    /// full line leaves the write position just past the end of the
    /// line, but not on the next line, so calling this method
    /// correctly moves to the next line in that case.
    pub fn newline(&mut self) -> &mut Self {
        self.text("\r\n")
    }

    /// Add an attribute string.  The codes passed should be the
    /// semicolon-separated list of numeric codes, for example
    /// "1;31;46".
    #[inline]
    pub fn attr(&mut self, codes: &str) -> &mut Self {
        self.csi().text(codes).asc('m')
    }

    /// Add an attribute string to provide the given HFB attribute-set
    /// expressed as 3 decimal digits `HFB`, or 2 decimal digits `FB`,
    /// or for 256-colours as a value returned by
    /// [`Output::hfb_alloc`].  This is intended for compact
    /// representation of combinations of the basic 8 colours and bold
    /// and/or underline.
    ///
    /// `H` is highlight, used to control bold/underline: 0 normal, 1
    /// bold, 2 underline, 3 bold+underline.  `F` and `B` are
    /// foreground and background in colour-intensity order, 0-9: 0
    /// black, 1 blue, 2 red, 3 magenta, 4 green, 5 cyan, 6 yellow, 7
    /// white, 9 default.  Note that this means that 99 gives the
    /// default colour combination, i.e. white on black on a black
    /// terminal, or black on white on a white terminal.
    ///
    /// [`Output::hfb_alloc`]: struct.Output.html#method.hfb_alloc
    #[inline]
    pub fn hfb(&mut self, hfb: u16) -> &mut Self {
        let mut hfb = hfb;

        if hfb >= 400 {
            if let Some(s) = self.hfb_arr.get(hfb as usize - 400) {
                self.buf.extend_from_slice(s.as_bytes());
                return self;
            }
            hfb = 162; // Yellow on Red to indicate "invalid value"
        }

        const HI: [&str; 4] = ["\x1B[0", "\x1B[0;1", "\x1B[0;4", "\x1B[0;1;4"];
        const FG: [&str; 10] = [
            ";30", ";34", ";31", ";35", ";32", ";36", ";33", ";37", "", "",
        ];
        const BG: [&str; 10] = [
            ";40", ";44", ";41", ";45", ";42", ";46", ";43", ";47", "", "",
        ];
        self.text(HI[((hfb / 100) & 3) as usize])
            .text(FG[(hfb / 10 % 10) as usize])
            .text(BG[(hfb % 10) as usize])
            .asc('m')
    }

    /// Add ANSI sequence to clear the whole display.  This clears
    /// with the terminal's default colour-pair (99), and leaves the
    /// current colours set to the default, and cursor at top-left.
    #[inline]
    pub fn clear_all_99(&mut self) -> &mut Self {
        self.text("\x1B[0m\x1B[2J");
        self.origin()
    }

    /// Add ANSI sequence to clear the whole display.  This clears
    /// with the terminal's default colour-pair (99), and leaves the
    /// current colours set to the default, and cursor at top-left.
    ///
    /// This also clears the internal idea of what the remote page
    /// looks like for the update and tile mechanism, so that updates
    /// operate correctly.  This would typically be called on a resize.
    #[inline]
    pub fn clear_all_99_and_remote(&mut self) -> &mut Self {
        self.remote_page = Some(self.new_page());
        self.text("\x1B[0m\x1B[2J");
        self.origin()
    }

    /// Clear the whole display to the current colour-pair.  This is
    /// done by writing spaces to all the cells on the display, since
    /// it cannot be done portably using ANSI sequences.  Leaves
    /// cursor at top-left.
    #[inline]
    pub fn clear_all(&mut self) -> &mut Self {
        for y in 0..self.sy() {
            self.at(y, 0);
            self.spaces(self.sx());
        }
        self.origin()
    }

    /// Add ANSI sequence to erase to end-of-line.  This erases with
    /// the terminal's default colour-pair (99), and leaves the
    /// current colours set to the default.  Some terminals can erase
    /// with any colour-pair, but this is not univerally supported, so
    /// is not provided here.
    #[inline]
    pub fn clear_eol_99(&mut self) -> &mut Self {
        self.text("\x1B[0m\x1B[K")
    }

    /// Add ANSI sequence to reset attributes to the default
    /// colour-pair (99).  The default colour-pair is white on black
    /// on a black terminal, or black on white on a white terminal.
    #[inline]
    pub fn attr_99(&mut self) -> &mut Self {
        self.text("\x1B[0m")
    }

    /// Add ANSI sequence to do a full reset of the terminal.  This is
    /// `ESC c`.  The screen is erased, and all settings are reset to
    /// defaults.  This includes tab stops, scroll region, character
    /// set, etc.
    #[inline]
    pub fn reset(&mut self) -> &mut Self {
        self.text("\x1Bc")
    }

    /// Add ANSI sequence to switch to underline cursor
    #[inline]
    pub fn cursor_underline(&mut self) -> &mut Self {
        self.text("\x1B[34h")
    }

    /// Add ANSI sequence to switch to block cursor
    #[inline]
    pub fn cursor_block(&mut self) -> &mut Self {
        self.text("\x1B[34l")
    }

    /// Add ANSI sequences to show cursor
    #[inline]
    pub fn cursor_show(&mut self) -> &mut Self {
        self.text("\x1B[?25h\x1B[?0c")
    }

    /// Add ANSI sequences to hide cursor
    #[inline]
    pub fn cursor_hide(&mut self) -> &mut Self {
        self.text("\x1B[?25l\x1B[?1c")
    }

    /// Move cursor to bottom line and do a linefeed.  This results in
    /// the screen scrolling one line, and the cursor being left at
    /// the bottom-left corner, with default colours active.
    #[inline]
    pub fn scroll_up(&mut self) -> &mut Self {
        self.at(-1, 0).attr_99().asc('\n')
    }

    /// Save the current contents of the output buffer as the cleanup
    /// string, then clear the output buffer.  The cleanup string will
    /// be output to the terminal on error or when the terminal is
    /// paused or shutdown.
    ///
    /// This string should reset any settings that have been modified,
    /// and put the cursor somewhere sensible.  The default is `Esc c`
    /// which completely resets the terminal, but usually it's better
    /// to do something less drastic, resetting just the terminal
    /// state that was changed by the app, for example
    /// `termout.attr_99().show_cursor().scroll_up().save_cleanup()`.
    ///
    /// The new cleanup string will be installed on the next flush.
    ///
    /// Note that when the [`Terminal`] actor is dropped, the cleanup
    /// string will be emitted if required.  However if the event loop
    /// has stopped running, then this will not happen.  So in the app
    /// shutdown, it's important that the [`Terminal`] actor is
    /// dropped before the main loop stops running.  See the examples
    /// for configurations that work.
    ///
    /// [`Terminal`]: struct.Terminal.html
    pub fn save_cleanup(&mut self) {
        self.new_cleanup = Some(self.buf.drain(..).collect());
    }

    /// Allocate an HFB value to represent the given attribute-set
    /// (bold, underline, foreground, background), where colours are
    /// expressed using `0xRRGGBB` values.  Chooses nearest colours
    /// out of standard 256-colour palette if available, or else
    /// reverts to 8-colour approximations.  If the colour has been
    /// allocated previously, returns that HFB value.  Colour
    /// allocations last as long as the `Terminal` actor remains
    /// alive.
    pub fn hfb_alloc(&mut self, bold: bool, underline: bool, fg: u32, bg: u32) -> u16 {
        if !self.features.colour_256 {
            let fg = closest_8(fg);
            let bg = closest_8(bg);
            return if bold { 100 } else { 0 } + if underline { 200 } else { 0 } + fg * 10 + bg;
        }

        const HI: [&str; 4] = ["\x1B[0m", "\x1B[0;1m", "\x1B[0;4m", "\x1B[0;1;4m"];
        let hi = HI[if bold { 1 } else { 0 } + if underline { 2 } else { 0 }];
        let cbg = closest_256(bg);
        let cfg = closest_256(fg);
        let s = format!("{hi}\x1B[38;5;{cfg}m\x1B[48;5;{cbg}m");
        if let Some(hfb) = self.hfb_map.get(&s) {
            return *hfb;
        }
        let hfb = (self.hfb_arr.len() + 400) as u16;
        self.hfb_arr.push(s.clone());
        self.hfb_map.insert(s, hfb);
        hfb
    }

    /// Generate an interpolated 24-bit RGB value based on the given
    /// value within the given ordered list of value-to-RGB mappings
    /// `(f64, u32)` which should be in order of value.  If the
    /// provided value is before the first or after the last, the
    /// first/last RGB value is returned instead of an interpolated
    /// value.
    pub fn rgb_interpolate(&self, list: &[(f64, u32)], value: f64) -> u32 {
        let len = list.len();
        match len {
            0 => 0,
            1 => list[0].1,
            _ if value < list[0].0 => list[0].1,
            _ => {
                for w in list.windows(2) {
                    let (pv, prgb) = w[0];
                    let (qv, qrgb) = w[1];
                    if (pv..qv).contains(&value) {
                        let prop = ((value - pv) / (qv - pv) * 4096.0) as i32;
                        let pr = ((prgb >> 16) & 255) as i32;
                        let pg = ((prgb >> 8) & 255) as i32;
                        let pb = (prgb & 255) as i32;
                        let qr = ((qrgb >> 16) & 255) as i32;
                        let qg = ((qrgb >> 8) & 255) as i32;
                        let qb = (qrgb & 255) as i32;
                        let r = ((2048 + prop * (qr - pr)) >> 12) + pr;
                        let g = ((2048 + prop * (qg - pg)) >> 12) + pg;
                        let b = ((2048 + prop * (qb - pb)) >> 12) + pb;
                        return ((r as u32) << 16) + ((g as u32) << 8) + (b as u32);
                    }
                }
                list[len - 1].1
            }
        }
    }

    pub(crate) fn data_to_flush(&self) -> &[u8] {
        &self.buf[..self.flush_to]
    }

    pub(crate) fn drain_flush(&mut self) {
        self.buf.drain(..self.flush_to);
        self.flush_to = 0;
    }

    /// Discard all buffered contents
    pub(crate) fn discard(&mut self) {
        self.buf.drain(..);
        self.flush_to = 0;
    }

    /// Set size
    pub(crate) fn set_size(&mut self, sy: i32, sx: i32) {
        self.size = (sy, sx);
    }

    /// Get a new generation number for tiles and clear the local page
    pub(crate) fn tile_new_generation(&mut self) -> u64 {
        self.local_page = Some(self.new_page());
        self.tile_generation = self.tile_generation.wrapping_add(1);
        self.tile_generation
    }

    /// Get a region referencing the local page and queue a lazy
    /// update
    pub(crate) fn get_local_page(&mut self, genr: u64) -> Option<Region<'_>> {
        if genr == self.tile_generation
            && let Some(ref mut local_page) = self.local_page
        {
            if !self.local_page_update_queued {
                self.local_page_update_queued = true;
                fwd!([self.fwd_lazy_upd]);
            }
            return Some(local_page.full());
        }
        None
    }

    /// Update the terminal screen to match the local page, and clear
    /// "queued" flag
    pub(crate) fn update_to_local_page(&mut self) {
        self.local_page_update_queued = false;
        let mut local_page = self.local_page.take();
        if let Some(ref mut local_page) = local_page {
            self.update_to(local_page, false);
        }
        self.local_page = local_page;
    }

    /// Update the terminal screen to match the given `Page`.  A
    /// minimised update is sent to the terminal (by comparing to the
    /// previous `Page` sent) unless `redraw` is `true` in which case
    /// the terminal is cleared and a full update is sent.
    pub fn update_to(&mut self, page: &mut Page, redraw: bool) {
        if redraw {
            self.remote_page = None;
        }
        if self.remote_page.is_none() {
            self.clear_all_99_and_remote();
        }
        let mut remote_page = self.remote_page.take();
        if let Some(ref mut curr) = remote_page {
            if curr.cursor.is_some() {
                self.cursor_hide();
            }

            let mut curr_hfb = !0;
            let sx = self.sx();
            Page::changes(curr, page, |y, bitmap, row| {
                if let Some(x0) = bitmap.iter().position(|v| *v) {
                    let mut x = x0 as i32;
                    self.at(y, x);
                    while x < sx {
                        if let Some((hfb, data)) = row.get(x) {
                            if hfb != curr_hfb {
                                self.hfb(hfb);
                                curr_hfb = hfb;
                            }
                            match data.first() {
                                None | Some(0xFE) => {
                                    // Replacement char (stored as
                                    // zero-len) or right part of CJK
                                    // without the left part
                                    self.repl_char();
                                    x += 1;
                                }
                                Some(0xFF) => {
                                    if Some(true)
                                        == row.get(x + 1).map(|(_, p)| p.first() == Some(&0xFE))
                                    {
                                        // CJK left part with right part
                                        self.bytes(&data[1..]);
                                        x += 2;
                                    } else {
                                        // CJK left part without right part
                                        self.repl_char();
                                        x += 1;
                                    }
                                }
                                _ => {
                                    self.bytes(data);
                                    x += 1;
                                }
                            }
                        } else {
                            break;
                        }

                        match bitmap[x as usize..].iter().position(|v| *v) {
                            None => break,
                            Some(skip) => {
                                let skip = skip as i32;
                                match skip {
                                    0 => (),
                                    1..=3 => {
                                        let mut join = true;
                                        for i in 0..skip {
                                            if let Some((hfb, data)) = row.get(x + i)
                                                && (curr_hfb != hfb
                                                    || matches!(data.first(), Some(0xFF | 0xFE)))
                                            {
                                                join = false;
                                            }
                                        }
                                        if join {
                                            for i in 0..skip {
                                                bitmap[(x + i) as usize] = true;
                                            }
                                        } else {
                                            self.skip(skip);
                                            x += skip;
                                        }
                                    }
                                    _ => {
                                        self.skip(skip);
                                        x += skip;
                                    }
                                }
                            }
                        }
                    }
                }
            });

            if let Some(ref cursor) = page.cursor {
                self.at(cursor.0, cursor.1).cursor_show();
            }
        }
        self.remote_page = Some(page.clone());
        self.flush();
    }

    /// Create a new blank [`Page`] with the same size as the
    /// terminal, cleared to default foreground/background colours
    /// (HFB 99).
    ///
    /// [`Page`]: struct.Page.html
    pub fn new_page(&self) -> Page {
        Page::new(self.sy(), self.sx(), 99, self.sizer.clone())
    }

    /// Unicode box-drawing characters.  There are four options for
    /// each direction: 0 nothing, 1 normal line, 2 thick line, 3
    /// double line.  The array index combines the four directions
    /// using `(up << 6) + (down << 4) + (left << 2) + right`, giving
    /// 256 combinations.  Where a combination doesn't exist in
    /// unicode (e.g. combined thick and double lines, or some
    /// double-line combinations) a replacement is made by converting
    /// double lines to thick lines.
    ///
    /// See also [`Output::box_from_ascii_art`]
    ///
    /// [`Output::box_from_ascii_art`]: struct.Output.html#method.box_from_ascii_art
    pub const BOX: &'static [char; 256] = &Self::BOX_DATA;

    // BOX_DATA is separate to stop `cargo doc` including all the data
    // in the docs.  Generated by `src/boxes.pl`
    const BOX_DATA: [char; 256] = [
        '\u{0020}', '\u{2576}', '\u{257A}', '\u{257A}', '\u{2574}', '\u{2500}', '\u{257C}',
        '\u{257C}', '\u{2578}', '\u{257E}', '\u{2501}', '\u{2501}', '\u{2578}', '\u{257E}',
        '\u{2501}', '\u{2550}', '\u{2577}', '\u{250C}', '\u{250D}', '\u{2552}', '\u{2510}',
        '\u{252C}', '\u{252E}', '\u{252E}', '\u{2511}', '\u{252D}', '\u{252F}', '\u{252F}',
        '\u{2555}', '\u{252D}', '\u{252F}', '\u{2564}', '\u{257B}', '\u{250E}', '\u{250F}',
        '\u{250F}', '\u{2512}', '\u{2530}', '\u{2532}', '\u{2532}', '\u{2513}', '\u{2531}',
        '\u{2533}', '\u{2533}', '\u{2513}', '\u{2531}', '\u{2533}', '\u{2533}', '\u{257B}',
        '\u{2553}', '\u{250F}', '\u{2554}', '\u{2556}', '\u{2565}', '\u{2532}', '\u{2532}',
        '\u{2513}', '\u{2531}', '\u{2533}', '\u{2533}', '\u{2557}', '\u{2531}', '\u{2533}',
        '\u{2566}', '\u{2575}', '\u{2514}', '\u{2515}', '\u{2558}', '\u{2518}', '\u{2534}',
        '\u{2536}', '\u{2536}', '\u{2519}', '\u{2535}', '\u{2537}', '\u{2537}', '\u{255B}',
        '\u{2535}', '\u{2537}', '\u{2567}', '\u{2502}', '\u{251C}', '\u{251D}', '\u{255E}',
        '\u{2524}', '\u{253C}', '\u{253E}', '\u{253E}', '\u{2525}', '\u{253D}', '\u{253F}',
        '\u{253F}', '\u{2561}', '\u{253D}', '\u{253F}', '\u{256A}', '\u{257D}', '\u{251F}',
        '\u{2522}', '\u{2522}', '\u{2527}', '\u{2541}', '\u{2546}', '\u{2546}', '\u{252A}',
        '\u{2545}', '\u{2548}', '\u{2548}', '\u{252A}', '\u{2545}', '\u{2548}', '\u{2548}',
        '\u{257D}', '\u{251F}', '\u{2522}', '\u{2522}', '\u{2527}', '\u{2541}', '\u{2546}',
        '\u{2546}', '\u{252A}', '\u{2545}', '\u{2548}', '\u{2548}', '\u{252A}', '\u{2545}',
        '\u{2548}', '\u{2548}', '\u{2579}', '\u{2516}', '\u{2517}', '\u{2517}', '\u{251A}',
        '\u{2538}', '\u{253A}', '\u{253A}', '\u{251B}', '\u{2539}', '\u{253B}', '\u{253B}',
        '\u{251B}', '\u{2539}', '\u{253B}', '\u{253B}', '\u{257F}', '\u{251E}', '\u{2521}',
        '\u{2521}', '\u{2526}', '\u{2540}', '\u{2544}', '\u{2544}', '\u{2529}', '\u{2543}',
        '\u{2547}', '\u{2547}', '\u{2529}', '\u{2543}', '\u{2547}', '\u{2547}', '\u{2503}',
        '\u{2520}', '\u{2523}', '\u{2523}', '\u{2528}', '\u{2542}', '\u{254A}', '\u{254A}',
        '\u{252B}', '\u{2549}', '\u{254B}', '\u{254B}', '\u{252B}', '\u{2549}', '\u{254B}',
        '\u{254B}', '\u{2503}', '\u{2520}', '\u{2523}', '\u{2523}', '\u{2528}', '\u{2542}',
        '\u{254A}', '\u{254A}', '\u{252B}', '\u{2549}', '\u{254B}', '\u{254B}', '\u{252B}',
        '\u{2549}', '\u{254B}', '\u{254B}', '\u{2579}', '\u{2559}', '\u{2517}', '\u{255A}',
        '\u{255C}', '\u{2568}', '\u{253A}', '\u{253A}', '\u{251B}', '\u{2539}', '\u{253B}',
        '\u{253B}', '\u{255D}', '\u{2539}', '\u{253B}', '\u{2569}', '\u{257F}', '\u{251E}',
        '\u{2521}', '\u{2521}', '\u{2526}', '\u{2540}', '\u{2544}', '\u{2544}', '\u{2529}',
        '\u{2543}', '\u{2547}', '\u{2547}', '\u{2529}', '\u{2543}', '\u{2547}', '\u{2547}',
        '\u{2503}', '\u{2520}', '\u{2523}', '\u{2523}', '\u{2528}', '\u{2542}', '\u{254A}',
        '\u{254A}', '\u{252B}', '\u{2549}', '\u{254B}', '\u{254B}', '\u{252B}', '\u{2549}',
        '\u{254B}', '\u{254B}', '\u{2551}', '\u{255F}', '\u{2523}', '\u{2560}', '\u{2562}',
        '\u{256B}', '\u{254A}', '\u{254A}', '\u{252B}', '\u{2549}', '\u{254B}', '\u{254B}',
        '\u{2563}', '\u{2549}', '\u{254B}', '\u{256C}',
    ];

    /// Generate a multi-line diagram in Unicode box characters from
    /// simple ASCII-art.  The input is a multi-line diagram using the
    /// following characters:
    ///
    /// - `|` and `-` for single-line vertical and horizontal
    /// - `H` and `=` for double-lined vertical and horizontal
    /// - `$` and `*` for thick single-line vertical and horizontal
    /// - `+` for a cautious joiner, which won't join to itself
    /// - `#` for an enthusiastic joiner, which will join to any joiner
    ///
    /// The joiner uses thick, thin or double parts depending on the
    /// adjacent characters.
    pub fn box_from_ascii_art(inp: &str) -> String {
        let (sx, sy) = inp.split('\n').fold((0, 0), |(sx, sy), line| {
            (sx.max(line.chars().count()), sy + 1)
        });
        let sx = sx + 1; // Makes joining algorithm easier

        // Convert to BOX indices in a rectangular array.  Joiners
        // become 0x100 and 0x200.
        let arrlen = sx * sy;
        let mut arr = vec![0u16; arrlen];
        let mut x = 0;
        let mut y = 0;
        for ch in inp.chars() {
            let boxch = match ch {
                '\n' => {
                    x = 0;
                    y += 1;
                    if y >= sy {
                        break;
                    }
                    continue;
                }
                '|' => 0x50,
                '-' => 0x05,
                'H' => 0xF0,
                '=' => 0x0F,
                '$' => 0xA0,
                '*' => 0x0A,
                '+' => 0x100,
                '#' => 0x200,
                _ => 0,
            };

            arr[sx * y + x] = boxch;
            x += 1;
        }

        // Figure out what character the joiners should be
        for o in 0..sx * sy {
            if matches!(arr[o], 0x100 | 0x200) {
                let cautious = arr[o] == 0x100;

                // Up and left will already have joiners resolved, so
                // just take adjacent line-part.  Going up/left may go
                // negative, so isize
                let mut p = o as isize - sx as isize;
                let up = if p >= 0 {
                    (arr[p as usize] >> 4) & 3
                } else {
                    0
                };
                p = o as isize - 1;
                let left = if p >= 0 { arr[p as usize] & 3 } else { 0 };

                // Down and right might be adjacent to joiners, in
                // which case jump over those joiners to see the
                // line-type on the other side.  Bail out if
                // cautious.  Down/right is positive, so usize
                let mut down = 0;
                let mut p = o + sx;
                while p < arrlen {
                    if arr[p] <= 0x100 {
                        down = (arr[p] >> 6) & 3;
                        break;
                    }
                    if cautious && arr[p] == 0x100 {
                        break;
                    }
                    p += sx;
                }
                let mut right = 0;
                p = o + 1;
                while p < arrlen {
                    if arr[p] <= 0x100 {
                        right = (arr[p] >> 2) & 3;
                        break;
                    }
                    if cautious && arr[p] == 0x100 {
                        break;
                    }
                    p += 1;
                }

                // Now we have UDLR for this joiner
                arr[o] = (up << 6) | (down << 4) | (left << 2) | right;
            }
        }

        let mut out = String::with_capacity(arrlen + sy);
        for y in 0..sy {
            for x in 0..sx {
                out.push(Self::BOX[arr[sx * y + x] as usize]);
            }
            out.truncate(out.trim_end_matches(' ').len());
            out.push('\n');
        }

        out
    }
}

impl Write for Output {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    /// Logically we consider the final destination of the Write trait
    /// to be the buffer.  So this `flush` call does nothing.  In
    /// general we'll want to gather all the updates into one big
    /// flush to avoid tearing on the terminal.
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Features supported by the terminal
pub struct Features {
    /// Supports 256 colours?
    pub colour_256: bool,
}

/// Find the closest colour in the typical 8-colour ANSI palette to
/// the given 0xRRGGBB colour.  Since these colours generally depend
/// on the theme, we can only guess approximately.
fn closest_8(c: u32) -> u16 {
    let r = ((c & 0xFF0000) >> 16) as u16;
    let g = ((c & 0xFF00) >> 8) as u16;
    let b = (c & 0xFF) as u16;

    let ri = if r > 85 { 2 } else { 0 };
    let gi = if g > 85 { 4 } else { 0 };
    let bi = if b > 85 { 1 } else { 0 };
    ri + gi + bi
}

/// Find the closest colour in the standard 256-colour palette to the
/// given 0xRRGGBB colour
fn closest_256(c: u32) -> u16 {
    let r = ((c & 0xFF0000) >> 16) as u16;
    let g = ((c & 0xFF00) >> 8) as u16;
    let b = (c & 0xFF) as u16;

    fn sqr_dist(a: u16, b: u16) -> i32 {
        let d = a as i32 - b as i32;
        d * d
    }

    // Check colour set first, 0-5 in each of R,G,B: 216 colours
    let ri = (r + 25) / 51; // Round to nearest
    let gi = (g + 25) / 51;
    let bi = (b + 25) / 51;
    let ci = 16 + ri * 36 + gi * 6 + bi;
    let ci_dist = sqr_dist(r, ri * 51) + sqr_dist(g, gi * 51) + sqr_dist(b, bi * 51);

    // Check greyscale section, 24 colours
    let av = (r + g + b) / 3;
    let wi = 232 + (((av + 5) * 23) >> 8);
    let wv = wi * 255 / 23;
    let wi_dist = sqr_dist(r, wv) + sqr_dist(g, wv) + sqr_dist(b, wv);

    if ci_dist < wi_dist { ci } else { wi }
}
