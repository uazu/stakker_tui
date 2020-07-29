use std::io::{Result, Write};

/// Output buffer for the terminal
///
/// This just buffers byte data on the way to the terminal.  It allows
/// batching up a whole screen update into a single write, to try to
/// avoid tearing.  This is shared between the [`Terminal`] actor and
/// the actor(s) that will be writing data to the terminal.
///
/// Note that coordinates and sizes are passed as `i32` here, because
/// that is more convenient when relative offsets might be negative.
/// It also allows positions modulo the screen dimensions,
/// i.e. negative positions measuring from the right or bottom edge.
///
/// Calls that add to the buffer return the same `self` value,
/// allowing the calls to be chained.
///
/// Not all ANSI sequences are covered here, just the basic ones
/// require to implement a full-screen application and that are
/// commonly supported everywhere.  If you need another ANSI sequence,
/// it is easy to create, for example
/// `termout.csi().num(5).asc('C')` to do "cursor forward 5 cells".
///
/// [`Terminal`]: struct.Terminal.html
pub struct TermOut {
    buf: Vec<u8>,
    flush_to: usize,
    features: Features,
    size: (i32, i32),
    pub(crate) new_cleanup: Option<Vec<u8>>,
}

impl TermOut {
    pub(crate) fn new(features: Features) -> Self {
        Self {
            buf: Vec::new(),
            flush_to: 0,
            features,
            new_cleanup: None,
            size: (0, 0),
        }
    }

    /// Get the features supported by the terminal
    #[inline]
    pub fn features(&self) -> &Features {
        &self.features
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
    /// end of the buffer as ready for flushing.  However the data
    /// won't be flushed until the [`Terminal`] actor receives a
    /// [`Terminal::flush`] call.  Any data added after this call
    /// won't be flushed unless another call to this method is made.
    ///
    /// [`Terminal::flush`]: struct.Terminal.html#method.flush
    /// [`Terminal`]: struct.Terminal.html
    #[inline]
    pub fn flush(&mut self) {
        self.flush_to = self.buf.len();
    }

    /// Add a chunk of UTF-8 string data to the output buffer.
    ///
    /// See also the `Write` implementation, which allows use of
    /// `write!` and `writeln!` to add data to the buffer.
    #[inline]
    pub fn out(&mut self, data: &str) -> &mut Self {
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

    /// Add an attribute string.  The codes passed should be the
    /// semicolon-separated list of numeric codes, for example
    /// "1;31;46".
    #[inline]
    pub fn attr(&mut self, codes: &str) -> &mut Self {
        self.csi().out(codes).asc('m')
    }

    /// Add an attribute string to provide the given HFB colour
    /// expressed as 3 decimal digits `HFB`, or 2 decimal digits `FB`.
    /// This is intended for compact representation of the basic
    /// colours.  `H` is highlight, used to control bold: 0 normal, 1
    /// bold.  `F` and `B` are foreground and background in
    /// colour-intensity order, 0-9: 0 black, 1 blue, 2 red, 3
    /// magenta, 4 green, 5 cyan, 6 yellow, 7 white, 8/9 default.
    #[inline]
    pub fn hfb(&mut self, hfb: u8) -> &mut Self {
        const FG: [i32; 10] = [30, 34, 31, 35, 32, 36, 33, 37, 39, 39];
        self.out("\x1B[0;");
        if hfb >= 100 {
            self.out("1;");
        }
        self.num(FG[(hfb / 10 % 10) as usize])
            .asc(';')
            .num(10 + FG[(hfb % 10) as usize])
            .asc('m')
    }

    /// Add ANSI sequence to switch to underline cursor
    #[inline]
    pub fn underline_cursor(&mut self) -> &mut Self {
        self.out("\x1B[34h")
    }

    /// Add ANSI sequence to switch to block cursor
    #[inline]
    pub fn block_cursor(&mut self) -> &mut Self {
        self.out("\x1B[34l")
    }

    /// Add ANSI sequences to show cursor
    #[inline]
    pub fn show_cursor(&mut self) -> &mut Self {
        self.out("\x1B[?25h\x1B[?0c")
    }

    /// Add ANSI sequences to hide cursor
    #[inline]
    pub fn hide_cursor(&mut self) -> &mut Self {
        self.out("\x1B[?25l\x1B[?1c")
    }

    /// Add ANSI sequence to move to origin (top-left)
    #[inline]
    pub fn origin(&mut self) -> &mut Self {
        self.out("\x1B[H")
    }

    /// Add ANSI sequence to erase to end-of-line
    #[inline]
    pub fn erase_eol(&mut self) -> &mut Self {
        self.out("\x1B[K")
    }

    /// Add ANSI sequence to erase whole display
    #[inline]
    pub fn clear(&mut self) -> &mut Self {
        self.out("\x1B[2J")
    }

    /// Add N spaces
    #[inline]
    pub fn spaces(&mut self, n: i32) -> &mut Self {
        for _ in 0..n {
            self.asc(' ');
        }
        self
    }

    /// Add ANSI sequence to reset attributes to the default
    #[inline]
    pub fn attr_reset(&mut self) -> &mut Self {
        self.out("\x1B[0m")
    }

    /// Add ANSI sequence to do a full reset of the terminal
    #[inline]
    pub fn full_reset(&mut self) -> &mut Self {
        self.out("\x1Bc")
    }

    /// Switch to UTF-8 mode.  Useful for those terminals that don't
    /// default to UTF-8.
    #[inline]
    pub fn utf8_mode(&mut self) -> &mut Self {
        self.out("\x1B%G")
    }

    /// Move cursor to bottom line and do a linefeed.  This results in
    /// the screen scrolling one line, and the cursor being left at
    /// the bottom-left corner.
    #[inline]
    pub fn scroll_up(&mut self) -> &mut Self {
        self.at(-1, 0).asc('\n')
    }

    /// Save the current contents of the output buffer as the cleanup
    /// string, then clear the output buffer.  The cleanup string will
    /// be output to the terminal on error or when the terminal is
    /// paused.  This string should reset any settings that have been
    /// modified, and put the cursor somewhere sensible.  Default is
    /// `Esc c` which completely resets the terminal, but usually it's
    /// better to do something less drastic, for example reset just
    /// the state that was changed, put the cursor at the bottom of
    /// the screen and do a LF.  This will take effect on the next
    /// flush.
    pub fn save_cleanup(&mut self) {
        self.new_cleanup = Some(self.buf.drain(..).collect());
    }

    pub(crate) fn data_to_flush(&self) -> &[u8] {
        &self.buf[..self.flush_to]
    }

    pub(crate) fn drain_flush(&mut self) {
        self.buf.drain(..self.flush_to);
        self.flush_to = 0;
    }

    // Discard all buffered contents
    pub(crate) fn discard(&mut self) {
        self.buf.drain(..);
        self.flush_to = 0;
    }

    // Set size
    pub(crate) fn set_size(&mut self, sy: i32, sx: i32) {
        self.size = (sy, sx);
    }
}

impl Write for TermOut {
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
