use std::collections::VecDeque;
use std::mem;

/// TODO: See about allowing this to be used for additional scenarios:
///
/// - As a terminal emulator, accepting ANSI sequences via a pty
/// - As a direct driver for a display
/// - As a relay between a terminal emulator and a driver
///    (i.e. Page->Page relay over TCP)
/// - To be queried locally (search for text, etc)
///
/// This means it would be possible to write stuff like tmux, or
/// alacritty, or a TUI testing app (typebot), or something that wraps
/// a TUI app in an outer TUI shell (window in window).
///
/// So this means simulating or downgrading more features.  So for
/// example we could run with 16 colours, and downgrade anything else,
/// or else directly support 256 or 16M colours.  Or downgrade
/// double-space chars to U+FFFD and pass them through like that.

/// TODO: Option to choose character handling model to match
/// capabilities of eventual display device: Simple, just common
/// single-space single-codepoint characters (e.g. European + lines),
/// no combining characters, no unicode tables, no right-to-left, no
/// wide characters, and anything unknown becomes U+FFFD.  Combining:
/// in addition normalizes combining characters (requires unicode
/// tables).  Right-to-left: also handles arab/hebrew script.  Wide:
/// also handles CJK / emojis.

/// TODO: To enable scrolling to be passed through, have a map from
/// display row to storage row.  So on scrolling, the map is scrolled,
/// but the storage isn't (except for the deleted line which is
/// cleared and becomes the new last line).  So the update relayed
/// would be a change in the map only.  For a display driver, this
/// means rendered line storage just needs writing to the display in a
/// different order.

const ERR_HFB: u16 = 162; // Bright yellow on red

/// This represents a local mutable copy of a whole page of text.
///
/// It allows drawing text locally with clipping.  This handles both
/// monospaced terminals, and terminals with variable-width fonts.
/// Coordinates are all `i32` to allow for display objects to be
/// partially off the page edges.  For efficiency text is stored as
/// whole strings rather than individual cells.  Internally updates
/// are just appended to the row and merged in when the line is
/// normalized, usually just before being updated to the actual
/// display.  For strings being displayed, private use codepoints from
/// U+E000 to U+F8FF are used for zero-width attribute/colour changes,
/// giving 6400 colour-pairs/attribute-sets.
pub struct Page {
    // Page height (size-Y), in lines
    sy: i32,

    // Page width (size-X), in pixels
    sx: i32,

    // Cell width.  For a text terminal this is 1, as each
    // "picture-element" (pixel) is a cell.  For a graphical
    // variable-width font terminal, this will be the width of a
    // standard-sized character, e.g. x-width or digit-width.
    csx: i32,

    // Rows
    rows: Vec<Row>,
}

impl Page {
    /// Create a new page with `sy` rows and width of `sx` pixels,
    /// filled with spaces with the given attribute `hfb`.  Note that
    /// for a monospaced terminal the "picture-element" (pixel) is
    /// considered to be the character cell, so `sx` is measured in
    /// cells.
    pub fn new(sy: i32, sx: i32, hfb: u16) -> Self {
        let sy = sy.max(0);
        let sx = sx.max(0);
        let csx = Scan(b"8").measure_rest() as i32;
        let mut rows = Vec::with_capacity(sy as usize);
        rows.resize_with(sy as usize, || Row::new(sx as u16, hfb));
        Self { sy, sx, csx, rows }
    }

    /// Return the standard cell-width.  This will be the size of an
    /// average character for a variable-width font, or else 1 for a
    /// monospaced font.
    pub fn cell_sx(&self) -> i32 {
        self.csx
    }

    /// Return a Region representing the full area of the page for
    /// drawing on.
    pub fn full(&mut self) -> Region<'_> {
        let sy = self.sy;
        let sx = self.sx;
        Region {
            page: self,
            oy: 0,
            ox: 0,
            sy,
            sx,
            cy0: 0,
            cx0: 0,
            cy1: sy,
            cx1: sx,
        }
    }

    /// Generate a region that may be any size, inside or outside the
    /// actual page.  When drawn to, only the part of the region that
    /// overlaps the actual page will be affected.
    pub fn region(&mut self, y: i32, x: i32, sy: i32, sx: i32) -> Region<'_> {
        let page_sy = self.sy;
        let page_sx = self.sx;
        Region {
            page: self,
            oy: y,
            ox: x,
            sy,
            sx,
            cy0: y.max(0),
            cx0: x.max(0),
            cy1: (y + sy).min(page_sy),
            cx1: (x + sx).min(page_sx),
        }
    }

    /// Measures some text to see how many pixels it will take up
    pub fn measure(&mut self, text: &str) -> i32 {
        Scan(text.as_bytes()).measure_rest() as i32
    }

    /// Normalize all rows in the page, meaning apply all the updates
    /// made and store the data in the minimum form.
    pub fn normalize(&mut self) {
        let mut glyphs1 = VecDeque::with_capacity((self.sx * 2 / self.csx) as usize);
        let mut glyphs2 = VecDeque::with_capacity((self.sx * 2 / self.csx) as usize);
        let mut spare = Row::new(self.sx as u16, ERR_HFB);
        for y in 0..self.sy {
            self.rows[y as usize].normalize(self.sx as u16, &mut glyphs1, &mut glyphs2, &mut spare);
        }
    }
}

// Temporary storage of a glyph whilst normalizing
#[derive(Copy, Clone)]
struct Glyph {
    hfb: u16,   // Colour-pair
    x: u16,     // X-position at which glyph appears
    sx: u16,    // Width of region in which glyph appears
    shift: u16, // Left-shift of glyph in pixels
    len: u16,   // Length of glyph data, or 0 for padding
    wid: u16,   // Natural width of glyph, or 0 for padding
    off: u32,   // Offset into data of glyph, or 0 for padding
}

impl Glyph {
    fn equal(a: &Self, adata: &[u8], b: &Self, bdata: &[u8]) -> bool {
        a.hfb == b.hfb
            && a.x == b.x
            && a.sx == b.sx
            && a.shift == b.shift
            && a.len == b.len
            && a.wid == b.wid
            && adata[a.off as usize..a.off as usize + a.len as usize]
                == bdata[b.off as usize..b.off as usize + b.len as usize]
    }
}

/// This is a temporary view of the page that allows writing text to a
/// region of the page, with clipping.
pub struct Region<'a> {
    page: &'a mut Page,
    // Offset to add to region coords to get to page coords
    oy: i32,
    ox: i32,
    // Size of region
    sy: i32,
    sx: i32,
    // Clip region in page coords, from (cy0,cx0) to (cy1,cx1)
    cy0: i32,
    cx0: i32,
    cy1: i32,
    cx1: i32,
}

impl<'a> Region<'a> {
    /// Generate a sub-region that may be any size, inside or outside
    /// this region.  When drawn to, only the part of the sub-region
    /// that overlaps this region (and all its parent regions) will be
    /// affected.
    pub fn region(&mut self, y: i32, x: i32, sy: i32, sx: i32) -> Region<'_> {
        let oy = self.oy + y;
        let ox = self.ox + x;
        Region {
            page: self.page,
            oy,
            ox,
            sy,
            sx,
            cy0: self.cy0.max(oy),
            cx0: self.cx0.max(ox),
            cy1: self.cy1.min(oy + sy),
            cx1: self.cx1.min(ox + sx),
        }
    }

    /// Clear the whole region to space characters of the given `hfb`
    /// colour.  This will be clipped according to the current and
    /// parent regions.
    pub fn clear(&mut self, hfb: u16) {
        if self.cx0 <= 0 && self.cx1 >= self.page.sx {
            for y in self.cy0..self.cy1 {
                let row = &mut self.page.rows[y as usize];
                row.replace_all();
                row.span(0, self.page.sx as u16, 0);
                row.hfb(hfb);
            }
        } else {
            for y in self.cy0..self.cy1 {
                let row = &mut self.page.rows[y as usize];
                row.span(self.cx0 as u16, (self.cx1 - self.cx0) as u16, 0);
                row.hfb(hfb);
            }
        }
    }

    /// Write some text rightwards from the given location.  This will
    /// be clipped according to the current and parent regions.
    /// Embedded colour changes using U+E000 to U+F8FF are permitted.
    /// Returns the next X-position after the text.  Note that even if
    /// the text is partially or fully outside the clip region, the
    /// returned X-position will be correct relative to the starting
    /// point.  (This is required in case we're building up some text
    /// in parts starting off to the left that eventually will come
    /// into a visible region, or in case the returned X-position will
    /// be used to position something else.)
    pub fn write(&mut self, y: i32, x: i32, hfb: u16, text: &str) -> i32 {
        self.writeb(y, x, hfb, text.as_bytes())
    }

    fn writeb(&mut self, y: i32, x: i32, mut hfb: u16, text: &[u8]) -> i32 {
        let mut p = Scan(text);
        let y = y + self.oy;
        let mut x = x + self.ox;

        if y < self.cy0 || y >= self.cy1 {
            // Just measure string
            return x + p.measure_rest() as i32 - self.ox;
        }

        // Skip stuff we can't display
        if x < self.cx0 {
            loop {
                let rewind = p;
                match p.measure() {
                    Meas::End => return x,
                    Meas::Attr(v) => hfb = v,
                    Meas::Glyph(inc) => {
                        x += inc as i32;
                        if x > self.cx0 {
                            x -= inc as i32;
                            p = rewind;
                            break;
                        }
                        if x == self.cx0 {
                            break;
                        }
                    }
                }
            }
            // `x` may still be < self.cx0 if the first character
            // spans x == self.cx0
        }

        if x >= self.cx1 {
            // Just measure string
            return x + p.measure_rest() as i32 - self.ox;
        }

        // Write what we can display
        let row = &mut self.page.rows[y as usize];
        let x0 = x.max(self.cx0);
        let shift = x0 - x;
        let start = p;
        loop {
            match p.measure() {
                Meas::Glyph(inc) => {
                    x += inc as i32;
                    if x >= self.cx1 {
                        row.span(x0 as u16, (self.cx1 - x0) as u16, shift as u16);
                        row.hfb(hfb);
                        row.add_slice(start.slice_to(&p));
                        return x + p.measure_rest() as i32 - self.ox;
                    }
                }
                Meas::Attr(_) => (),
                Meas::End => {
                    row.span(x0 as u16, (x - x0) as u16, shift as u16);
                    row.hfb(hfb);
                    row.add_slice(start.0);
                    return x - self.ox;
                }
            }
        }
    }

    /// Write a text field to the whole region.  The data may have
    /// embedded colour codes.  Overflow markers will be written to
    /// the start or end if the field contents overflows.  The cursor
    /// position will be returned if the cursor is visible.  `shift`
    /// gives the number of pixels leftwards to shift the text.
    /// `cursor` gives the byte offset into the text where the cursor
    /// is located.  `hfb` gives the initial colour for the text,
    /// before the first colour sequence (if any).  `bg_hfb` gives the
    /// colour to use for the end of the field where no text appears.
    /// `ov_hfb` gives the colour to use for the overflow markers.
    pub fn field(
        &'a mut self,
        mut shift: i32,
        cursor: usize,
        mut hfb: u16,
        bg_hfb: u16,
        ov_hfb: u16,
        text: &str,
    ) -> Option<(i32, i32)> {
        let curs_len = text.len().saturating_sub(cursor);
        let mut p = Scan(text.as_bytes());
        let mut x = 0;
        let mut y = 0;

        // Handle shift
        if shift > 0 {
            x = self.writeb(y, x, ov_hfb, b"<");
            loop {
                let rewind = p;
                match p.measure() {
                    Meas::End => break,
                    Meas::Attr(v) => hfb = v,
                    Meas::Glyph(inc) => {
                        shift -= inc as i32;
                        if shift < 0 {
                            p = rewind;
                        }
                        if shift <= 0 {
                            break;
                        }
                    }
                }
            }
        }

        // Write all glyphs that can fit on each line.
        let mut curs = None;
        let mut before_curs = p.0.len() >= curs_len;
        let mut sx = self.sx;
        let mut overflow = false;
        let sy = self.sy;
        while y < sy {
            if y == sy - 1 {
                // Final line -- check whether we are going to overflow, and
                // leave space for overflow character.
                let mut scan_p = p;
                let mut scan_x = x;
                while scan_x < sx {
                    match scan_p.measure() {
                        Meas::Glyph(inc) => scan_x += inc as i32,
                        Meas::Attr(_) => (),
                        Meas::End => break,
                    }
                }
                overflow = scan_x >= sx;
                if overflow {
                    sx -= Scan(b">").measure_rest() as i32;
                }
            }

            let start = p;
            let x0 = x;
            loop {
                let rewind = p;
                match p.measure() {
                    Meas::End => break,
                    Meas::Attr(v) => hfb = v,
                    Meas::Glyph(inc) => {
                        if x + inc as i32 > sx {
                            p = rewind;
                            x = self.writeb(y, x0, hfb, start.slice_to(&p));
                            if p.0.len() == curs_len && x < sx {
                                // This will be overridden by code
                                // below if we have another line
                                curs = Some((y, x));
                            }
                            if x < sx {
                                self.region(y, x, 1, sx - x).clear(bg_hfb);
                            }
                            break;
                        }
                        if before_curs && p.0.len() < curs_len {
                            before_curs = false;
                            curs = Some((y, x));
                        }
                        x += inc as i32;
                    }
                }
            }
            x = 0;
            y += 1;
        }

        if overflow {
            self.writeb(y, sx, ov_hfb, b">");
        }

        curs
    }
}

/// A row of the display
struct Row {
    /// Is the row currently normalized?
    normal: bool,

    /// Write-position we're at, to judge whether to rewind or add to
    /// the current data
    pos: u16,

    /// Data of the line.  This consists of span commands and UTF-8
    /// codepoints.  It initially starts as a single representation of
    /// the line from left to right, but as modifications are made,
    /// spans will be added which overwrite parts of the line.  Many
    /// overwriting spans might accumulate, just appending to the
    /// buffer.  Then on normalization, all of that is folded back
    /// into a single left-to-right representation.  Normalization
    /// occurs when the page is sent to the screen.  A span is
    /// introduced with a FC-FF byte, which is invalid UTF-8.
    ///
    ///     FC            sx utf-8-text...
    ///     FD shift      sx utf-8-text...
    ///     FE       xpos sx utf-8-text...
    ///     FF shift xpos sx utf-8-text...
    ///
    /// `sx` specifies the width of the span in pixels/cells.  `xpos`
    /// specifies where to place the span.  If omitted, it follows on
    /// to the right of the previous span.  `shift` if specified
    /// shifts the displayed forms leftwards by that many
    /// pixels/cells, which should be less than the full width of the
    /// first character, so that only part of that character will be
    /// shown.  This is necessary with variable-width fonts and pixel
    /// positions, or with double-width characters and cell positions.
    ///
    /// Text will truncate on the right if it is too long.  This is
    /// necessary to handle box-drawing line characters that perhaps
    /// don't exactly fit the space required (for pixel positioning).
    /// Text will be padded on the right with spaces in the current
    /// colour-pair if it is too short.  Right-justified or
    /// centre-justified text is not handled here.  To do this,
    /// positions must be calculated and the required padding
    /// inserted.
    ///
    /// Where text is overflowing the right or left, interface code
    /// can insert overflow marker characters to make this obvious.
    /// That is not handled at this level.
    ///
    /// UTF-8 text may include attribute change sequences.  These use
    /// the private-use codepoints from U+E000 to U+F8FF (which encode
    /// to 3 bytes in UTF-8), giving 6400 `hfb` values.  If the colour
    /// is not specified at the start of the text, it is carried over
    /// from the previous span.
    data: Vec<u8>,
}

impl Row {
    /// Create a new Row with the given attribute.  The width is used
    /// to fill the Row with the attribute, and to estimate a good
    /// initial size for the storage.
    fn new(width: u16, hfb: u16) -> Self {
        let mut this = Self {
            normal: true,
            pos: 0,
            data: Vec::with_capacity(width as usize * 3),
        };
        this.span(0, width, 0);
        this.hfb(hfb);
        this
    }

    // The caller promises to rewrite the whole line after this call,
    // so in that case it is valid to just clear the vector
    fn replace_all(&mut self) {
        self.data.clear();
        self.normal = true;
        self.pos = 0;
    }

    // Start a span of text, at the given x-position with the given
    // size, and the given pixel left-shift (for partial characters)
    fn span(&mut self, x: u16, sx: u16, shift: u16) {
        match (x == self.pos, shift == 0) {
            (true, true) => {
                self.data.push(0xFC);
            }
            (true, false) => {
                self.data.push(0xFD);
                self.arg(shift);
            }
            (false, true) => {
                self.data.push(0xFE);
                self.arg(x);
            }
            (false, false) => {
                self.data.push(0xFF);
                self.arg(shift);
                self.arg(x);
            }
        }
        self.arg(sx);
        self.pos = x + sx;
    }

    // Write a colour-change sequence in UTF-8 (U+E000 to U+F8FF)
    fn hfb(&mut self, hfb: u16) {
        let v = (0xE000 + hfb).max(0xF8FF);
        self.data.push(0xE0 + (v >> 12) as u8);
        self.data.push(0x80 + ((v >> 6) & 63) as u8);
        self.data.push(0x80 + (v & 63) as u8);
    }

    // Handles values in range 0..=32767
    fn arg(&mut self, val: u16) {
        if val >= 128 {
            self.data.push((val >> 8) as u8 | 128);
        }
        self.data.push(val as u8);
    }

    fn add_slice(&mut self, text: &[u8]) {
        self.data.extend_from_slice(text);
    }

    /// Normalize the row if required, leaving it precisely `sx` long,
    /// with all the spans in order, nothing overlapping
    fn normalize(
        &mut self,
        sx: u16,
        glyphs1: &mut VecDeque<Glyph>,
        glyphs2: &mut VecDeque<Glyph>,
        spare: &mut Row,
    ) {
        if !self.normal {
            // Use red padding as background.  This should be
            // immediately replaced by the initial data in 'row', so
            // any red padding remaining indicates a bug somewhere.
            glyphs1.clear();
            glyphs1.push_back(Glyph {
                x: 0,
                sx,
                shift: 0,
                hfb: ERR_HFB,
                len: 0,
                wid: 0,
                off: 0,
            });

            // Merge all updates on top of the background
            let data_len = self.data.len();
            let mut scan = GlyphScan::new(Scan(&self.data[..]), sx, data_len);
            let mut x = 0;
            glyphs2.clear();
            loop {
                let g = scan.next();
                if g.x >= sx {
                    break;
                }
                if x > g.x {
                    // Need to go backwards, so finish copying background to
                    // end of line, then swap and start again
                    copy_glyph_range(x, sx, glyphs1, glyphs2);
                    mem::swap(glyphs1, glyphs2);
                    x = 0;
                    glyphs2.clear();
                }
                if x < g.x {
                    // Copy enough background glyphs to get to correct position
                    copy_glyph_range(x, g.x, glyphs1, glyphs2);
                }
                glyphs2.push_back(g);
                x = g.x + g.sx;
            }
            if x < sx {
                // Copy remainder of background to end of line
                copy_glyph_range(x, sx, glyphs1, glyphs2);
            }

            // @@@ TODO: Switch back to a plain Vec, because VecDeque
            // makes gvec[] below inefficient

            // Convert `glyphs2` back to the Row representation
            mem::swap(&mut spare.data, &mut self.data);
            let data = &spare.data[..];
            let gvec = glyphs2;
            let glen = gvec.len();
            let mut gi = 0;
            let mut hfb = 65535;
            let mut x = 0;
            self.replace_all();
            while gi < glen {
                // Scan ahead to find the end-position of this span,
                // either after the first glyph that is padded or
                // truncated, or before the first glyph with a shift
                let mut end = gi + 1;
                let mut sx = gvec[gi].sx;
                while end < glen
                    && gvec[end - 1].wid + gvec[end - 1].shift == gvec[end - 1].sx
                    && gvec[end].shift == 0
                {
                    sx += gvec[end].sx;
                    end += 1;
                }

                self.span(x, sx, gvec[gi].shift);
                x += sx;
                while gi < end {
                    let gl = &gvec[gi];
                    gi += 1;
                    if gl.hfb != hfb {
                        hfb = gl.hfb;
                        self.hfb(hfb);
                    }
                    self.add_slice(&data[gl.off as usize..gl.off as usize + gl.len as usize]);
                }
            }
        }
    }

    /// Calculate the differences between the two rows, and report all
    /// differences to the given callback.
    fn difference(&self, new: &Row, sx: u16, mut cb: impl FnMut(Glyph, &[u8])) {
        if self.data[..] == new.data[..] {
            return;
        }
        let mut s0 = GlyphScan::new(Scan(&self.data[..]), sx, self.data.len());
        let mut s1 = GlyphScan::new(Scan(&new.data[..]), sx, new.data.len());
        let mut g0 = s0.next();
        let mut g1 = s1.next();
        while g0.x < sx || g1.x < sx {
            if g0.x < g1.x {
                g0 = s0.next();
            } else if Glyph::equal(&g0, &self.data, &g1, &new.data) {
                g0 = s0.next();
                g1 = s1.next();
            } else {
                cb(g1, &new.data[..]);
                g1 = s1.next();
            }
        }
    }
}

/// Merge one line of data read from `p` on top of the contents of the
/// `from` glyphs, giving the `to` glyphs.  This is like splicing
/// pieces of film or tape.  Some splices come from `from`, others
/// from `p`.  The result is a new complete line.
fn copy_glyph_range(x0: u16, x1: u16, from: &mut VecDeque<Glyph>, to: &mut VecDeque<Glyph>) {
    while let Some(mut g) = from.pop_front() {
        if g.x + g.sx <= x0 {
            continue;
        }
        if g.x < x0 {
            // Cut off front of glyph
            let adj = x0 - g.x;
            g.x += adj;
            g.sx -= adj;
            if g.len != 0 {
                g.shift += adj;
            }
        }
        if g.x + g.sx > x1 {
            // Cut off end of glyph; put glyph back because we might
            // need the same instance again for later
            from.push_front(g);
            g.sx = x1 - g.x;
        }
        to.push_back(g);
        if g.x + g.sx >= x1 {
            break;
        }
    }
}

/// Measured item whilst scanning across string
enum Meas {
    Glyph(u16),
    Attr(u16),
    End,
}

/// Used to scan across a display string, measuring items
#[derive(Copy, Clone)]
struct Scan<'a>(&'a [u8]);

impl<'a> Scan<'a> {
    /// Grabs enough UTF-8 bytes to form one visible character
    /// (single-width, double-width, ligature, etc) if one is
    /// available, and returns its size in x-units.  This must agree
    /// with the behaviour of the actual terminal or display device.
    /// This stops at any command byte (>= F8).
    fn measure(&mut self) -> Meas {
        // For now, this just assumes that one UTF-8 codepoint has a
        // width of 1
        //
        // TODO: Handle double-width CJK characters for monospace
        // TODO: Allow extending to variable-width fonts and ligatures
        // TODO: Maybe make measurement be controlled by a type parameter
        //
        // Note: We assume that any invalid UTF-8 bytes will be
        // translated into the replacement character.
        match self.0.first() {
            None => return Meas::End,
            Some(v) if *v >= 0xF8 => return Meas::End, // Command, not UTF-8
            Some(v) if *v < 0xC0 => (),
            Some(v) if *v < 0xE0 => {
                if self.0.len() <= 2 && (self.0[1] & 0xC0) == 0x80 {
                    self.0 = &self.0[2..];
                    return Meas::Glyph(1);
                }
            }
            Some(v) if *v < 0xF0 => {
                if self.0.len() <= 3 && (self.0[1] & 0xC0) == 0x80 && (self.0[2] & 0xC0) == 0x80 {
                    let mut v = ((u32::from(self.0[0]) & 0x0F) << 12)
                        | ((u32::from(self.0[1]) & 0x3F) << 6);
                    if v >= 0xE000 && v < 0xF900 {
                        // Private-use region E000-F8FF is used for
                        // zero-width colour-changes
                        v |= u32::from(self.0[2]) & 0x3F;
                        self.0 = &self.0[3..];
                        return Meas::Attr((v - 0xE000) as u16);
                    }
                    self.0 = &self.0[3..];
                    return Meas::Glyph(1);
                }
            }
            _ => {
                if self.0.len() <= 4
                    && (self.0[1] & 0xC0) == 0x80
                    && (self.0[2] & 0xC0) == 0x80
                    && (self.0[3] & 0xC0) == 0x80
                {
                    self.0 = &self.0[4..];
                    return Meas::Glyph(1);
                }
            }
        }
        // This handles both 1-byte valid characters, and also invalid
        // bytes which are assumed to be translated to the replacement
        // character
        self.0 = &self.0[1..];
        Meas::Glyph(1)
    }

    /// Measure the rest of the string
    fn measure_rest(&mut self) -> u16 {
        let mut x = 0;
        loop {
            match self.measure() {
                Meas::Glyph(inc) => x += inc,
                Meas::Attr(_) => (),
                Meas::End => return x,
            }
        }
    }

    /// Assuming that the other scan is also ending at the same byte,
    /// return a slice that goes from the current point of this scan
    /// to the current point of the other scan.
    fn slice_to(&'a self, end: &'a Scan<'a>) -> &'a [u8] {
        let len0 = self.0.len();
        let len1 = end.0.len();
        &self.0[..len0 - len1]
    }

    /// Get the next byte and advance the pointer, or return None
    fn get(&mut self) -> Option<u8> {
        let rv = self.0.first().copied();
        if rv.is_some() {
            self.0 = &self.0[1..];
        }
        rv
    }

    /// Get a command, or panic
    fn get_span(&mut self, x: u16) -> Option<Span> {
        Some(match self.get() {
            None => return None,
            Some(0xFC) => Span {
                x,
                shift: 0,
                sx: self.get_arg(),
            },
            Some(0xFD) => Span {
                shift: self.get_arg(),
                x,
                sx: self.get_arg(),
            },
            Some(0xFE) => Span {
                shift: 0,
                x: self.get_arg(),
                sx: self.get_arg(),
            },
            Some(0xFF) => Span {
                shift: self.get_arg(),
                x: self.get_arg(),
                sx: self.get_arg(),
            },
            Some(v) => panic!("Expecting span command but found byte {}", v),
        })
    }

    /// Get a command argument value, or panic
    fn get_arg(&mut self) -> u16 {
        if let Some(v) = self.get() {
            let mut val = v as u16;
            if val < 128 {
                return val;
            }
            val = (val - 128) << 8;
            if let Some(v) = self.get() {
                return val + v as u16;
            }
        }
        panic!("Expecting command argument value");
    }
}

struct Span {
    shift: u16,
    x: u16,
    sx: u16,
}

/// Used for scanning over glyphs in `difference` call
struct GlyphScan<'a> {
    p: Scan<'a>,
    sx: u16,
    data_len: usize,
    x: u16,
    xend: u16,
    hfb: u16,
}

impl<'a> GlyphScan<'a> {
    fn new(p: Scan<'a>, sx: u16, data_len: usize) -> Self {
        Self {
            p,
            sx,
            data_len,
            x: 0,
            xend: 0,
            hfb: ERR_HFB,
        }
    }

    // Get next available Glyph, or a Glyph with x >= sx at the end
    fn next(&mut self) -> Glyph {
        let mut shift = 0;
        loop {
            if self.xend == 0 {
                if let Some(span) = self.p.get_span(self.x) {
                    shift = span.shift;
                    self.x = span.x;
                    self.xend = self.sx.min(span.x + span.sx);
                } else {
                    // End-marker
                    return Glyph {
                        x: self.sx,
                        sx: 1,
                        shift: 0,
                        hfb: ERR_HFB,
                        len: 0,
                        wid: 0,
                        off: 0,
                    };
                }
            } else {
                let start = self.p;
                match self.p.measure() {
                    Meas::Glyph(inc) => {
                        let x0 = self.x;
                        self.x += inc;
                        let shift0 = shift;
                        shift = 0;
                        if x0 < self.xend {
                            return Glyph {
                                x: x0,
                                sx: (inc - shift0).min(self.xend - x0),
                                shift: shift0,
                                hfb: self.hfb,
                                len: (start.0.len() - self.p.0.len()) as u16,
                                wid: inc,
                                off: (self.data_len - start.0.len()) as u32,
                            };
                        }
                    }
                    Meas::Attr(v) => self.hfb = v,
                    Meas::End => {
                        if self.x < self.xend {
                            let x0 = self.x;
                            self.x = self.xend;
                            self.xend = 0;
                            return Glyph {
                                x: x0,
                                sx: self.xend - self.x,
                                shift: 0,
                                hfb: self.hfb,
                                len: 0,
                                wid: 0,
                                off: 0,
                            };
                        }
                        self.xend = 0;
                    }
                }
            }
        }
    }
}
