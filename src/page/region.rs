use crate::{Page, sizer::Sizer};
use std::io::{Result, Write};

/// A temporary view of a logical region of a [`Page`] that allows
/// writing text
///
/// The region is logical in the sense that it may be partly or fully
/// outside of the actual boundaries of the page, or of any region it
/// was derived from.  All data written is only written if it is
/// within the clip.  The clip is the smallest of the page, the region
/// and all the parent regions.  However on writing, the logical write
/// position is advanced as if the characters were written in any
/// case.
///
/// This acts like a mini terminal in that it remembers its current
/// `hfb` colour and write position, and lets you change them, and
/// write text to the 'terminal'.  Text will character-wrap at the
/// region's logical right boundary back to the region's logical left
/// boundary.  This also implements the `Write` trait so it can be
/// used for formatted output with `write!(region, ...)`.
///
/// However note that the region doesn't scroll like a real terminal
/// when you reach the bottom.  (It can't since the page doesn't
/// necessarily store the whole region's contents.)
///
/// Any sequences of the form `\0ZZZ` written to the region represent
/// an HFB colour and change the current HFB colour.  Generate one of
/// these sequences using a string escape sequence (e.g. `"\0099"` to
/// set default fg/bg), or else by using the [`HFB`] type which allows
/// encoding any 16-bit HFB value.
///
/// [`HFB`]: struct.HFB.html
/// [`Page`]: struct.Page.html
pub struct Region<'a> {
    pub(super) page: &'a mut Page,
    // Offset to add to region coords to get to page coords
    pub(super) oy: i32,
    pub(super) ox: i32,
    // Size of region
    pub(super) sy: i32,
    pub(super) sx: i32,
    // Clip region in page coords, from (cy0,cx0) to (cy1,cx1)
    pub(super) cy0: i32,
    pub(super) cx0: i32,
    pub(super) cy1: i32,
    pub(super) cx1: i32,
    // Write position relative to region top-left, and current HFB
    pub(super) wy: i32,
    pub(super) wx: i32,
    pub(super) hfb: u16,
}

impl Region<'_> {
    /// Generate a child region the same size as this one.  This is
    /// like a clone of the region except that it borrows from the
    /// parent region whilst it exists.
    #[inline]
    pub fn full(&mut self) -> Region<'_> {
        Region {
            page: self.page,
            oy: self.oy,
            ox: self.ox,
            sy: self.sy,
            sx: self.sx,
            cy0: self.cy0,
            cx0: self.cx0,
            cy1: self.cy1,
            cx1: self.cx1,
            wy: 0,
            wx: 0,
            hfb: 99,
        }
    }

    /// Generate a child region that may be any size, inside or
    /// outside this region.  When drawn to, only the part of the
    /// child region that overlaps this region (and all its parent
    /// regions) will be affected.
    #[inline]
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
            wy: 0,
            wx: 0,
            hfb: 99,
        }
    }

    /// Replace this region with a child region that may be any size,
    /// inside or outside the old region.  When drawn to, only the
    /// part of the new region that overlaps the old region (and all
    /// its parent regions) will be affected.
    #[inline]
    pub fn region_inplace(&mut self, y: i32, x: i32, sy: i32, sx: i32) {
        let oy = self.oy + y;
        let ox = self.ox + x;
        self.oy = oy;
        self.ox = ox;
        self.sy = sy;
        self.sx = sx;
        self.cy0 = self.cy0.max(oy);
        self.cx0 = self.cx0.max(ox);
        self.cy1 = self.cy1.min(oy + sy);
        self.cx1 = self.cx1.min(ox + sx);
        self.wy = 0;
        self.wx = 0;
        self.hfb = 99;
    }

    /// Get the active [`Sizer`]
    ///
    /// [`Sizer`]: sizer/struct.Sizer.html
    pub fn sizer(&self) -> Sizer {
        self.page.sizer.clone()
    }

    /// Get the region size
    pub fn size(&self) -> (i32, i32) {
        (self.sy, self.sx)
    }

    /// Get the region size-Y, i.e. rows
    pub fn sy(&self) -> i32 {
        self.sy
    }

    /// Get the region size-X, i.e. columns
    pub fn sx(&self) -> i32 {
        self.sx
    }

    /// Get the current write row (Y-position)
    pub fn get_y(&self) -> i32 {
        self.wy
    }

    /// Get the current write column (X-position)
    pub fn get_x(&self) -> i32 {
        self.wx
    }

    /// Check whether any part of this region is visible on the page
    pub fn is_visible(&self) -> bool {
        self.cy0 < self.cy1 && self.cx0 < self.cx1
    }

    /// Change the current HFB
    pub fn hfb(&mut self, hfb: u16) -> &mut Self {
        self.hfb = hfb;
        self
    }

    /// Change the write position to the given position relative to
    /// region top-left.
    pub fn at(&mut self, wy: i32, wx: i32) -> &mut Self {
        self.wy = wy;
        self.wx = wx;
        self
    }

    /// Add a `char` to the region.  This should represent a single
    /// glyph.  Glyphs using combining characters can't be added this
    /// way.
    pub fn char(&mut self, ch: char) -> &mut Self {
        let mut buf = [0; 4];
        self.writeb(ch.encode_utf8(&mut buf).as_bytes());
        self
    }

    /// Add N spaces to the region.
    pub fn spaces(&mut self, n: i32) -> &mut Self {
        let buf = [b' '; 1];
        for _ in 0..n {
            self.writeb(&buf[..1]);
        }
        self
    }

    /// Add the Unicode replacement character U+FFFD
    pub fn repl_char(&mut self) -> &mut Self {
        self.writeb("\u{FFFD}".as_bytes());
        self
    }

    /// Move write position to the region top-left
    pub fn origin(&mut self) -> &mut Self {
        self.wy = 0;
        self.wx = 0;
        self
    }

    /// Skip the write position N cells forwards without overwriting
    /// any characters
    pub fn skip(&mut self, n: i32) -> &mut Self {
        self.wx += n;
        while self.wx > self.sx {
            self.wx -= self.sx;
            self.wy += 1;
        }
        self
    }

    /// Move write position to the start of the next row.  Note that
    /// writing a full line leaves the write position just past the
    /// end of the line, but not on the next line, so calling this
    /// method correctly moves to the next line in that case.
    pub fn newline(&mut self) -> &mut Self {
        self.wy += 1;
        self.wx = 0;
        self
    }

    /// Clear the whole region to the default colour-pair (99).  The
    /// write position is set to top-left, and the current colour is
    /// set to 99.
    pub fn clear_all_99(&mut self) -> &mut Self {
        self.hfb = 99;
        self.clear_all()
    }

    /// Clear the whole region to space characters of the current
    /// `hfb` colour-pair.  This will be clipped according to the
    /// current and parent regions.  The write position is set to
    /// top-left, and the current colour is set to `hfb`.
    pub fn clear_all(&mut self) -> &mut Self {
        for y in self.cy0..self.cy1 {
            let row = &mut self.page.rows[y as usize];
            row.setn(self.cx0, self.cx1, self.hfb, b" ".as_slice());
        }
        self.wy = 0;
        self.wx = 0;
        self
    }

    /// Clear to end-of-line with the default colour-pair.  Leaves the
    /// current colour-pair set to 99.
    pub fn clear_eol_99(&mut self) -> &mut Self {
        self.hfb = 99;
        self.clear_eol()
    }

    /// Clear to end-of-line with the current colour-pair.
    pub fn clear_eol(&mut self) -> &mut Self {
        self.spaces(self.sx - self.wx)
    }

    /// Write some text at the current location with the current HFB,
    /// wrapping to the start of the next line at each row-end of the
    /// region.  What is actually output to the page will be clipped
    /// according to all current and parent regions.  Interprets
    /// `\0ZZZ` colour sequences to change the current HFB.
    /// Interprets `\n`.
    ///
    /// Note that even if the text is partially or fully outside the
    /// clip region, the write position will still be advanced
    /// correctly.
    #[inline]
    pub fn text(&mut self, text: &str) -> &mut Self {
        self.writeb(text.as_bytes());
        self
    }

    /// Write some text (expressed as bytes of UTF-8) at the current
    /// location with the current HFB, wrapping to the start of the
    /// next line at each row-end of the region.  What is actually
    /// output to the page will be clipped according to all current
    /// and parent regions.  Interprets `\0ZZZ` colour sequences to
    /// change the current HFB.  Interprets `\n`.
    ///
    /// Note that even if the text is partially or fully outside the
    /// clip region, the write position will still be advanced
    /// correctly.
    #[inline]
    pub fn bytes(&mut self, text: &[u8]) -> &mut Self {
        self.writeb(text);
        self
    }

    /// Get the contents of the cell at the given location relative to
    /// the region's top-left.  Returns `None` if the location is
    /// outside the region's clip.  Otherwise returns `Some((hfb,
    /// data))` where `data` is the UTF-8 data of the cell.  Note that
    /// in the case of a double-width character, the data will be
    /// prefixed with an 0xFF byte in the left cell, and will contain
    /// an 0xFE byte alone in the right cell.
    pub fn get(&mut self, y: i32, x: i32) -> Option<(u16, &[u8])> {
        let y = y + self.cy0;
        let x = x + self.cx0;
        if y < self.cx0 || y >= self.cy1 || x < self.cx0 || x >= self.cx1 {
            return None;
        }
        self.page.rows[y as usize].get(x)
    }

    /// Set the HFB colour of a cell without affecting the glyph.
    /// Location is relative to region top-left.
    pub fn set_hfb(&mut self, y: i32, x: i32, hfb: u16) {
        let y = y + self.cy0;
        let x = x + self.cx0;
        if y >= self.cx0 && y < self.cy1 && x >= self.cx0 && x < self.cx1 {
            self.page.rows[y as usize].set_hfb(x, hfb);
        }
    }

    /// Set the HFB colour and UTF-8 glyph data of a cell.  Location
    /// is relative to region top-left.  For double-width glyphs, the
    /// left cell should contain byte 0xFF plus the UTF-8, and the
    /// right cell should contain only an 0xFE byte.  If one or other
    /// half is missing, the incorrect cells will show as the
    /// replacement character.
    pub fn set(&mut self, y: i32, x: i32, hfb: u16, data: &[u8]) {
        let y = y + self.cy0;
        let x = x + self.cx0;
        if y >= self.cy0 && y < self.cy1 && x >= self.cx0 && x < self.cx1 {
            self.page.rows[y as usize].set(x, hfb, data);
        }
    }

    /// Plot or unplot a point using Braille glyphs.  Braille glyphs
    /// contain 2 points across and 4 down.  They can be used for
    /// lo-res graphics.  The total grid available to be addressed by
    /// (py, px) is `4*sy` high and `2*sx` wide.  The only restriction
    /// is that the HFB colour-pair cannot be set independently for
    /// each point.  Those apply at the glyph level.  On plotting, if
    /// the point being plotted belongs to a non-Braille glyph, then
    /// that glyph is overwritten.  If it is a Braille glyph then the
    /// point is added to it (`set == true`) or removed from it (`set
    /// == false`).
    pub fn braille_plot(&mut self, py: i32, px: i32, hfb: u16, set: bool) {
        let y = (py >> 2) + self.cy0;
        let x = (px >> 1) + self.cx0;
        if y >= self.cy0 && y < self.cy1 && x >= self.cx0 && x < self.cx1 {
            let (b1, b2) = match ((py & 3) << 1) + (px & 1) {
                0 => (0x00, 0x01),
                1 => (0x00, 0x08),
                2 => (0x00, 0x02),
                3 => (0x00, 0x10),
                4 => (0x00, 0x04),
                5 => (0x00, 0x20),
                6 => (0x01, 0x00),
                _ => (0x02, 0x00),
            };
            let row = &mut self.page.rows[y as usize];
            let mut glyph = [0xE2, 0xA0, 0x80];
            if let Some((_, s)) = row.get(x)
                && s.len() == 3
                && s[0] == 0xE2
                && (s[1] & 0xFC) == 0xA0
            {
                glyph[1] = s[1];
                glyph[2] = s[2];
            }
            if set {
                glyph[1] |= b1;
                glyph[2] |= b2;
            } else if 0 == ((glyph[1] & b1) | (glyph[2] & b2)) {
                return;
            } else {
                glyph[1] &= !b1;
                glyph[2] &= !b2;
            }
            row.set(x, hfb, &glyph);
        }
    }

    fn writeb(&mut self, text: &[u8]) {
        let mut p = Scan::new(text);
        let mut y = self.wy + self.oy;
        let mut y_in_clip = y >= self.cy0 && y < self.cy1;

        loop {
            let start = p;
            if p.consume_lf() {
                self.wx = 0;
                self.wy += 1;
                if self.wy >= self.sy {
                    return;
                }
                y = self.wy + self.oy;
                y_in_clip = y >= self.cy0 && y < self.cy1;
                continue;
            }
            if p.consume_hfb(&mut self.hfb) {
                continue;
            }
            if let Some((inc, repl)) = p.measure(&self.page.sizer) {
                if self.wx >= self.sx {
                    self.wx = 0;
                    self.wy += 1;
                    if self.wy >= self.sy {
                        return;
                    }
                    y = self.wy + self.oy;
                    y_in_clip = y >= self.cy0 && y < self.cy1;
                }

                let x = self.wx + self.ox;
                if y_in_clip && x >= self.cx0 && x < self.cx1 {
                    let row = &mut self.page.rows[y as usize];
                    if inc == 1 {
                        // Mono glyph, either normal or replacement
                        if !repl {
                            row.set(x, self.hfb, start.slice_to(&p));
                        } else {
                            row.set_repl(x, self.hfb);
                        }
                        self.wx += 1;
                    } else if self.wx + 2 > self.sx {
                        // CJK doesn't fit at end of row: Write a
                        // space here, wrap, rewind and retry glyph at
                        // the start of the next row.
                        p = start;
                        row.set(x, self.hfb, b" ".as_slice());
                        self.wx += 1;
                    } else if x + 1 < self.cx1 {
                        // Unclipped CJK
                        row.set_left(x, self.hfb, start.slice_to(&p));
                        row.set_right(x + 1, self.hfb);
                        self.wx += 2;
                    } else {
                        // CJK split across edge of clip region: Can't
                        // be represented, so write replacement
                        // character
                        row.set_repl(x, self.hfb);
                        self.wx += 2;
                    }
                } else {
                    // First cell is outside clip
                    if inc == 1 {
                        self.wx += 1;
                    } else if self.wx + 2 > self.sx {
                        // CJK doesn't fit at end of row: Wrap, rewind
                        // and try again on the next row.
                        p = start;
                        self.wx += 1;
                    } else if y_in_clip && x + 1 == self.cx0 && x + 1 < self.cx1 {
                        // The second half of the glyph is visible;
                        // show as replacement char
                        self.page.rows[y as usize].set_repl(x + 1, self.hfb);
                        self.wx += 2;
                    } else {
                        self.wx += 2;
                    }
                }
                continue;
            }
            break;
        }
    }

    //    /// Write a text field to the whole region.  Text is
    //    /// character-wrapped if the region is multi-line.  The data may
    //    /// have embedded colour codes (\0ZZZ).  Overflow markers will be
    //    /// written to the start or end if the field contents overflows.
    //    /// The cursor position will be returned if the cursor is visible.
    //    ///
    //    /// `cursor` is the offset of the cursor position within the text.
    //    /// `hfb` gives the initial colour for the text, before the first
    //    /// colour sequence (if any).  `bg_hfb` gives the colour to use
    //    /// for the end of the field where no text appears.  `ov_hfb`
    //    /// gives the colour to use for the overflow markers.
    //    pub fn field(
    //        &'a mut self,
    //        cursor: usize,
    //        mut hfb: u16,
    //        bg_hfb: u16,
    //        ov_hfb: u16,
    //        text: &str,
    //    ) -> Option<(i32, i32)> {
    //        let curs_len = text.len().saturating_sub(cursor);
    //        let mut p = Scan::new(text.as_bytes());
    //        let mut x = 0;
    //        let mut y = 0;
    //
    //        // Handle shift
    //        if shift > 0 {
    //            x = self.writeb(y, x, ov_hfb, b"<");
    //            loop {
    //                let rewind = p;
    //                let rewind_hfb = hfb;
    //                match p.measure_hfb(&mut hfb, &self.page.sizer) {
    //                    None => break,
    //                    Some(inc) => {
    //                        shift -= inc as i32;
    //                        if shift < 0 {
    //                            p = rewind;
    //                            hfb = rewind_hfb;
    //                        }
    //                        if shift <= 0 {
    //                            break;
    //                        }
    //                    }
    //                }
    //            }
    //        }
    //
    //        // Write all glyphs that can fit on each line.
    //        let mut curs = None;
    //        let mut before_curs = p.len() >= curs_len;
    //        let mut sx = self.sx;
    //        let mut overflow = false;
    //        let sy = self.sy;
    //        while y < sy {
    //            if y == sy - 1 {
    //                // Final line -- check whether we are going to overflow, and
    //                // leave space for overflow character.
    //                let mut scan_p = p;
    //                let mut scan_x = x;
    //                while scan_x < sx {
    //                    match scan_p.measure(&self.page.sizer) {
    //                        Some(inc) => scan_x += inc as i32,
    //                        None => break,
    //                    }
    //                }
    //                overflow = scan_x >= sx;
    //                if overflow {
    //                    sx -= Scan::new(b">").measure_rest(&self.page.sizer) as i32;
    //                }
    //            }
    //
    //            let start = p;
    //            let x0 = x;
    //            loop {
    //                let rewind = p;
    //                let rewind_hfb = hfb;
    //                match p.measure_hfb(&mut hfb, &self.page.sizer) {
    //                    None => break,
    //                    Some(inc) => {
    //                        if x + inc as i32 > sx {
    //                            p = rewind;
    //                            hfb = rewind_hfb;
    //                            x = self.writeb(y, x0, hfb, start.slice_to(&p));
    //                            if p.len() == curs_len && x < sx {
    //                                // This will be overridden by code
    //                                // below if we have another line
    //                                curs = Some((y, x));
    //                            }
    //                            if x < sx {
    //                                self.region(y, x, 1, sx - x).clear(bg_hfb);
    //                            }
    //                            break;
    //                        }
    //                        if before_curs && p.len() < curs_len {
    //                            before_curs = false;
    //                            curs = Some((y, x));
    //                        }
    //                        x += inc as i32;
    //                    }
    //                }
    //            }
    //            x = 0;
    //            y += 1;
    //        }
    //
    //        if overflow {
    //            self.writeb(y, sx, ov_hfb, b">");
    //        }
    //
    //        curs
    //    }
}

impl Write for Region<'_> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.writeb(buf);
        Ok(buf.len())
    }

    /// The data is considered already flushed as soon as it is
    /// written to the region.
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl std::fmt::Debug for Region<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Region")
            .field("oy", &self.oy)
            .field("ox", &self.ox)
            .field("sy", &self.sy)
            .field("sx", &self.sx)
            .field("cy0", &self.cy0)
            .field("cx0", &self.cx0)
            .field("cy1", &self.cy1)
            .field("cx1", &self.cx1)
            .field("wy", &self.wy)
            .field("wx", &self.wx)
            .field("hfb", &self.hfb)
            .finish()
    }
}

/// Used to scan across a UTF-8 string, measuring items.  Understands
/// HFB sequences of the form `\0ZZZ` where ZZZ is either three
/// decimal digits, or else a 16-bit number encoded as three
/// characters '@' + 0..63, which together form in big-endian order
/// the 16-bit HFB value: 4-bits, 6-bits, 6-bits.
#[derive(Copy, Clone)]
pub struct Scan<'a> {
    p: &'a [u8],
}

impl<'a> Scan<'a> {
    pub fn new(slice: &'a [u8]) -> Self {
        Scan { p: slice }
    }

    #[inline]
    fn len(&self) -> usize {
        self.p.len()
    }

    /// Skip `skip` bytes at start of slice.  Panics if out of range
    #[inline]
    fn skip(&mut self, skip: usize) {
        self.p = &self.p[skip..];
    }

    /// Try to consume a LF
    fn consume_lf(&mut self) -> bool {
        if !self.p.is_empty() && self.p[0] == b'\n' {
            self.skip(1);
            true
        } else {
            false
        }
    }

    /// Try to consume an `\0ZZZ` sequence
    fn consume_hfb(&mut self, hfb: &mut u16) -> bool {
        if !self.p.is_empty() && self.p[0] == 0 && self.p.len() >= 4 {
            if self.p[1] < b'@' {
                *hfb = (self.p[1] as u16 & 15) * 100
                    + (self.p[2] as u16 & 15) * 10
                    + (self.p[3] as u16 & 15);
            } else {
                *hfb = ((self.p[1] as u16 & 15) << 12)
                    | ((self.p[2] as u16 & 63) << 6)
                    | (self.p[3] as u16 & 63);
            }
            self.skip(4);
            true
        } else {
            false
        }
    }

    /// Measure next glyph from the UTF-8 data (single-width,
    /// double-width, ligature, etc), or return `None` for end.
    /// Returns `(width, repl)` where `width` is 1 or 2, and `repl` is
    /// `true` if a replacement character should be written.  Records
    /// any attribute-set changes in `*hfb`.
    fn measure(&mut self, sizer: &Sizer) -> Option<(u16, bool)> {
        if !self.p.is_empty() {
            let (len, wid) = sizer.measure(self.p);
            self.skip(len);
            Some((wid.unsigned_abs(), wid < 0))
        } else {
            None
        }
    }

    /// Measure the rest of the line, up to the end or the next `\n`.
    /// Skips `\0ZZZ` sequences.
    pub fn measure_rest(&mut self, sizer: &Sizer) -> u16 {
        let mut x = 0;
        while !self.p.is_empty() {
            match self.p[0] {
                0 => {
                    if self.p.len() < 4 {
                        break;
                    }
                    self.skip(4);
                }
                b'\n' => {
                    break;
                }
                _ => {
                    let (len, wid) = sizer.measure(self.p);
                    self.skip(len);
                    x += wid.unsigned_abs();
                }
            }
        }
        x
    }

    /// Assuming that the other scan is also ending at the same byte,
    /// return a slice that goes from the current point of this scan
    /// to the current point of the other scan.
    fn slice_to(&'a self, end: &'a Scan<'a>) -> &'a [u8] {
        let len0 = self.len();
        let len1 = end.len();
        &self.p[..len0 - len1]
    }
}

/// A HFB colour-pair that can be formatted, for use with a [`Region`]
///
/// This is used to insert colour-changes in to strings that are
/// passed to a [`Region`].  The value wrapped by it is either a
/// simple 8-bit colour combination as described in [`Output::hfb`],
/// or to access more colours, an index returned by
/// [`Output::hfb_alloc`].
///
/// For example:
///
///```ignore
/// write!(region, "{} Red {} Green ", HFB(20), HFB(40)).unwrap();
///```
///
/// or
///
///```ignore
/// let s = format!("{} Red {} Green ", HFB(20), HFB(40));
/// region.text(&s);
///```
///
/// The output is 4 bytes in the form `\0ZZZ` where `ZZZ` are three
/// ASCII digits that encode the colour-pair.
///
/// [`Output::hfb_alloc`]: struct.Output.html#method.hfb_alloc
/// [`Output::hfb`]: struct.Output.html#method.hfb
/// [`Region`]: struct.Region.html
#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub struct HFB(pub u16);

impl std::fmt::Display for HFB {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let c0 = char::from_u32(((self.0 as u32) >> 12) + 64).unwrap();
        let c1 = char::from_u32((((self.0 as u32) >> 6) & 63) + 64).unwrap();
        let c2 = char::from_u32((self.0 as u32) + 64).unwrap();
        write!(f, "\0{c0}{c1}{c2}")
    }
}
