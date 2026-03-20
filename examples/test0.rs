//! Test colour and unicode handling.  Uses direct ANSI output via
//! `Output`.
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
        if key == Key::Ctrl('L') {
            self.redraw(cx);
        } else {
            stop!(cx);
        }
    }

    fn draw(o: &mut Output) {
        o.clear_all_99();
        if o.sy() < 24 || o.sx() < 80 {
            o.hfb(162);
            o.at(o.sy() >> 1, (o.sx() - 30) >> 1);
            write!(o, " Terminal too small: {} x {} ", o.sy(), o.sx()).unwrap();
            o.hfb(99);
            o.flush();
            return;
        }

        let x0 = (o.sx() - 80) >> 1;
        let y0 = (o.sy() - 24) >> 1;
        let mut yy = y0;
        let c256 = o.features().colour_256;
        o.at(yy, x0).hfb(99).text("Colour mode:");
        o.at(yy, x0 + 15)
            .hfb(199)
            .text(if c256 { "256-colour" } else { "8-colour" });
        yy += 2;

        o.at(yy, x0).hfb(99).text("Attributes:");
        yy += 1;
        const TEXT: [&str; 4] = ["White:", "Brown:", "Blue:", "Black:"];
        const BASE_HFB: [u16; 4] = [70, 60, 10, 3];
        for i in 0..4 {
            let hfb = BASE_HFB[i];
            o.at(yy, x0 + 2).hfb(99).text(TEXT[i]);
            o.at(yy, x0 + 16).hfb(hfb).text(" norm ");
            o.at(yy, x0 + 22).hfb(100 + hfb).text("bold ");
            o.at(yy, x0 + 27).hfb(200 + hfb).text("undl ");
            o.at(yy, x0 + 32).hfb(300 + hfb).text("both ");
            yy += 1;
        }
        o.at(yy + 1, x0)
            .hfb(60)
            .text(" (On 8-colour expect bold/ul to");
        o.at(yy + 2, x0)
            .hfb(60)
            .text("  be emulated as colour changes.)");
        let y1a = yy + 3;

        yy = y0;
        o.at(yy, x0 + 40)
            .hfb(99)
            .text("Base, bold and background colours:");
        o.at(yy + 1, x0 + 42).hfb(7).text("  black   ");
        o.at(yy + 2, x0 + 42).hfb(10).text("  blue    ");
        o.at(yy + 3, x0 + 42).hfb(20).text("  red     ");
        o.at(yy + 4, x0 + 42).hfb(30).text("  magenta ");
        o.at(yy + 5, x0 + 42).hfb(40).text("  green   ");
        o.at(yy + 6, x0 + 42).hfb(50).text("  cyan    ");
        o.at(yy + 7, x0 + 42).hfb(60).text("  yellow  ");
        o.at(yy + 8, x0 + 42).hfb(70).text("  white   ");
        o.at(yy + 1, x0 + 54).hfb(100).text("  black   ");
        o.at(yy + 2, x0 + 54).hfb(110).text("  blue    ");
        o.at(yy + 3, x0 + 54).hfb(120).text("  red     ");
        o.at(yy + 4, x0 + 54).hfb(130).text("  magenta ");
        o.at(yy + 5, x0 + 54).hfb(140).text("  green   ");
        o.at(yy + 6, x0 + 54).hfb(150).text("  cyan    ");
        o.at(yy + 7, x0 + 54).hfb(160).text("  yellow  ");
        o.at(yy + 8, x0 + 54).hfb(170).text("  white   ");
        o.at(yy + 1, x0 + 66).hfb(70).text("  black   ");
        o.at(yy + 2, x0 + 66).hfb(71).text("  blue    ");
        o.at(yy + 3, x0 + 66).hfb(72).text("  red     ");
        o.at(yy + 4, x0 + 66).hfb(73).text("  magenta ");
        o.at(yy + 5, x0 + 66).hfb(4).text("  green   ");
        o.at(yy + 6, x0 + 66).hfb(5).text("  cyan    ");
        o.at(yy + 7, x0 + 66).hfb(6).text("  yellow  ");
        o.at(yy + 8, x0 + 66).hfb(7).text("  white   ");
        o.at(yy + 10, x0 + 40)
            .hfb(60)
            .text(" (Expect all 16 colours to be");
        o.at(yy + 11, x0 + 40)
            .hfb(60)
            .text("  visible in all cases.)");
        let y1b = yy + 12;
        let end = y1a.max(y1b);

        yy = end + 10;
        o.at(yy - 1, x0).hfb(99).text("Hue wheel");
        o.at(yy, x0);
        if o.features().colour_256 {
            const RED: [i32; 7] = [255, 255, 0, 0, 0, 255, 255];
            const GREEN: [i32; 7] = [0, 255, 255, 255, 0, 0, 0];
            const BLUE: [i32; 7] = [0, 0, 0, 255, 255, 255, 0];
            for x in 0..78 {
                let i = x / 13;
                let off = (x - i * 13) as i32;
                let r = RED[i] + (RED[i + 1] - RED[i]) * off / 13;
                let g = GREEN[i] + (GREEN[i + 1] - GREEN[i]) * off / 13;
                let b = BLUE[i] + (BLUE[i + 1] - BLUE[i]) * off / 13;
                let rgb = (r << 16) + (g << 8) + b;
                let hfb = o.hfb_alloc(false, false, 0xFFFFFF, rgb as u32);
                o.hfb(hfb).spaces(1);
            }
        } else {
            o.hfb(2).spaces(13);
            o.hfb(6).spaces(13);
            o.hfb(4).spaces(13);
            o.hfb(5).spaces(13);
            o.hfb(1).spaces(13);
            o.hfb(3).spaces(13);
        }

        yy = end + 1;
        let mut xx = x0 + 40;

        o.at(yy, xx).hfb(99).text("ASCII");
        for a in 0..96 {
            o.at(yy + 1 + (a >> 4), xx + 1 + (a & 15))
                .char(char::from_u32((a + 32) as u32).unwrap());
        }

        xx += 20;
        o.at(yy, xx).hfb(99).text("Latin-1");
        for a in 0..96 {
            o.at(yy + 1 + (a >> 4), xx + 1 + (a & 15))
                .char(char::from_u32((a + 160) as u32).unwrap());
        }

        const BOXTITLE1: &str = "\
Single  Single  Double  Thick";
        const BOXTITLE2: &str = "\
+Double +Thick  +Single +Single";

        const BOXTEST: &str = "\
+-+-+-+ +-+-+-+ +=+=+=+ +*+*+*+
| | H | | | $ | H H | H $ $ | $
+-+-+-+ +-+-+-+ +=+=+=+ +*+*+*+
| | H | | | $ | H H | H $ $ | $
+=+=+=+ +*+*+*+ +-+-+-+ +-+-+-+
| | H | | | $ | H H | H $ $ | $
+-+-+-+ +-+-+-+ +=+=+=+ +*+*+*+";

        let boxtest = Output::box_from_ascii_art(BOXTEST);
        yy = end + 1;
        xx = x0 + 2;
        o.at(yy - 2, xx).text(BOXTITLE1);
        o.at(yy - 1, xx).text(BOXTITLE2);
        for line in boxtest.split('\n') {
            o.at(yy, xx).text(line);
            yy += 1;
        }

        o.flush();
    }
}
