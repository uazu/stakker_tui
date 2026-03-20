//! Page update test.  Uses Page-based update of the terminal
//! (`Output::update_to`)
use stakker::{ActorOwn, CX, Stakker, StopCause, actor, fwd_to, ret_shutdown, ret_some_to, stop};
use stakker_mio::MioPoll;
use stakker_mio::mio::{Events, Poll};
use stakker_tui::{Key, Page, TermShare, Terminal, sizer::SimpleSizer};
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
    page: Page,
    spacing: i32,
    spacing_on: bool,
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
            page: Page::empty(),
            spacing: 0,
            spacing_on: false,
        })
    }

    fn resize(&mut self, cx: CX![], tshare: Option<TermShare>) {
        self.tshare = tshare;

        if let Some(ref tshare) = self.tshare {
            let o = tshare.output(cx);
            o.attr_99().cursor_show().scroll_up().save_cleanup();
            o.cursor_hide();
            self.page = o.new_page();
        }

        self.redraw(cx, true);
    }

    fn input(&mut self, cx: CX![], key: Key) {
        match key {
            Key::Ctrl('L') => self.redraw(cx, true),
            Key::Ctrl('C') => stop!(cx),
            _ => {
                if self.spacing_on {
                    self.spacing_on = false;
                } else {
                    self.spacing_on = true;
                    self.spacing = ((self.spacing + 1) % 20).max(3);
                }
                self.redraw(cx, false);
            }
        }
    }

    fn redraw(&mut self, cx: CX![], full: bool) {
        let pg = &mut self.page;
        let sx = pg.sx();
        let sy = pg.sy();

        if sy < 24 || sx < 80 {
            let mut r = pg.full();
            r.clear_all_99();
            r.at(sy >> 1, (sx - 30) >> 1).hfb(162);
            write!(r, " Terminal too small: {sy} x {sx} ").unwrap();
            return;
        }

        const TEXT: &str = "This is a test.  ";
        const TEXTLEN: usize = TEXT.len();
        let mid = sx >> 1;
        for y in 0..sy {
            let mut r = pg.region(y, 0, 1, sx + TEXTLEN as i32 * 2);
            r.at(0, y % TEXTLEN as i32 - TEXTLEN as i32).hfb(99);
            while r.get_x() < sx {
                r.text(TEXT);
            }
            if self.spacing_on {
                let mut x = (sx + sy) / 2 - y;
                while x > 0 {
                    x -= self.spacing
                }
                r.at(0, x);
                while r.get_x() < sx {
                    r.text("/");
                    r.skip(self.spacing - 1);
                }
            }
            let shift = y * 6 / sy;
            for x in 0..sx {
                r.set_hfb(0, x, (((x - mid) >> shift) & 3) as u16 + 70);
            }
        }
        pg.full()
            .at(sy - 2, (sx - 40) >> 1)
            .hfb(6)
            .text("  PRESS ANY KEY TO CONTINUE, Ctrl-C TO END  ");

        self.update(cx, full);
    }

    fn update(&mut self, cx: CX![], full: bool) {
        if let Some(ref tshare) = self.tshare {
            let o = tshare.output(cx);
            o.update_to(&mut self.page, full);
        }
    }
}
