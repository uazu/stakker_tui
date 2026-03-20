use crate::{Region, sizer::Sizer};
use std::rc::Rc;

pub mod region;

// TODO(enhancement): See about allowing this to be used for
// additional scenarios:
//
// - As a terminal emulator, accepting ANSI sequences via a pty
// - As a direct driver for a display
// - As a relay between a terminal emulator and a driver
//    (i.e. Page->Page relay over TCP)
// - To be queried locally (search for text, etc)
//
// This means it would be possible to write stuff like tmux, or
// alacritty, or a TUI testing app (typebot), or something that wraps
// a TUI app in an outer TUI shell (window in window).
//
// So this means simulating or downgrading more features.  So for
// example we could run with 16 colours, and downgrade anything else,
// or else directly support 256 or 16M colours.  Or downgrade
// double-space chars to U+FFFD and pass them through like that.

// TODO(enhancement): Enable scrolling to be passed through
//
// Maybe have a map from display row to storage row.  So on scrolling,
// the map is scrolled, but the storage isn't (except for the deleted
// line which is cleared and becomes the new last line).  So the
// update relayed would be a change in the map only.  For a display
// driver, this means rendered line storage just needs writing to the
// display in a different order.

/// Default terminal colours, either white-on-black or black-on-white,
/// according to the terminal type and configuration
const DEFAULT_HFB: u16 = 99;

/// A whole page of text
///
/// This is a local copy of the entire terminal screen where the
/// required display can be built up before sending to the terminal
/// with an update [`TermShare::update`].
///
/// This allows drawing text locally with clipping via the [`Region`]
/// struct.  Coordinates are all `i32` to allow for display objects to
/// be partially off the page edges.  Each row stores cells with a
/// width in bytes according to the largest glyph in that row.  So
/// storage adapts to the data.
///
/// Cloning a `Page` is efficient as the cloned page shares the same
/// row data as the original, and does copy-on-write.
///
/// [`Region`]: struct.Region.html
/// [`TermShare::update`]: struct.TermShare.html#method.update
#[derive(Clone)]
pub struct Page {
    sizer: Sizer,

    // Page height (size-Y), in lines
    sy: i32,

    // Page width (size-X), in cells
    sx: i32,

    // Cursor position if displayed, else `None`
    pub(super) cursor: Option<(i32, i32)>,

    // Rows
    rows: Vec<Row>,
}

impl Page {
    /// Create a new page with `sy` rows and width of `sx` cells,
    /// filled with spaces with the given attribute `hfb`, and with
    /// `sizer` to determine glyph boundaries and widths.
    ///
    /// In general it's easier to create a new page using
    /// [`TermShare::page`].
    ///
    /// [`TermShare::page`]: struct.TermShare.html#method.page
    pub fn new(sy: i32, sx: i32, hfb: u16, sizer: Sizer) -> Self {
        let sy = sy.max(0);
        let sx = sx.max(0);
        let empty = Row::new(hfb, sx);
        let mut rows = Vec::with_capacity(sy as usize);
        rows.resize_with(sy as usize, || empty.clone());
        Self {
            sizer,
            sy,
            sx,
            cursor: None,
            rows,
        }
    }

    /// Create a dummy empty page, of zero size
    pub fn empty() -> Self {
        Self::new(0, 0, DEFAULT_HFB, crate::sizer::SimpleSizer::new())
    }

    /// Width of page in cells
    #[inline]
    pub fn sx(&self) -> i32 {
        self.sx
    }

    /// Height of page in lines
    #[inline]
    pub fn sy(&self) -> i32 {
        self.sy
    }

    /// Return a `Region` representing the full area of the page for
    /// drawing on.
    #[inline]
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
            wy: 0,
            wx: 0,
            hfb: DEFAULT_HFB,
        }
    }

    /// Generate a `Region` that may be any size, inside or outside
    /// the actual page.  When drawn to, only the part of the region
    /// that overlaps the actual page will be affected.
    #[inline]
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
            wy: 0,
            wx: 0,
            hfb: DEFAULT_HFB,
        }
    }

    /// Measures some text to see how many cells it will take up
    /// horizontally.  Skips HFB sequences of the form `\0ZZZ`
    pub fn measure(&mut self, text: &str) -> i32 {
        region::Scan::new(text.as_bytes()).measure_rest(&self.sizer) as i32
    }

    /// Set the cursor position, or hide the cursor if `None` is
    /// passed.  If the cursor position is provided but is off the
    /// page, then the cursor is hidden instead.
    pub fn set_cursor(&mut self, cursor: Option<(i32, i32)>) {
        if let Some((y, x)) = cursor
            && y >= 0
            && y < self.sy
            && x >= 0
            && x < self.sx
        {
            self.cursor = cursor;
        } else {
            self.cursor = None;
        }
    }

    /// Get the cursor position, or `None` if hidden
    pub fn cursor(&self) -> Option<(i32, i32)> {
        self.cursor
    }

    /// Scroll the given region `y0..y1` up the given number of lines.
    /// Empty lines are filled with spaces of the given `hfb` colour.
    /// This is very efficient as it manipulates whole rows.
    pub fn scroll_up(&mut self, y0: i32, y1: i32, count: i32, hfb: u16) {
        let y0 = y0.max(0);
        let y1 = y1.min(self.sy);
        let mov = y1 - y0 - count;
        let count = count.min(y1 - y0);
        if mov > 0 {
            for i in 0..mov {
                self.rows.swap((y0 + i) as usize, (y0 + i + count) as usize);
            }
        }
        let empty = Row::new(hfb, self.sx);
        for i in 0..count {
            self.rows[(y1 - count + i) as usize] = empty.clone();
        }
    }

    /// Scroll the given region `y0..y1` down by the given number of
    /// lines.  Empty lines are filled with spaces of the given `hfb`
    /// colour.  This is very efficient as it manipulates whole rows.
    pub fn scroll_down(&mut self, y0: i32, y1: i32, count: i32, hfb: u16) {
        let y0 = y0.max(0);
        let y1 = y1.min(self.sy);
        let mov = y1 - y0 - count;
        let count = count.min(y1 - y0);
        if mov > 0 {
            for i in 1..=mov {
                self.rows.swap((y1 - i) as usize, (y1 - i - count) as usize);
            }
        }
        let empty = Row::new(hfb, self.sx);
        for i in 0..count {
            self.rows[(y0 + i) as usize] = empty.clone();
        }
    }

    /// Copy lines from one part of a page to another, from line
    /// `ya..ya+sy` to `yb..yb+sy`.  Handles overlapping copies
    /// correctly.  Clips to page size before copying.  Since this
    /// copies whole rows using CoW it is very efficient.
    pub fn copy_lines(&mut self, mut ya: i32, mut yb: i32, mut sy: i32) {
        // Clip top side
        let yadj = 0.max(-ya).max(-yb);
        ya += yadj;
        yb += yadj;
        sy -= yadj;

        // Clip bottom side
        let syadj = 0.max(ya + sy - self.sy).max(yb + sy - self.sy);
        sy -= syadj;

        if sy > 0 {
            if ya >= yb {
                for i in 0..sy {
                    self.rows[(yb + i) as usize] = self.rows[(ya + i) as usize].clone();
                }
            } else {
                for i in 1..=sy {
                    self.rows[(yb + sy - i) as usize] = self.rows[(ya + sy - i) as usize].clone();
                }
            }
        }
    }

    /// Copy one region with size `(sy,sx)` at `(ya,xa)` to another at
    /// `(yb,xb)`, cell by cell.  Handles overlapping regions
    /// correctly.  Clips to page size before copying.
    pub fn copy_region(
        &mut self,
        mut ya: i32,
        mut xa: i32,
        mut yb: i32,
        mut xb: i32,
        mut sy: i32,
        mut sx: i32,
    ) {
        // Clip top side
        let yadj = 0.max(-ya).max(-yb);
        ya += yadj;
        yb += yadj;
        sy -= yadj;

        // Clip bottom side
        let syadj = 0.max(ya + sy - self.sy).max(yb + sy - self.sy);
        sy -= syadj;

        // Clip left side
        let xadj = 0.max(-xa).max(-xb);
        xa += xadj;
        xb += xadj;
        sx -= xadj;

        // Clip right side
        let sxadj = 0.max(xa + sx - self.sx).max(xb + sx - self.sx);
        sx -= sxadj;

        if sx > 0 && sy > 0 {
            let mut buf = Vec::new();
            if ya >= yb {
                for i in 0..sy {
                    self.rows[(ya + i) as usize].get_subrow(xa, xa + sx, &mut buf);
                    self.rows[(yb + i) as usize].set_subrow(xb, &buf);
                }
            } else {
                for i in 1..=sy {
                    self.rows[(ya + sy - i) as usize].get_subrow(xa, xa + sx, &mut buf);
                    self.rows[(yb + sy - i) as usize].set_subrow(xb, &buf);
                }
            }
        }
    }

    /// Calculate the changes required to update the `old` page to
    /// match the `new` page, row by row.  `cb` is called as `cb(y,
    /// bitmap, row)` for each row which needs updating.  The bitmap
    /// has a flag set for each cell that needs updating.  It is
    /// mutable, but any changes made to it are ignored.
    pub(super) fn changes(
        old: &mut Self,
        new: &mut Self,
        mut cb: impl FnMut(i32, &mut [bool], &Row),
    ) {
        let sx = new.sx;
        let sy = new.sy;
        if old.sx != sx || old.sy != sy {
            // Resized, so we have no idea the state of the screen.
            // Force it all to be updated.  However it would be better
            // if this were handled at a higher level, where the
            // screen could be cleared to a known colour, saving on
            // these updates.

            for y in 0..sy {
                let mut bitmap = vec![true; sx as usize];
                let row = new.rows.get_mut(y as usize).unwrap();
                cb(y, &mut bitmap, row);
            }
        } else {
            let mut bitmap = vec![false; sx as usize];
            for y in 0..sy {
                let r0 = &mut old.rows[y as usize];
                let r1 = &mut new.rows[y as usize];
                if r0.changes_bitmap(r1, &mut bitmap) {
                    cb(y, &mut bitmap, r1);
                }
            }
        }
    }
}

impl Default for Page {
    /// Defaults to an empty 0x0-sized page
    fn default() -> Self {
        Self::empty()
    }
}

impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Page sy={} sx={} cursor={:?}:",
            self.sy, self.sx, self.cursor
        )?;
        for row in &self.rows {
            write!(f, "\n  {row:?}")?;
        }
        Ok(())
    }
}

/// A row of the display
///
/// This is cheap to clone, because data is kept behind an `Rc` and is
/// modified using `Rc::make_mut`.
// Row data is made up of a series of cells.  Each cell contains the
// HFB value in 2 bytes followed by the glyph UTF-8 data padded with
// zeros.
//
// There are several special values that can be stored instead of the
// glyph UTF-8:
//
// - (empty) meaning the U+FFFD replacement character
//
// - FF plus UTF-8 data to indicate the left half of a double-width
// character
//
// - FE alone to indicate the right half of a double-width character
//
// If either half of a double-width character is overwritten by
// another glyph, then on output the other half is replaced with
// U+FFFD.  That is why it is necessary to mark them in the data.
//
// Cell lengths are a multiple of 2.  So common values are:
//
// - 4 if all the data is Latin-1 or similar (i.e. HFB + 2 bytes UTF-8)
// - 6 if all the data is common 3-byte CJK (i.e. HFB + FF + 3 bytes of UTF-8)
#[derive(Clone)]
pub(super) struct Row {
    /// Row info; split out to aid borrow checker
    info: RowInfo,

    /// Cell data for row
    cells: Rc<Vec<u8>>,
}

#[derive(Clone)]
struct RowInfo {
    /// Width of row in cells
    width: i32,

    /// Cell storage width in bytes, a multiple of 2, >= 4.  First two
    /// bytes is HFB.  Rest is UTF-8, padded with zeros.
    cell_len: usize,
}

impl Row {
    /// Create a new Row, initially filled with padding of the given
    /// HFB and width.
    fn new(hfb: u16, width: i32) -> Self {
        let width = width.max(0);
        let mut data = Vec::with_capacity(4 * width as usize);
        for _ in 0..width {
            data.push((hfb >> 8) as u8);
            data.push(hfb as u8);
            data.push(32);
            data.push(0);
        }
        Self {
            info: RowInfo { width, cell_len: 4 },
            cells: Rc::new(data),
        }
    }

    /// Change the colour of a single cell.  Clips to row width.
    pub fn set_hfb(&mut self, x: i32, hfb: u16) {
        if x >= 0 && x < self.info.width {
            let off = x as usize * self.info.cell_len;
            let cells = Rc::make_mut(&mut self.cells);
            let p = &mut cells[off..off + 2];
            p[0] = (hfb >> 8) as u8;
            p[1] = hfb as u8;
        }
    }

    /// Write a glyph to a single cell.  Clips to row width.
    pub fn set(&mut self, x: i32, hfb: u16, data: &[u8]) {
        if x >= 0 && x < self.info.width {
            let cells = Rc::make_mut(&mut self.cells);
            self.info.make_space(cells, data.len());
            self.info.qset(cells, x, hfb, data);
        }
    }

    /// Write the left-hand part of a double-width glyph to a cell.
    /// Clips to row width.
    pub fn set_left(&mut self, x: i32, hfb: u16, data: &[u8]) {
        if x >= 0 && x < self.info.width {
            let cells = Rc::make_mut(&mut self.cells);
            self.info.make_space(cells, data.len() + 1); // +1 for \xFF
            let off = x as usize * self.info.cell_len;
            let p = &mut cells[off..off + self.info.cell_len];
            let data_len = data.len();
            p[0] = (hfb >> 8) as u8;
            p[1] = hfb as u8;
            p[2] = 0xFF;
            p[3..3 + data_len].copy_from_slice(data);
            p[3 + data_len..].fill(0);
        }
    }

    /// Write the right-hand part of a double-width glyph to a cell.
    /// Clips to row width.
    pub fn set_right(&mut self, x: i32, hfb: u16) {
        if x >= 0 && x < self.info.width {
            let cells = Rc::make_mut(&mut self.cells);
            self.info.qset(cells, x, hfb, b"\xFE".as_slice());
        }
    }

    /// Write the replacement character to the given cell.  Actually
    /// this is stored as an empty string.  Clips to row width.
    pub fn set_repl(&mut self, x: i32, hfb: u16) {
        if x >= 0 && x < self.info.width {
            let cells = Rc::make_mut(&mut self.cells);
            self.info.qset(cells, x, hfb, b"".as_slice());
        }
    }

    /// Write the same glyph to several cells in a row.  Clips to row
    /// width.
    pub fn setn(&mut self, x0: i32, x1: i32, hfb: u16, data: &[u8]) {
        let x0 = x0.max(0);
        let x1 = x1.min(self.info.width);
        if x0 < x1 {
            let cells = Rc::make_mut(&mut self.cells);
            self.info.make_space(cells, data.len());
            for x in x0..x1 {
                self.info.qset(cells, x, hfb, data);
            }
        }
    }

    /// Write data obtained using `get_subrow` rightwards from the
    /// given `x` position, which may be off the edge.
    pub fn set_subrow(&mut self, x: i32, data: &[u8]) {
        let mut x = x;
        let mut p = data;
        while p.len() >= 3 {
            let hfb = ((p[0] as u16) << 8) | (p[1] as u16);
            p = &p[2..];
            let len = p.len();
            let end = p.iter().position(|&c| c == 0).unwrap_or(len);
            self.set(x, hfb, &p[..end]);
            p = &p[(end + 1).min(len)..];
            x += 1;
        }
    }

    /// Get the data in a single cell
    pub fn get(&self, x: i32) -> Option<(u16, &[u8])> {
        if x >= 0 && x < self.info.width {
            return Some(self.qget(x));
        }
        None
    }

    /// Get the data in a horizontal group of cells.  `x0` and `x1`
    /// are clipped to the actual width of the row.  Saves the cells
    /// to `buf` in an internal format that is usable for
    /// `set_subrow()`.
    pub fn get_subrow(&self, x0: i32, x1: i32, buf: &mut Vec<u8>) {
        let x0 = x0.max(0);
        let x1 = x1.min(self.info.width);
        buf.clear();
        for x in x0..x1 {
            let (hfb, data) = self.qget(x);
            buf.push((hfb >> 8) as u8);
            buf.push(hfb as u8);
            buf.extend_from_slice(data);
            buf.push(0);
        }
    }

    /// Get the data in a single cell, assuming that `x` is within
    /// range (or else it will panic).
    fn qget(&self, x: i32) -> (u16, &[u8]) {
        let off = x as usize * self.info.cell_len;
        let p = &self.cells[off..off + self.info.cell_len];
        let hfb = ((p[0] as u16) << 8) | (p[1] as u16);
        let mut data = &p[2..];

        while let Some((last, rest)) = data.split_last() {
            if *last == 0 {
                data = rest;
            } else {
                break;
            }
        }

        (hfb, data)
    }

    //    /// Calculate the changes between two rows in terms of glyphs, and
    //    /// report them to the given callback.
    //    pub fn changes(&self, new: &Row, mut cb: impl FnMut(i32, u16, &'_ [u8])) {
    //        for x in 0..self.info.width.min(new.info.width) {
    //            let (h0, d0) = self.qget(x);
    //            let (h1, d1) = new.qget(x);
    //            if h0 != h1 || d0 != d1 {
    //                cb(x, h1, d1);
    //            }
    //        }
    //    }

    /// Calculate the changes between two rows, clearing and setting
    /// flags in the given bitmap to indicate the changes.
    pub fn changes_bitmap(&self, new: &Row, bitmap: &mut [bool]) -> bool {
        let mut rv = false;
        for x in 0..self.info.width.min(new.info.width) {
            let (h0, d0) = self.qget(x);
            let (h1, d1) = new.qget(x);
            let flag = h0 != h1 || d0 != d1;
            bitmap[x as usize] = flag;
            rv = rv || flag;
        }
        rv
    }
}

impl RowInfo {
    /// Ensure that there is enough space in `cells` for a glyph with
    /// `data_len` bytes of UTF-8
    #[inline(always)]
    fn make_space(&mut self, cells: &mut Vec<u8>, data_len: usize) {
        if data_len + 2 > self.cell_len {
            self.make_space_aux(cells, data_len);
        }
    }

    #[inline(never)]
    fn make_space_aux(&mut self, cells: &mut Vec<u8>, data_len: usize) {
        let new_cell_len = (((data_len + 2) + 1) | 1) - 1;
        let mut new_cells = Vec::with_capacity(new_cell_len * self.width as usize);
        for cell in cells.chunks_exact(self.cell_len) {
            new_cells.extend_from_slice(cell);
            new_cells.resize(new_cells.len() + new_cell_len - self.cell_len, 0);
        }
        *cells = new_cells;
        self.cell_len = new_cell_len;
    }

    /// Write to a single cell, assuming that space has been made
    /// already (or otherwise it will panic).
    #[inline(always)]
    fn qset(&mut self, cells: &mut [u8], x: i32, hfb: u16, data: &[u8]) {
        let off = x as usize * self.cell_len;
        let p = &mut cells[off..off + self.cell_len];
        let data_len = data.len();
        p[0] = (hfb >> 8) as u8;
        p[1] = hfb as u8;
        p[2..2 + data_len].copy_from_slice(data);
        p[2 + data_len..].fill(0);
    }
}

impl std::fmt::Debug for Row {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Row width={} cell_len={}: ",
            self.info.width, self.info.cell_len
        )?;
        let mut cc = 65535;
        for x in 0..self.info.width {
            if let Some((hfb, s)) = self.get(x) {
                if hfb != cc {
                    write!(f, "({hfb})")?;
                    cc = hfb;
                }
                f.write_str(std::str::from_utf8(s).unwrap_or("?"))?;
            } else {
                break;
            }
        }
        Ok(())
    }
}
