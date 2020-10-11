use crate::os_glue::Glue;
use crate::{Features, Key, TermOut};
use stakker::{fwd, timer_max, Fwd, MaxTimerKey, Share, CX};
use std::error::Error;
use std::mem;
use std::panic::PanicInfo;
use std::sync::Arc;
use std::time::Duration;

/// Actor that manages the connection to the terminal
pub struct Terminal {
    resize: Fwd<Option<Share<TermOut>>>,
    input: Fwd<Key>,
    termout: Share<TermOut>,
    glue: Glue,
    disable_output: bool,
    paused: bool,
    inbuf: Vec<u8>,
    check_enable: bool,
    force_timer: MaxTimerKey,
    check_timer: MaxTimerKey,
    cleanup: Vec<u8>,
    panic_hook: Arc<Box<dyn Fn(&PanicInfo<'_>) + 'static + Sync + Send>>,
}

impl Terminal {
    /// Set up the terminal.  Sends a message back to `resize`
    /// immediately, which provides a reference to the shared
    /// [`TermOut`] which is used to buffer and flush terminal output
    /// data.
    ///
    /// Whenever the window size changes, a new `resize` message is
    /// sent.  When the terminal output is paused, `None` is sent to
    /// `resize` to let the app know that there is no output available
    /// right now.
    ///
    /// Input keys received are sent to `input` once decoded.
    ///
    /// In case of an error that can't be handled, cleans up the
    /// terminal state and terminates the actor with
    /// `ActorDied::Failed`.  The actor that created the terminal can
    /// catch that and do whatever cleanup is necessary before
    /// aborting the process.
    ///
    /// # Panic handling
    ///
    /// When Rust panics, the terminal must be restored to its normal
    /// state otherwise things would be left in a bad state for the
    /// user (in cooked mode with no echo, requiring the user to
    /// blindly type `reset` on the command-line).  So this code saves
    /// a copy of the current panic handler (using
    /// `std::panic::take_hook`), and then installs its own handler
    /// that does terminal cleanup before calling on to the saved
    /// panic handler.  This mean that if any custom panic handler is
    /// needed by the application, then it must be set up before the
    /// call to [`Terminal::init`].
    ///
    /// [`TermOut`]: struct.TermOut.html
    pub fn init(cx: CX![], resize: Fwd<Option<Share<TermOut>>>, input: Fwd<Key>) -> Option<Self> {
        // TODO: Query TERM/terminfo/environment for features to put in Features
        let features = Features { colour_256: false };
        let term = cx.this().clone();
        let glue = match Glue::new(cx, term) {
            Ok(v) => v,
            Err(e) => {
                cx.fail(e);
                return None;
            }
        };
        let termout = Share::new(cx, TermOut::new(features));
        let mut this = Self {
            resize,
            input,
            termout,
            glue,
            disable_output: false,
            paused: false,
            inbuf: Vec::new(),
            check_enable: false,
            force_timer: MaxTimerKey::default(),
            check_timer: MaxTimerKey::default(),
            cleanup: b"\x1Bc".to_vec(),
            panic_hook: Arc::new(std::panic::take_hook()),
        };
        this.handle_resize(cx);
        this.update_panic_hook();
        Some(this)
    }

    /// Enable or disable generation of the [`Key::Check`] keypress,
    /// which occurs in a gap in typing, 300ms after the last key
    /// pressed.  This may be used to do validation if that's too
    /// expensive to do on every keypress.
    ///
    /// [`Key::Check`]: enum.Key.html#variant.Check
    pub fn check(&mut self, _cx: CX![], enable: bool) {
        self.check_enable = enable;
    }

    /// Ring the bell (i.e. beep) immediately.  Doesn't wait for the
    /// buffered terminal data to be flushed.  Will output even when
    /// paused.
    pub fn bell(&mut self, cx: CX![]) {
        if !self.disable_output {
            if let Err(e) = self.glue.write(&b"\x07"[..]) {
                self.disable_output = true;
                self.failure(cx, e);
            }
        }
    }

    /// Pause terminal input and output handling.  Sends the cleanup
    /// sequence to the terminal, and switches to cooked mode.  Sends
    /// a `resize` message with `None` to tell the app that output is
    /// disabled.
    ///
    /// This call should be used before forking off a process which
    /// might prompt the user and receive user input, otherwise this
    /// process would compete with the sub-process for user input.
    /// Resume after the subprocess has finished with the `resume`
    /// call.
    pub fn pause(&mut self, cx: CX![]) {
        if !self.paused {
            fwd!([self.resize], None);
            self.glue.input(false);
            self.termout.rw(cx).discard();
            self.termout.rw(cx).bytes(&self.cleanup[..]);
            self.flush(cx);
            self.paused = true;
            self.update_panic_hook();
        }
    }

    /// Resume terminal output and input handling.  Switches to raw
    /// mode and sends a resize message to trigger a full redraw.
    pub fn resume(&mut self, cx: CX![]) {
        if self.paused {
            self.paused = false;
            self.glue.input(true);
            self.termout.rw(cx).discard();
            self.handle_resize(cx);
            self.update_panic_hook();
        }
    }

    // Handle an unrecoverable failure.  Try to clean up before
    // terminating the actor.
    fn failure(&mut self, cx: CX![], e: impl Error + 'static) {
        self.pause(cx);
        cx.fail(e);
    }

    /// Flush to the terminal all the data that's ready for sending
    /// from the TermOut buffer.  Use [`TermOut::flush`] first to mark
    /// the point up to which data should be flushed.
    ///
    /// [`TermOut::flush`]: struct.TermOut.html#method.flush
    pub fn flush(&mut self, cx: CX![]) {
        if self.termout.rw(cx).new_cleanup.is_some() {
            // Don't replace unless we're sure there's a new value
            if let Some(cleanup) = mem::replace(&mut self.termout.rw(cx).new_cleanup, None) {
                self.cleanup = cleanup;
                self.update_panic_hook();
            }
        }

        if !self.disable_output {
            if self.paused {
                // Just drop the output whilst paused.  We'll trigger
                // a full refresh on resuming
                self.termout.rw(cx).drain_flush();
            } else {
                let ob = self.termout.rw(cx);
                let result = self.glue.write(ob.data_to_flush());
                ob.drain_flush();
                if let Err(e) = result {
                    self.disable_output = true;
                    self.failure(cx, e);
                }
            }
        }
    }

    /// Handle a resize event from the TTY.  Gets new size, and
    /// notifies upstream.
    pub(crate) fn handle_resize(&mut self, cx: CX![]) {
        match self.glue.get_size() {
            Ok((sy, sx)) => {
                self.termout.rw(cx).set_size(sy, sx);
                fwd!([self.resize], Some(self.termout.clone()));
            }
            Err(e) => self.failure(cx, e),
        }
    }

    /// Handle an I/O error on the TTY input
    pub(crate) fn handle_error_in(&mut self, cx: CX![], err: std::io::Error) {
        self.failure(cx, err);
    }

    /// Handle new bytes from the TTY input
    pub(crate) fn handle_data_in(&mut self, cx: CX![]) {
        self.glue.read_data(&mut self.inbuf);
        self.do_data_in(cx, false);
    }

    fn do_data_in(&mut self, cx: CX![], force: bool) {
        let mut pos = 0;
        let len = self.inbuf.len();
        if len != 0 {
            if !force {
                // Note that this is too fast to catch M-Esc passed
                // through screen, as that seems to apply a 300ms
                // pause between the two Esc chars.  For everything
                // else including real terminals it should be okay.
                timer_max!(
                    &mut self.force_timer,
                    cx.now() + Duration::from_millis(100),
                    [cx],
                    do_data_in(true)
                );
            }
            while pos < len {
                match Key::decode(&self.inbuf[pos..len], force) {
                    None => break,
                    Some((count, key)) => {
                        pos += count;
                        fwd!([self.input], key);
                        if self.check_enable {
                            let check_expiry = cx.now() + Duration::from_millis(300);
                            timer_max!(&mut self.check_timer, check_expiry, [cx], check_key());
                        }
                    }
                }
            }
        }
        self.inbuf.drain(..pos);
    }

    fn check_key(&mut self, _cx: CX![]) {
        if self.check_enable {
            fwd!([self.input], Key::Check);
        }
    }

    // Install a panic hook that (if necessary) outputs the current
    // cleanup string, restores cooked mode and then does the default
    // panic action (e.g. dump out backtrace).  This should be called
    // every time we switch to/from raw mode, and every time the
    // cleanup string is changed.
    fn update_panic_hook(&mut self) {
        // Discard old hook
        let _ = std::panic::take_hook();

        let defhook = self.panic_hook.clone();
        if self.paused {
            std::panic::set_hook(Box::new(move |info| defhook(info)));
        } else {
            let cleanup_fn = self.glue.cleanup_fn();
            let cleanup = self.cleanup.clone();

            std::panic::set_hook(Box::new(move |info| {
                cleanup_fn(&cleanup[..]);
                defhook(info);
            }));
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Drop panic hook and clean up terminal
        let _ = std::panic::take_hook();
        if !self.paused {
            self.glue.cleanup_fn()(&self.cleanup[..]);
        }
    }
}
