//! Font editor, using `Tile` interface
#![allow(clippy::octal_escapes)]

use stakker::{Stakker, actor, ret_shutdown};
use stakker_mio::MioPoll;
use stakker_mio::mio::{Events, Poll};
use std::time::{Duration, Instant};

mod app;
mod font;
use app::App;
use font::Font;

// Let main loop dump setup I/O errors to console.  You could catch
// them instead if you want.
fn main() -> std::io::Result<()> {
    let font = if let Some(fnam) = std::env::args().nth(1) {
        if fnam.starts_with('-') {
            println!("Usage: cargo run --example fontedit [<filename.txt>]");
            println!("  Run with filename that doesn't exist to create a new font.");
            println!("  Also exports BDF in parallel with readable font file.");
            return Ok(());
        }
        Font::load_or_create(fnam.into()).unwrap() // Panic on load failure
    } else {
        Font::demo()
    };

    let mut stakker = Stakker::new(Instant::now());
    let s = &mut stakker;
    let miopoll = MioPoll::new(s, Poll::new()?, Events::with_capacity(1024), 0)?;

    let _app = actor!(s, App::init(font), ret_shutdown!(s));

    const MAXDELAY: Duration = Duration::from_secs(60);
    let mut idle_pending = s.run(Instant::now(), false);
    let mut io_pending = false;
    let mut activity;
    while s.not_shutdown() {
        let maxdur = s.next_wait_max(Instant::now(), MAXDELAY, idle_pending || io_pending);
        (activity, io_pending) = miopoll.poll(maxdur)?;
        idle_pending = s.run(Instant::now(), !activity);
    }
    if let Some(reason) = s.shutdown_reason()
        && reason.has_error()
    {
        println!("{reason}");
    }

    Ok(())
}
