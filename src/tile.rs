use crate::{Region, TermShare};
use stakker::Core;

/// A tile of the page
///
/// A [`Tile`] represents exclusive access to a rectangular region of
/// the page.  It may be passed to another actor, and when that actor
/// writes to the tile, the display will be updated automatically
/// using a `lazy!` handler.  This automatically batches changes made
/// by multiple actors into a single terminal update.
///
/// Tiles may be split horizontally or vertically.  The expected
/// pattern is as follows:
///
/// - When the app receives a resize from the [`Terminal`] or when it
///   decides to change its own layout, it creates a top-level tile
///   using [`TermShare::tile`].
///
/// - It breaks this tile into sub-tiles according to the layout of
///   the application
///
/// - The sub-tiles may be passed to other actors responsible for
///   those regions of the screen, for example via a `draw` method on
///   those actors.  In that method the actor should draw the required
///   contents on that [`Tile`].
///
/// - The actor should then save the [`Tile`] to use for any updates
///   to the display from then on, i.e. the actor can update that region
///   of the terminal screen with new information whenever it wants to
///   once it has been given a [`Tile`].
///
/// - The `draw` method of an actor may also break the tile into
///   sub-tiles and pass them on to other actors or other code, as
///   necessary.
///
/// Whenever there is another resize or the app decides to change its
/// layout, the same process repeats.  Each time a new top-level tile
/// is created using [`TermShare::tile`] all previous tiles or
/// sub-tiles are invalidated, and they no longer can be used to write
/// to the terminal.  So new sub-tiles have to be created and passed
/// to the relevant actors for them to continue being able to write to
/// the terminal.  All active tiles are also invalidated when the
/// terminal is paused (with [`Terminal::pause`]) to run an external
/// tool.
///
/// There are various models for how to handle actors related to the
/// page, for example:
///
/// - If the actor is long-lived, then have a method `fn draw(&mut
///   self, cx: CX![], tile: Tile)`, and pass it a new [`Tile`] whenever
///   the layout changes.  The actor can redraw the tile from its state,
///   and optionally pass down sub-tiles to other actors.
///
/// - If the actor doesn't need to hold long-term state, then pass the
///   [`Tile`] in the actor's `init` method, and drop the actor and
///   create a new one whenever the layout changes
///
/// Since old tiles are invalidated, this is tolerant of just about
/// any approach, since old actors with old tiles but not yet fully
/// shutdown won't be able to corrupt the terminal display.
///
/// [`TermShare::tile`]: struct.TermShare.html#method.tile
/// [`Terminal::pause`]: struct.Terminal.html#method.pause
/// [`Terminal`]: struct.Terminal.html
/// [`Tile`]: struct.Tile.html
pub struct Tile {
    pub(crate) share: Option<TermShare>,
    pub(crate) genr: u64,
    pub(crate) oy: i32,
    pub(crate) ox: i32,
    pub(crate) sy: i32,
    pub(crate) sx: i32,
}

impl Tile {
    /// Create a zero-size invalid tile that always returns `None` if
    /// you try to write on it.  This call may be convenient to
    /// initialise an actor's `Tile` struct member before the first
    /// real tile is passed to it.  To create a real tile once you
    /// have access to a [`TermShare`], use [`TermShare::tile`].
    ///
    /// [`TermShare::tile`]: struct.TermShare.html#method.tile
    /// [`TermShare`]: struct.TermShare.html
    pub fn new() -> Self {
        Self {
            share: None,
            genr: 0,
            oy: 0,
            ox: 0,
            sy: 0,
            sx: 0,
        }
    }

    /// Get size of `Tile`: (rows, columns)
    pub fn size(&self) -> (i32, i32) {
        (self.sy, self.sx)
    }

    /// Get number of rows in `Tile` (size-Y)
    pub fn sy(&self) -> i32 {
        self.sy
    }

    /// Get number of columns in `Tile` (size-X)
    pub fn sx(&self) -> i32 {
        self.sx
    }

    /// Split a [`Tile`] in two at a particular X-position (relative
    /// to the tile's left edge).  This consumes the original
    /// [`Tile`].  The X-value is cropped to the size of the tile.
    ///
    /// [`Tile`]: struct.Tile.html
    pub fn split_x(self, x: i32) -> (Self, Self) {
        let x = x.clamp(0, self.sx);
        (
            Self {
                share: self.share.clone(),
                genr: self.genr,
                oy: self.oy,
                ox: self.ox,
                sy: self.sy,
                sx: x,
            },
            Self {
                share: self.share,
                genr: self.genr,
                oy: self.oy,
                ox: self.ox + x,
                sy: self.sy,
                sx: self.sx - x,
            },
        )
    }

    /// Split a [`Tile`] in 3 parts, at the given X-positions
    /// (relative to the tile's left edge).  The X-values are cropped
    /// to the size of the tile.
    ///
    /// [`Tile`]: struct.Tile.html
    pub fn split_xx(self, x0: i32, x1: i32) -> (Self, Self, Self) {
        let (t01, t2) = self.split_x(x1);
        let (t0, t1) = t01.split_x(x0);
        (t0, t1, t2)
    }

    /// Split a [`Tile`] in two at a particular Y-position (relative
    /// to the tile's top edge).  This consumes the original [`Tile`].
    /// The Y-value is cropped to the size of the tile.
    ///
    /// [`Tile`]: struct.Tile.html
    pub fn split_y(self, y: i32) -> (Self, Self) {
        let y = y.clamp(0, self.sy);
        (
            Self {
                share: self.share.clone(),
                genr: self.genr,
                oy: self.oy,
                ox: self.ox,
                sy: y,
                sx: self.sx,
            },
            Self {
                share: self.share,
                genr: self.genr,
                oy: self.oy + y,
                ox: self.ox,
                sy: self.sy - y,
                sx: self.sx,
            },
        )
    }

    /// Split a [`Tile`] in 3 parts, at the given Y-positions
    /// (relative to the tile's top edge).  The Y-values are cropped
    /// to the size of the tile.
    ///
    /// [`Tile`]: struct.Tile.html
    pub fn split_yy(self, y0: i32, y1: i32) -> (Self, Self, Self) {
        let (t01, t2) = self.split_y(y1);
        let (t0, t1) = t01.split_y(y0);
        (t0, t1, t2)
    }

    /// Get a [`Region`] that allows writing to the whole [`Tile`], or
    /// return `None` if this tile is now invalid.
    ///
    /// [`Region`]: struct.Region.html
    /// [`Tile`]: struct.Tile.html
    pub fn full<'a>(&'a mut self, core: &'a mut Core) -> Option<Region<'a>> {
        if let Some(ref mut ts) = self.share {
            let (oy, ox, sy, sx) = (self.oy, self.ox, self.sy, self.sx);
            ts.output(core).get_local_page(self.genr).map(|mut r| {
                r.region_inplace(oy, ox, sy, sx);
                r
            })
        } else {
            None
        }
    }

    /// Get a [`Region`] that allows writing to just part of the
    /// [`Tile`], or return `None` if this tile is now invalid.
    ///
    /// [`Region`]: struct.Region.html
    /// [`Tile`]: struct.Tile.html
    pub fn region<'a>(
        &'a mut self,
        core: &'a mut Core,
        y: i32,
        x: i32,
        sy: i32,
        sx: i32,
    ) -> Option<Region<'a>> {
        if let Some(ref mut ts) = self.share {
            let (self_oy, self_ox, self_sy, self_sx) = (self.oy, self.ox, self.sy, self.sx);
            ts.output(core).get_local_page(self.genr).map(|mut r| {
                r.region_inplace(self_oy, self_ox, self_sy, self_sx);
                r.region_inplace(y, x, sy, sx);
                r
            })
        } else {
            None
        }
    }
}

impl Default for Tile {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Tile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tile")
            .field("genr", &self.genr)
            .field("oy", &self.oy)
            .field("ox", &self.ox)
            .field("sy", &self.sy)
            .field("sx", &self.sx)
            .finish()
    }
}
