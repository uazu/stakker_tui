//! ANSI terminal handling for Stakker
//!
//! **This is a work-in-progress.** Only UNIX and the first level
//! described below is supported for now.
//!
//! Eventually this will provide several levels of abstraction at
//! which the application may interface to the terminal, from lowest
//! to highest:
//!
//! ## Output buffering, input decoding, resizes and features
//!
//! This can be used for applications which prefer to generate the
//! ANSI output sequences themselves directly, for example a pager or
//! a simple editor.  The application has maximum control and can use
//! specific ANSI features to optimise its output (for example scroll
//! regions).
//!
//! The input handling decodes keypress sequences and forwards them to
//! application code.  Terminal resizes are detected and notified as
//! soon as they occur.  Terminal features such as 256 colour support
//! are detected and notified to the application.
//!
//! ## Full-screen page buffering and minimised updates
//!
//! The application code keeps one or more full-screen pages in memory
//! which it updates locally, and the terminal code keeps its own page
//! which represents what is currently displayed on the terminal.
//! When the application code wishes to update the terminal, the
//! terminal code compares the two pages and sends a minimised update.
//!
//! Input handling, resizes and features are handled the same as
//! above.
//!
//! ## Immediate mode UI
//!
//! This will provide an immediate-mode UI (fields, widgets) on top of
//! a full-screen buffer.

#![deny(rust_2018_idioms)]

mod key;
mod terminal;
mod termout;

pub use key::Key;
pub use terminal::Terminal;
pub use termout::{Features, TermOut};

#[cfg(unix)]
mod os_mio_unix;
#[cfg(unix)]
use os_mio_unix as os_glue;

#[cfg(not(unix))]
std::compile_error!("OS interface not yet implemented on this platform");

#[cfg(feature = "unstable")]
mod page;
#[cfg(feature = "unstable")]
pub use page::{Page, Region};
