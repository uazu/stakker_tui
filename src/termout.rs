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
    pub fn features(&self) -> &Features {
        &self.features
    }

    /// Get current terminal size: (rows, columns)
    pub fn size(&self) -> (i32, i32) {
        self.size
    }

    /// Mark all the data from the start of the buffer to the current
    /// end of the buffer as ready for flushing.  However the data
    /// won't be flushed until the [`Terminal`] actor receives a
    /// [`Terminal::flush`] call.  Any data added after this call
    /// won't be flushed unless another call to this method is made.
    ///
    /// [`Terminal::flush`]: struct.Terminal.html#method.flush
    /// [`Terminal`]: struct.Terminal.html
    pub fn flush(&mut self) {
        self.flush_to = self.buf.len();
    }

    /// Add a chunk of byte data to the output buffer.
    ///
    /// See also the `Write` implementation, which allows use of
    /// `write!` and `writeln!` to add data to the buffer.
    #[inline]
    pub fn out(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Add a single byte to the output buffer.
    pub fn out1(&mut self, v1: u8) {
        self.buf.push(v1);
    }

    /// Add two bytes to the output buffer.
    pub fn out2(&mut self, v1: u8, v2: u8) {
        self.buf.push(v1);
        self.buf.push(v2);
    }

    /// Add three bytes to the output buffer.
    pub fn out3(&mut self, v1: u8, v2: u8, v3: u8) {
        self.buf.push(v1);
        self.buf.push(v2);
        self.buf.push(v3);
    }

    /// Add a 1-3 digit decimal number to the output buffer, as used
    /// in control sequences.  If number is out of range, then nearest
    /// valid number is used.
    pub fn out_num(&mut self, v: i32) {
        if v <= 0 {
            self.out1(b'0');
        } else if v <= 9 {
            self.out1(v as u8 + b'0');
        } else if v <= 99 {
            self.out2((v / 10) as u8 + b'0', (v % 10) as u8 + b'0');
        } else if v <= 999 {
            self.out3(
                (v / 100) as u8 + b'0',
                (v / 10 % 10) as u8 + b'0',
                (v % 10) as u8 + b'0',
            );
        } else {
            self.out3(b'9', b'9', b'9');
        }
    }

    /// Add ANSI sequence to move cursor to the given coordinates.
    /// Note that coordinates are row-first, with (0,0) as top-left.
    pub fn to(&mut self, y: i32, x: i32) {
        self.out2(27, b'[');
        self.out_num(y + 1);
        self.out1(b';');
        self.out_num(x + 1);
        self.out1(b'H');
    }

    /// Add ANSI sequence to switch to underline cursor
    pub fn underline_cursor(&mut self) {
        self.out(b"\x1B[34h");
    }

    /// Add ANSI sequence to switch to block cursor
    pub fn block_cursor(&mut self) {
        self.out(b"\x1B[34l");
    }

    /// Add ANSI sequences to show cursor
    pub fn show_cursor(&mut self) {
        self.out(b"\x1B[?25h\x1B[?0c");
    }

    /// Add ANSI sequences to hide cursor
    pub fn hide_cursor(&mut self) {
        self.out(b"\x1B[?25l\x1B[?1c");
    }

    /// Add ANSI sequence to move to origin (top-left)
    pub fn origin(&mut self) {
        self.out(b"\x1B[H");
    }

    /// Add ANSI sequence to erase to end-of-line
    pub fn erase_eol(&mut self) {
        self.out(b"\x1B[K");
    }

    /// Add ANSI sequence to erase to end-of-display
    pub fn erase_eod(&mut self) {
        self.out(b"\x1B[J");
    }

    /// Add ANSI sequence to reset attributes to the default
    pub fn attr_reset(&mut self) {
        self.out(b"\x1B[0m");
    }

    /// Add ANSI sequence to do a full reset of the terminal
    pub fn full_reset(&mut self) {
        self.out(b"\x1Bc");
    }

    /// Switch to UTF-8 mode.  Useful for those terminals that don't
    /// default to UTF-8.
    pub fn utf8_mode(&mut self) {
        self.out(b"\x1B%G");
    }

    /// Move cursor to bottom line and do a linefeed.  This results in
    /// the screen scrolling one line, and the cursor being left at
    /// the bottom-left corner.
    pub fn scroll_up(&mut self) {
        self.to(self.size.0 - 1, 0);
        self.out1(10);
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
