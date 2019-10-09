use std::mem;

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
        let csx = Scan(b"8").measure_rest();
        let mut rows = Vec::with_capacity(sy as usize);
        rows.resize_with(sy as usize, || Row::new(sx, hfb));
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
        Scan(text.as_bytes()).measure_rest()
    }

    /// Normalize all rows in the page, meaning apply all the updates
    /// made and store the data in the minimum form.
    pub fn normalize(&mut self) {
        let mut glyphs1 = Vec::with_capacity((self.sx * 2 / self.csx) as usize);
        let mut glyphs2 = Vec::with_capacity((self.sx * 2 / self.csx) as usize);
        for y in 0..self.sy {
            let row = &mut self.rows[y as usize];
            if row.normal {
                continue;
            }

            // Use red padding as background.  This should be
            // immediately replaced by the initial data in 'row', so
            // any red padding remaining indicates a bug somewhere.
            glyphs1.clear();
            glyphs1.push(Glyph {
                x: 0,
                sx: self.sx as u16,
                shift: 0,
                hfb: 2, // H=0 F=0 B=2
                len: 0,
                off: 0,
            });

            // Merge all updates on top of the background
            let data_len = row.data.len();
            let mut p = Scan(&row.data[..]);
            while !p.0.is_empty() {
                p = Self::merge_line(p, self.sx, data_len, &glyphs1, &mut glyphs2);
                mem::swap(&mut glyphs1, &mut glyphs2);
            }

            //@@@ Convert `glyphs1` back to the 'row' representation
        }

        //@@@
    }

    // Merge one line of data read from `p` on top of the contents of
    // the `from` glyphs, giving the `to` glyphs
    fn merge_line<'a>(
        mut p: Scan<'a>,
        sx: i32,
        data_len: usize,
        from: &[Glyph],
        to: &mut Vec<Glyph>,
    ) -> Scan<'a> {
        to.clear();
        let mut x = 0;
        let mut shift = 0;
        let mut fi = 0;
        while x < sx {
            let skip;
            match p.get_cmd() {
                Cmd::Text(cnt, v) => {
                    // Copy text to 'to'
                    let mut hfb = v as u16;
                    let xend = x + cnt;
                    loop {
                        let start = p;
                        match p.measure() {
                            Meas::Glyph(inc) => {
                                if x < xend {
                                    to.push(Glyph {
                                        x,
                                        sx: inc.min(xend - x) as u16,
                                        shift: shift as u16,
                                        hfb,
                                        len: (start.0.len() - p.0.len()) as u16,
                                        off: (data_len - start.0.len()) as u32,
                                    });
                                }
                                x += inc;
                                shift = 0;
                            }
                            Meas::Attr(v) => hfb = v,
                            Meas::End => break,
                        }
                    }
                    if x < xend {
                        to.push(Glyph {
                            x,
                            sx: (xend - x) as u16,
                            shift: 0,
                            hfb,
                            len: 0,
                            off: 0,
                        });
                    }
                    x = xend;
                    shift = 0;
                    continue;
                }
                Cmd::Shift(cnt) => {
                    shift = cnt;
                    continue;
                }
                Cmd::Skip(cnt) => skip = cnt, // Drop down to skip code
                Cmd::End | Cmd::Rewind => skip = sx - x, // Drop down to skip code
            }

            // Do a skip: Copy data from `from`, pixel `x` to `xend`
            let xend = x + skip;
            while x < xend {
                let mut gl = from[fi];
                fi += 1;
                if gl.x + i32::from(gl.sx) <= x {
                    continue;
                }
                if gl.x < x {
                    // Cut off front of glyph
                    let adj = x - gl.x;
                    gl.x += adj;
                    gl.sx -= adj as u16;
                    if gl.len != 0 {
                        gl.shift += adj as u16;
                    }
                }
                if gl.x + i32::from(gl.sx) > xend {
                    // Cut off end of glyph; reverse one glyph
                    // because we might need the same Glyph
                    // instance again for later
                    gl.sx = (xend - gl.x) as u16;
                    fi -= 1;
                }
                x = gl.x + i32::from(gl.sx);
                if gl.sx > 0 {
                    to.push(gl);
                }
            }
        }

        // Pass back Scan for remaining data
        p
    }
}

// Temporary storage of a glyph whilst normalizing
#[derive(Copy, Clone)]
struct Glyph {
    x: i32,     // X-position of region to show glyph
    sx: u16,    // Width of region
    shift: u16, // Left-shift of glyph
    hfb: u16,   // Colour-pair
    len: u16,   // Length of glyph data, or 0 for padding
    off: u32,   // Offset into data of glyph, or 0 for padding
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
    /// colour
    pub fn clear(&mut self, hfb: u16) {
        if self.cx0 <= 0 && self.cx1 >= self.page.sx {
            for y in self.cy0..self.cy1 {
                let row = &mut self.page.rows[y as usize];
                row.replace_all();
                row.text(self.page.sx, hfb);
            }
        } else {
            for y in self.cy0..self.cy1 {
                let row = &mut self.page.rows[y as usize];
                row.moveto(self.cx0);
                row.text(self.cx1 - self.cx0, hfb);
            }
        }
    }

    /// Write some text rightwards from the given location.  This will
    /// be clipped according to the current and parent regions.
    /// Embedded colour changes are permitted.  Returns the next
    /// X-position after the text.  Note that even if the text is
    /// partially or fully outside the clip region, the returned
    /// X-position will be correct relative to the starting point.
    /// (This is required in case we're building up some text in parts
    /// starting off to the left that eventually will come into a
    /// visible region, or in case the returned X-position will be
    /// used to position something else.)
    pub fn write(&mut self, y: i32, x: i32, hfb: u16, text: &str) -> i32 {
        self.writeb(y, x, hfb, text.as_bytes())
    }

    fn writeb(&mut self, y: i32, x: i32, mut hfb: u16, text: &[u8]) -> i32 {
        let mut p = Scan(text);
        let y = y + self.oy;
        let mut x = x + self.ox;

        if y < self.cy0 || y >= self.cy1 {
            // Just measure string
            return x + p.measure_rest() - self.ox;
        }

        // Skip stuff we can't display
        if x < self.cx0 {
            loop {
                let rewind = p;
                match p.measure() {
                    Meas::End => return x,
                    Meas::Attr(v) => hfb = v,
                    Meas::Glyph(inc) => {
                        x += inc;
                        if x > self.cx0 {
                            x -= inc;
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
            return x + p.measure_rest() - self.ox;
        }

        // Write what we can display
        let row = &mut self.page.rows[y as usize];
        let x0 = x.max(self.cx0);
        row.moveto(x0);
        let shift = x0 - x;
        let start = p;
        loop {
            match p.measure() {
                Meas::Glyph(inc) => {
                    x += inc;
                    if x >= self.cx1 {
                        row.shift(shift);
                        row.text(self.cx1 - x0, hfb);
                        row.add_slice(start.slice_to(&p));
                        break;
                    }
                }
                Meas::Attr(_) => (),
                Meas::End => {
                    row.shift(shift);
                    row.text(x - x0, hfb);
                    row.add_slice(start.0);
                    return x - self.ox;
                }
            }
        }

        x + p.measure_rest() - self.ox
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
                        shift -= inc;
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
                        Meas::Glyph(inc) => scan_x += inc,
                        Meas::Attr(_) => (),
                        Meas::End => break,
                    }
                }
                overflow = scan_x >= sx;
                if overflow {
                    sx -= Scan(b">").measure_rest();
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
                        if x + inc > sx {
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
                        x += inc;
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

struct Row {
    // Is the row currently normalized?
    normal: bool,

    // Write-position we're at, to judge whether to rewind or add to
    // the current data
    pos: i32,

    // Data of the line.  This consists of commands and embedded UTF-8
    // codepoints.  It initially starts as a single representation of
    // the line from left to right, but as modifications are made, F8
    // will be added which returns to the left side and specifies text
    // to overwrite the existing text.  Many F8 sections might
    // accumulate, just appending to the buffer.  Then on
    // normalization, all of that is folded back into a single
    // left-to-right representation.  Normalization occurs when the
    // page is sent to the screen, which aids in cache-locality.
    //
    // F8                Return to position 0
    // F9 cnt            Advance 'cnt' pixels, not changing anything
    // FA cnt hfb text   Advance, writing left-justified, right-padded/right-truncated text
    //                   into 'cnt' pixels.  Text follows, up to next F8+ byte.
    // FB shift          Specify left-shift in pixels for following FA sequence
    //
    // 'shift' values shift the string leftwards by that many
    // positions, which should be less than the full width of the
    // first character, so that only part of that character will be
    // shown.  This is necessary with variable-width fonts and pixel
    // positions, or with double-width characters and cell positions.
    //
    // Text will truncate on the right if it is too long.  This is
    // necessary to handle box-drawing line characters that perhaps
    // don't exactly fit the space required (for pixel positioning).
    //
    // Where text is overflowing the right or left, interface code can
    // insert overflow marker characters to make this obvious.  That
    // is not handled at this level.
    //
    // To handle right-justified or centre-justified text, positions
    // must be calculated and the required padding inserted first.
    //
    // UTF-8 text may include attribute change sequences.  These use
    // the private-use codepoints from U+E000 to U+F8FF, giving 6400
    // `hfb` values.
    data: Vec<u8>,
}

impl Row {
    /// Create a new Row with the given attribute.  The width is used
    /// to fill the Row with the attribute, and to estimate a good
    /// initial size for the storage.
    fn new(width: i32, hfb: u16) -> Self {
        let mut this = Self {
            normal: true,
            pos: 0,
            data: Vec::with_capacity(width as usize * 3),
        };
        this.text(width, hfb);
        this
    }

    // The caller promises to rewrite the whole line after this call,
    // so in that case it is valid to just clear the vector
    fn replace_all(&mut self) {
        self.data.clear();
        self.normal = true;
        self.pos = 0;
    }

    // Move the update cursor to the beginning of the line
    fn cr(&mut self) {
        self.data.push(0xF8);
        self.normal = false;
        self.pos = 0;
    }

    // Move to the given position, by doing optional `cr` then
    // optional `skip`.
    fn moveto(&mut self, x0: i32) {
        if x0 != self.pos {
            if x0 < self.pos {
                self.cr();
            }
            if x0 != self.pos {
                self.skip(x0 - self.pos);
            }
        }
    }

    fn skip(&mut self, cnt: i32) {
        self.data.push(0xF9);
        self.arg(cnt);
        self.pos += cnt;
    }

    fn text(&mut self, cnt: i32, hfb: u16) {
        self.data.push(0xFA);
        self.arg(cnt);
        self.arg(i32::from(hfb));
        self.pos += cnt;
    }

    fn shift(&mut self, shift: i32) {
        if shift != 0 {
            self.data.push(0xFB);
            self.arg(shift);
        }
    }

    fn arg(&mut self, val: i32) {
        let mut val = val as u32;
        while val >= 128 {
            self.data.push(128 + (val & 127) as u8);
            val >>= 7;
        }
        self.data.push(val as u8);
    }

    fn add_slice(&mut self, text: &[u8]) {
        self.data.extend_from_slice(text);
    }
}

enum Meas {
    Glyph(i32),
    Attr(u16),
    End,
}

#[derive(Copy, Clone)]
struct Scan<'a>(&'a [u8]);

impl<'a> Scan<'a> {
    // Grabs enough UTF-8 bytes to form one visible character
    // (single-width, double-width, ligature, etc) if one is
    // available, and returns its size in x-units.  This must agree
    // with the behaviour of the actual terminal or display device.
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
    fn measure_rest(&mut self) -> i32 {
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

    // Get a command
    fn get_cmd(&mut self) -> Cmd {
        match self.0.first() {
            None => Cmd::End,
            Some(0xF8) => Cmd::Rewind,
            Some(0xF9) => {
                let cnt = self.get_arg();
                Cmd::Skip(cnt)
            }
            Some(0xFA) => {
                let cnt = self.get_arg();
                let hfb = self.get_arg();
                Cmd::Text(cnt, hfb)
            }
            Some(0xFB) => {
                let cnt = self.get_arg();
                Cmd::Shift(cnt)
            }
            Some(v) => panic!("Expecting command but found byte {}", v),
        }
    }

    // Get a command argument value, or panic
    fn get_arg(&mut self) -> i32 {
        let mut val = 0_i32;
        while let Some(v) = self.0.first() {
            val = (val << 7) + i32::from(v & 127);
            if (v & 128) == 0 {
                return val;
            }
        }
        panic!("Expecting command argument value");
    }
}

enum Cmd {
    End,
    Rewind,
    Skip(i32),
    Text(i32, i32),
    Shift(i32),
}
