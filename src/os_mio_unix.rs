//! Handle WINCH through a UNIX signal forwarded through a pipe to
//! MIO.  Dump output data straight to stdout FD, with a blocking
//! call.  This will block the whole thread if the TTY is in XOFF.

use crate::terminal::Terminal;
use libc::{c_int, c_ushort, c_void, ioctl, size_t, TIOCGWINSZ};
use signal_hook::SigId;
use stakker::{call, fwd_do, Actor, Core};
use stakker_mio::mio::Interest;
use stakker_mio::{FdSource, MioPoll, MioSource};
use std::io::{Error, ErrorKind, Result};
use std::mem;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;

#[repr(C)]
#[derive(Default)]
struct WinSize {
    row: c_ushort,
    col: c_ushort,
    xpixel: c_ushort,
    ypixel: c_ushort,
}

pub struct Glue {
    poll: MioPoll,
    term: Actor<Terminal>,
    _read: UnixStream,
    _winch_src: MioSource<FdSource>,
    stdin_src: Option<MioSource<FdSource>>,
    sigid: SigId,
    saved: Option<libc::termios>,
}

const STDIN_FD: c_int = 0;
const STDOUT_FD: c_int = 1;

impl Glue {
    pub fn new(core: &mut Core, term: Actor<Terminal>) -> Result<Self> {
        let poll = core.anymap_get::<MioPoll>();

        // Setup notification of WINCH signals
        let (read, write) = UnixStream::pair()?;
        let sigid = signal_hook::pipe::register(signal_hook::SIGWINCH, write)?;
        read.set_nonblocking(true)?;
        let fd = read.as_raw_fd();
        let fdsrc = FdSource::new(fd);
        let term2 = term.clone();
        let fwd = fwd_do!(move |_| {
            // Read all bytes from the notification pipe, to make sure
            // we get a new Ready notification for the next byte sent
            let mut buf = [0u8; 32];
            while 0 < unsafe { libc::read(fd, &mut buf[0] as *mut u8 as *mut _, buf.len()) } {}
            call!([term2], handle_resize());
        });
        let winch_src = poll.add(fdsrc, Interest::READABLE, 16, fwd)?;

        // Setup notification of input
        if 0 > unsafe { libc::fcntl(STDIN_FD, libc::F_SETFL, libc::O_NONBLOCK) } {
            return Err(Error::last_os_error());
        }
        let mut this = Self {
            poll,
            term,
            _read: read,
            _winch_src: winch_src,
            stdin_src: None,
            sigid,
            saved: None,
        };

        this.input(true);

        Ok(this)
    }

    /// Get the terminal size
    pub fn get_size(&mut self) -> Result<(i32, i32)> {
        let mut ws = WinSize::default();
        match unsafe { ioctl(1, TIOCGWINSZ, &mut ws as *mut _ as *mut u8) } {
            -1 => Err(Error::last_os_error()),
            _ => Ok((i32::from(ws.row), i32::from(ws.col))),
        }
    }

    /// Write data to the terminal
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        Self::write_aux(data)
    }

    fn write_aux(mut data: &[u8]) -> Result<()> {
        while !data.is_empty() {
            let cnt = unsafe {
                libc::write(
                    STDOUT_FD,
                    &data[0] as *const _ as *const c_void,
                    data.len() as size_t,
                )
            };
            if cnt < 0 {
                return Err(Error::last_os_error());
            }
            data = &data[cnt as usize..];
        }
        Ok(())
    }

    /// Enable or disable input
    pub fn input(&mut self, enable: bool) {
        if enable && self.stdin_src.is_none() && self.termios_set_raw() {
            let fdsrc = FdSource::new(STDIN_FD);
            let term = self.term.clone();
            let fwd = fwd_do!(move |_| call!([term], handle_data_in()));
            match self.poll.add(fdsrc, Interest::READABLE, 16, fwd) {
                Err(e) => call!([self.term], handle_error_in(e)),
                Ok(src) => self.stdin_src = Some(src),
            }
        }
        if !enable {
            // MioSource drop handler removes `mio` handler for stdin
            self.stdin_src = None;
            self.termios_restore();
        }
    }

    /// Generate a new standalone cleanup function that will make a
    /// best effort to restore the terminal to normal from the state
    /// that it's currently in, ignoring errors.  This is for use from
    /// a panic handler.
    pub fn cleanup_fn(&mut self) -> Box<dyn Fn(&[u8]) + Send + Sync + 'static> {
        let saved = self.saved;
        Box::new(move |reset| {
            let _ = Self::write_aux(reset);
            if let Some(saved) = saved {
                unsafe { libc::tcsetattr(STDIN_FD, libc::TCSANOW, &saved as *const libc::termios) };
            };
        })
    }

    // Read all available stdin data into given Vec
    pub fn read_data(&mut self, inbuf: &mut Vec<u8>) {
        let mut buf = [0u8; 32];
        loop {
            let cnt = unsafe { libc::read(STDIN_FD, &mut buf[0] as *mut u8 as *mut _, buf.len()) };
            if cnt < 0 {
                #[allow(unreachable_patterns)]
                match errno::errno().0 {
                    libc::EWOULDBLOCK | libc::EAGAIN => (),
                    _ => call!([self.term], handle_error_in(Error::last_os_error())),
                }
                break;
            }
            inbuf.extend_from_slice(&buf[..cnt as usize]);
        }
    }

    // Set terminal into raw mode if not already in raw mode, and save
    // previous state so that it can be restored
    fn termios_set_raw(&mut self) -> bool {
        if self.saved.is_some() {
            return false;
        }

        if 0 == unsafe { libc::isatty(STDIN_FD) } {
            let err = Error::new(ErrorKind::Other, "Standard input is not a TTY");
            call!([self.term], handle_error_in(err));
            return false;
        }

        let mut tbuf = mem::MaybeUninit::uninit();
        if 0 > unsafe { libc::tcgetattr(STDIN_FD, tbuf.as_mut_ptr()) } {
            let err = Error::new(Error::last_os_error().kind(), "Unable to get terminal mode");
            call!([self.term], handle_error_in(err));
            return false;
        }
        let mut tbuf = unsafe { tbuf.assume_init() };

        self.saved = Some(tbuf);
        unsafe { libc::cfmakeraw(&mut tbuf as *mut _) };

        if 0 > unsafe { libc::tcsetattr(STDIN_FD, libc::TCSANOW, &tbuf as *const libc::termios) } {
            let err = Error::new(
                Error::last_os_error().kind(),
                "Unable to set terminal raw mode",
            );
            call!([self.term], handle_error_in(err));
            return false;
        }

        true
    }

    // Restore terminal settings
    fn termios_restore(&mut self) {
        if let Some(saved) = mem::replace(&mut self.saved, None) {
            if 0 > unsafe {
                libc::tcsetattr(STDIN_FD, libc::TCSANOW, &saved as *const libc::termios)
            } {
                let err = Error::new(
                    Error::last_os_error().kind(),
                    "Unable to restore terminal mode",
                );
                call!([self.term], handle_error_in(err));
            }
        }
    }
}

impl Drop for Glue {
    fn drop(&mut self) {
        // This call cleans up the UnixStream write end
        signal_hook::unregister(self.sigid);
    }
}
