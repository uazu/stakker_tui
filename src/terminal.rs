use crate::os_glue::Glue;
use crate::{Features, Key, Output, TermShare, sizer::Sizer};
use stakker::{CX, Fwd, MaxTimerKey, Share, fwd, fwd_to, lazy, timer_max};
use std::error::Error;
use std::panic::PanicHookInfo;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Actor that manages the connection to the terminal
pub struct Terminal {
    resize: Fwd<Option<TermShare>>,
    input: Fwd<Key>,
    share: TermShare,
    glue: Glue,
    disable_output: bool,
    paused: bool,
    paused_share: Arc<AtomicBool>, // Holds same value as `paused`
    inbuf: Vec<u8>,
    check_enable: bool,
    force_timer: MaxTimerKey,
    check_timer: MaxTimerKey,
    cleanup: Vec<u8>,
    #[allow(clippy::type_complexity)]
    panic_hook: Arc<Box<dyn Fn(&PanicHookInfo<'_>) + 'static + Sync + Send>>,
}

impl Terminal {
    /// Set up the terminal.  Sends a message back to `resize`
    /// immediately, which provides a [`TermShare`] which is used to
    /// access various means of outputting data to the terminal.
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
    /// `sizer` is the [`Sizer`] that will be used for
    /// measuring glyphs.  It is passed through to [`TermShare`], and
    /// can be retrieved from there using [`Output::sizer`].
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
    /// [`Output::sizer`]: struct.Output.html#method.sizer
    /// [`Sizer`]: sizer/struct.Sizer.html
    /// [`TermShare`]: struct.TermShare.html
    /// [`Terminal::init`]: struct.Terminal.html#method.init
    pub fn init(
        cx: CX![],
        sizer: Sizer,
        resize: Fwd<Option<TermShare>>,
        input: Fwd<Key>,
    ) -> Option<Self> {
        let mut features = Features { colour_256: false };
        if let Ok(env_term) = std::env::var("TERM")
            && env_term.contains("256")
        {
            features.colour_256 = true;
        } else if let Ok(output) = std::process::Command::new("tput").arg("colors").output()
            && let Ok(colors_str) = str::from_utf8(output.stdout.as_slice())
            && let Ok(n_colors) = colors_str.trim().parse::<u64>()
            && n_colors >= 256
        {
            features.colour_256 = true;
        }

        let term = cx.this().clone();
        let glue = match Glue::new(cx, term) {
            Ok(v) => v,
            Err(e) => {
                cx.fail(e);
                return None;
            }
        };
        let fwd_flush = fwd_to!([cx], flush() as ());
        let fwd_lazy_upd = fwd_to!([cx], lazy_update() as ());
        let share = TermShare::new(Share::new(
            cx,
            Output::new(features, sizer, fwd_flush, fwd_lazy_upd),
        ));
        let mut this = Self {
            resize,
            input,
            share,
            glue,
            disable_output: false,
            paused: false,
            paused_share: Arc::new(AtomicBool::new(false)),
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
        if !self.disable_output
            && let Err(e) = self.glue.write(&b"\x07"[..])
        {
            self.disable_output = true;
            self.failure(cx, e);
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
            let o = self.share.output(cx);
            o.discard();
            o.bytes(&self.cleanup[..]);
            o.flush();

            // As an optimisation, change the tile generation to
            // disable all current tiles, to stop any actors using
            // those tiles from updating the local_page needlessly.
            o.tile_generation = o.tile_generation.wrapping_add(1);

            self.flush(cx);
            self.paused = true;
            self.paused_share.store(true, Ordering::SeqCst);
        }
    }

    /// Resume terminal output and input handling.  Switches to raw
    /// mode and sends a resize message to trigger a full redraw.
    pub fn resume(&mut self, cx: CX![]) {
        if self.paused {
            self.paused = false;
            self.paused_share.store(false, Ordering::SeqCst);
            self.glue.input(true);
            self.share.output(cx).discard();
            self.handle_resize(cx);
        }
    }

    // Handle an unrecoverable failure.  Try to clean up before
    // terminating the actor.
    fn failure(&mut self, cx: CX![], e: impl Error + 'static) {
        self.pause(cx);
        cx.fail(e);
    }

    // Handle an unrecoverable failure.  Try to clean up before
    // terminating the actor.
    fn fail_str(&mut self, cx: CX![], msg: &str) {
        self.pause(cx);
        cx.fail_string(msg);
    }

    /// Flush to the terminal all the data that's ready for sending
    /// from the [`Output`] buffer.  Use [`Output::flush`] first to
    /// mark the point up to which data should be flushed.
    ///
    /// [`Output::flush`]: struct.Output.html#method.flush
    /// [`Output`]: struct.Output.html
    fn flush(&mut self, cx: CX![]) {
        if self.share.output(cx).new_cleanup.is_some() {
            // Don't replace unless we're sure there's a new value
            if let Some(cleanup) = self.share.output(cx).new_cleanup.take() {
                self.cleanup = cleanup;
                self.update_panic_hook();
            }
        }

        if !self.disable_output {
            if self.paused {
                // Just drop the output whilst paused.  We'll trigger
                // a full refresh on resuming
                self.share.output(cx).drain_flush();
            } else {
                let ob = self.share.output(cx);
                let data = ob.data_to_flush();
                if !data.is_empty() {
                    let result = self.glue.write(data);
                    ob.drain_flush();
                    if let Err(e) = result {
                        self.disable_output = true;
                        self.failure(cx, e);
                    }
                }
            }
        }
    }

    /// Set a lazy callback to do a terminal update.  Used by `Tile`
    /// code to gather all the tile changes into a single update.
    fn lazy_update(&mut self, cx: CX![]) {
        lazy!([cx], |this, cx| {
            this.share.output(cx).update_to_local_page();
        });
    }

    /// Handle a resize event from the TTY.  Gets new size, and
    /// notifies upstream.
    pub(crate) fn handle_resize(&mut self, cx: CX![]) {
        match self.glue.get_size() {
            Ok(Some((sy, sx))) => {
                self.share.output(cx).set_size(sy, sx);
                fwd!([self.resize], Some(self.share.clone()));
            }
            Ok(None) => {
                // Standard output is not a TTY.
                self.fail_str(cx, "This app requires that standard output is a TTY");
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

    // Install a panic hook that, if the Terminal is not paused,
    // outputs the current cleanup string and restores cooked mode,
    // and then in all cases does the default panic action (e.g. dump
    // out backtrace).  This method should be called every time the
    // cleanup string is changed.  It detects the paused state through
    // `paused_share`, and doesn't do terminal cleanup in that case.
    fn update_panic_hook(&mut self) {
        // Discard old hook
        let _ = std::panic::take_hook();

        let defhook = self.panic_hook.clone();
        let paused_share = self.paused_share.clone();
        let cleanup_fn = self.glue.cleanup_fn();
        let cleanup = self.cleanup.clone();

        std::panic::set_hook(Box::new(move |info| {
            if !paused_share.load(Ordering::SeqCst) {
                cleanup_fn(&cleanup[..]);
            }
            defhook(info);
        }));
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Clean up terminal if not paused
        if !self.paused {
            self.glue.cleanup_fn()(&self.cleanup[..]);

            // We're not allowed to change the panic hook in this method
            // because we might be unwinding as part of a panic.  So leave
            // the panic hook there but disable it by setting
            // `paused_share` to `true`.  This results in a small memory
            // leak.  If another `Terminal` is started, then the default
            // hook it finds will be this (disabled) panic hook.  Since
            // running one Terminal after another is not normal behaviour,
            // this should be okay.
            self.paused_share.store(true, Ordering::SeqCst);
        }
    }
}
