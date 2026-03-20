//! Colour-combination and keypress detection test.  Uses direct
//! access via `Output`.
use stakker::{ActorOwn, CX, Stakker, StopCause, actor, fwd_to, ret_shutdown, ret_some_to, stop};
use stakker_mio::MioPoll;
use stakker_mio::mio::{Events, Poll};
use stakker_tui::{Key, Output, TermShare, Terminal, sizer::SimpleSizer};
use std::io::Write;
use std::time::{Duration, Instant};

// Let main loop dump setup I/O errors to console.  You could catch
// them instead if you want.
fn main() -> std::io::Result<()> {
    let mut stakker = Stakker::new(Instant::now());
    let s = &mut stakker;
    let miopoll = MioPoll::new(s, Poll::new()?, Events::with_capacity(1024), 0)?;

    let _app = actor!(s, App::init(), ret_shutdown!(s));

    const MAXDELAY: Duration = Duration::from_secs(60);
    let mut idle_pending = s.run(Instant::now(), false);
    let mut io_pending = false;
    let mut activity;
    while s.not_shutdown() {
        let maxdur = s.next_wait_max(Instant::now(), MAXDELAY, idle_pending || io_pending);
        (activity, io_pending) = miopoll.poll(maxdur)?;
        idle_pending = s.run(Instant::now(), !activity);
    }
    if let Some(reason) = s.shutdown_reason() {
        if reason.has_error() {
            println!("{reason}");
        }
    }

    Ok(())
}

struct App {
    _terminal: ActorOwn<Terminal>,
    tshare: Option<TermShare>,
}

impl App {
    fn init(cx: CX![]) -> Option<Self> {
        let _terminal = actor!(
            cx,
            Terminal::init(
                SimpleSizer::new(),
                fwd_to!([cx], resize() as (Option<TermShare>)),
                fwd_to!([cx], input() as (Key))
            ),
            ret_some_to!([cx], |_, cx, cause: StopCause| {
                println!("Terminal actor failed: {cause}");
                stop!(cx);
            })
        );
        Some(Self {
            _terminal,
            tshare: None,
        })
    }

    fn resize(&mut self, cx: CX![], tshare: Option<TermShare>) {
        self.tshare = tshare;
        if self.tshare.is_some() {
            self.redraw(cx);
        }
    }

    fn redraw(&mut self, cx: CX![]) {
        if let Some(ref tshare) = self.tshare {
            let o = tshare.output(cx);
            o.attr_99().cursor_show().scroll_up().save_cleanup();
            o.cursor_hide();
            Self::draw(o);
        }
    }

    fn input(&mut self, cx: CX![], key: Key) {
        match key {
            Key::Ctrl('L') => self.redraw(cx),
            Key::Ctrl('C') => stop!(cx),
            _ => {
                use std::str::FromStr;
                let s = format!("{key}");
                let k = Key::from_str(&s);
                assert!(k.is_ok(), "Round-trip failed for {s}");
                let k = k.unwrap();
                assert_eq!(k, key, "Mismatch for {key} -> {s} -> {k}");
                let hfb = if s.starts_with("M-") {
                    172
                } else if s.starts_with("C-") {
                    141
                } else if matches!(key, Key::Pr(_)) {
                    151
                } else {
                    162
                };
                if let Some(ref tshare) = self.tshare {
                    let o = tshare.output(cx);
                    o.hfb(hfb).text(&s);
                    o.hfb(71).text(" ");
                    o.flush();
                }
            }
        }
    }

    fn draw(o: &mut Output) {
        o.clear_all_99();
        let sx = o.sx();
        let sy = o.sy();
        if sy < 24 || sx < 80 {
            o.hfb(162);
            o.at(o.sy() >> 1, (o.sx() - 30) >> 1);
            write!(o, " Terminal too small: {} x {} ", o.sy(), o.sx()).unwrap();
            o.hfb(99);
            o.flush();
            return;
        }

        const TEXT: [char; 17] = [
            'T', 'h', 'i', 's', ' ', 'i', 's', ' ', 'a', ' ', 't', 'e', 's', 't', '.', ' ', ' ',
        ];
        let mut last_hfb = 99;
        for y in 0..sy {
            o.at(y, 0);
            for x in 0..sx {
                let hfb = ((x * 8) / sx + (y & 7) * 10 + ((y >> 3) & 1) * 100) as u16;
                if hfb != last_hfb {
                    o.hfb(hfb);
                    last_hfb = hfb;
                }
                o.char(TEXT[(1000 + x - y) as usize % TEXT.len()]);
            }
        }
        o.at(sy - 2, (sx - 60) / 2)
            .hfb(162)
            .text("[ Type to show decoding, Ctrl-C to exit, Ctrl-L to redraw ]")
            .hfb(99);
        o.at(0, 0);
        o.flush();
    }
}
