#![allow(clippy::manual_range_contains)]

use std::rc::Rc;

// TODO: Some possible additional `Sizer` implementations, to match
// capabilities of target terminals:
//
// - Combining: in addition interpret combining characters (or some
// combining characters), i.e. assumes that terminal handles them
// correctly
//
// - Right-to-left: maybe handle arab+hebrew script ... somehow
//
// - Wide: also handle CJK / emojis.  Need to know what the terminal
// does and do the same.

/// A `Sizer` is used to measure glyphs in a string, and to mark
/// invalid sequences
///
/// The interface handles both combining characters resulting in a
/// single cell (e.g. accented chars), and single codepoints resulting
/// in multiple cells (e.g. CJK).  Each implementation is intended to
/// model the behaviour of a particular class of terminals.  It also
/// allows rejecting bytes or codepoints which are invalid or not
/// representable on the target terminal and converting them to the
/// replacement character.
///
/// A `Sizer` can be cloned to get another reference to the same
/// implementation.
///
/// See [`SimpleSizer`] for a basic monospace implementation.
///
/// [`SimpleSizer`]: ../sizer/struct.SimpleSizer.html
#[derive(Clone)]
pub struct Sizer {
    #[allow(clippy::type_complexity)]
    sizer: Rc<dyn Fn(&[u8]) -> (usize, i16)>,
}

impl Sizer {
    /// Create a new Sizer.  The given closure will be used to measure
    /// glyphs.  The closure must implement the `Sizer::measure`
    /// operation.
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(&[u8]) -> (usize, i16) + 'static,
    {
        Self { sizer: Rc::new(f) }
    }

    /// Measure the size of the first glyph in the given binary
    /// string.  Returns number of bytes consumed, and the width in
    /// cells.  In the case that the next glyph or codepoint is
    /// invalid in any way, it should grab at least one byte, and
    /// return the negated width of the replacement character
    /// (i.e. typically -1).  The calling code will replace those
    /// bytes with U+FFFD on output to the terminal.
    ///
    /// Note: This code must never grab 0 bytes because that would
    /// result in an endless loop.  Also it must never grab more bytes
    /// than are present, because that will cause a panic.  It will
    /// always be called with at least one byte in the slice `p`.
    pub fn measure(&self, p: &[u8]) -> (usize, i16) {
        (*self.sizer)(p)
    }
}

/// Is this a combining character?
fn is_combining_char_2_byte(c: u16) -> bool {
    matches!(c, 0x0300..=0x036F)
}

/// Is this a combining character?
fn is_combining_char_3_byte(c: u16) -> bool {
    matches!(c,
             0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF |
             0x20D0..=0x20FF | 0xFE20..=0xFE2F)
}

/// Is this a wide character ("F" or "W" classification in the
/// EastAsianWidth.txt file from Unicode 15.0)
fn is_wide_char_3_byte(c: u16) -> bool {
    match c {
        0x1100..=0x115F => true,
        0x2300..=0xFFFF => {
            matches!(
                c,
                0x231A..=0x231B | 0x2329..=0x232A | 0x23E9..=0x23EC |
                0x23F0..=0x23F0 | 0x23F3..=0x23F3 | 0x25FD..=0x25FE |
                0x2614..=0x2615 | 0x2648..=0x2653 | 0x267F..=0x267F |
                0x2693..=0x2693 | 0x26A1..=0x26A1 | 0x26AA..=0x26AB |
                0x26BD..=0x26BE | 0x26C4..=0x26C5 | 0x26CE..=0x26CE |
                0x26D4..=0x26D4 | 0x26EA..=0x26EA | 0x26F2..=0x26F3 |
                0x26F5..=0x26F5 | 0x26FA..=0x26FA | 0x26FD..=0x26FD |
                0x2705..=0x2705 | 0x270A..=0x270B | 0x2728..=0x2728 |
                0x274C..=0x274C | 0x274E..=0x274E | 0x2753..=0x2755 |
                0x2757..=0x2757 | 0x2795..=0x2797 | 0x27B0..=0x27B0 |
                0x27BF..=0x27BF | 0x2B1B..=0x2B1C | 0x2B50..=0x2B50 |
                0x2B55..=0x2B55 | 0x2E80..=0x2E99 | 0x2E9B..=0x2EF3 |
                0x2F00..=0x2FD5 | 0x2FF0..=0x2FFB | 0x3000..=0x303E |
                0x3041..=0x3096 | 0x3099..=0x30FF | 0x3105..=0x312F |
                0x3131..=0x318E | 0x3190..=0x31E3 | 0x31F0..=0x321E |
                0x3220..=0x3247 | 0x3250..=0x4DBF | 0x4E00..=0xA48C |
                0xA490..=0xA4C6 | 0xA960..=0xA97C | 0xAC00..=0xD7A3 |
                0xF900..=0xFAFF | 0xFE10..=0xFE19 | 0xFE30..=0xFE52 |
                0xFE54..=0xFE66 | 0xFE68..=0xFE6B | 0xFF01..=0xFF60 |
                0xFFE0..=0xFFE6)
        }
        _ => false,
    }
}

/// Is this a wide character ("F" or "W" classification in the
/// EastAsianWidth.txt file from Unicode 15.0)
fn is_wide_char_4_byte(c: u32) -> bool {
    match c {
        0x16000..=0x3FFFF => {
            matches!(
                c,
                0x16FE0..=0x16FE4 | 0x16FF0..=0x16FF1 | 0x17000..=0x187F7 |
                0x18800..=0x18CD5 | 0x18D00..=0x18D08 | 0x1AFF0..=0x1AFF3 |
                0x1AFF5..=0x1AFFB | 0x1AFFD..=0x1AFFE | 0x1B000..=0x1B122 |
                0x1B132..=0x1B132 | 0x1B150..=0x1B152 | 0x1B155..=0x1B155 |
                0x1B164..=0x1B167 | 0x1B170..=0x1B2FB | 0x1F004..=0x1F004 |
                0x1F0CF..=0x1F0CF | 0x1F18E..=0x1F18E | 0x1F191..=0x1F19A |
                0x1F200..=0x1F202 | 0x1F210..=0x1F23B | 0x1F240..=0x1F248 |
                0x1F250..=0x1F251 | 0x1F260..=0x1F265 | 0x1F300..=0x1F320 |
                0x1F32D..=0x1F335 | 0x1F337..=0x1F37C | 0x1F37E..=0x1F393 |
                0x1F3A0..=0x1F3CA | 0x1F3CF..=0x1F3D3 | 0x1F3E0..=0x1F3F0 |
                0x1F3F4..=0x1F3F4 | 0x1F3F8..=0x1F43E | 0x1F440..=0x1F440 |
                0x1F442..=0x1F4FC | 0x1F4FF..=0x1F53D | 0x1F54B..=0x1F54E |
                0x1F550..=0x1F567 | 0x1F57A..=0x1F57A | 0x1F595..=0x1F596 |
                0x1F5A4..=0x1F5A4 | 0x1F5FB..=0x1F64F | 0x1F680..=0x1F6C5 |
                0x1F6CC..=0x1F6CC | 0x1F6D0..=0x1F6D2 | 0x1F6D5..=0x1F6D7 |
                0x1F6DC..=0x1F6DF | 0x1F6EB..=0x1F6EC | 0x1F6F4..=0x1F6FC |
                0x1F7E0..=0x1F7EB | 0x1F7F0..=0x1F7F0 | 0x1F90C..=0x1F93A |
                0x1F93C..=0x1F945 | 0x1F947..=0x1F9FF | 0x1FA70..=0x1FA7C |
                0x1FA80..=0x1FA88 | 0x1FA90..=0x1FABD | 0x1FABF..=0x1FAC5 |
                0x1FACE..=0x1FADB | 0x1FAE0..=0x1FAE8 | 0x1FAF0..=0x1FAF8 |
                0x20000..=0x2FFFD | 0x30000..=0x3FFFD)
        }
        _ => false,
    }
}

/// Glyph sizer for simple monospace terminals
///
/// This is intended to represent the lowest common denominator of
/// terminals.  This assumes that the terminal uses monospace
/// characters, with no handling of double-width characters, and no
/// handling of combining characters.  Any double-width or combining
/// codepoint is converted to the replacement character before it is
/// sent to the terminal, to avoid any unexpected behaviour from the
/// terminal which may cause misalignment.  Also this assumes that the
/// terminal will show any characters it doesn't have in its font as a
/// single monospace replacement character.  This will work with
/// terminals with more features, but won't take advantage of those
/// features.
// TODO: Maybe convert some other character classes to U+FFFD,
// i.e. anything else the terminal might interpret unreliably,
// e.g. zero-width codepoints or whatever
pub struct SimpleSizer;

impl SimpleSizer {
    /// Create a simple-sizer `Sizer`
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Sizer {
        Sizer::new(Self::measure)
    }

    fn measure(p: &[u8]) -> (usize, i16) {
        if let Some(c0) = p.first() {
            if *c0 < 0x80 {
                // One-byte UTF-8
                if *c0 < 0x20 {
                    return (1, -1);
                }
                return (1, 1);
            }
            if *c0 < 0xC0 {
                // UTF-8 continuation byte in invalid position
                return (1, -1);
            }
            if let Some(c1 @ 0x80..=0xBF) = p.get(1) {
                if *c0 < 0xE0 {
                    // Two-byte UTF-8
                    let cp = ((*c0 as u16 & 0x1F) << 6) | (*c1 as u16 & 0x3F);
                    if cp < 0x80 || is_combining_char_2_byte(cp) {
                        return (2, -1);
                    }
                    return (2, 1);
                }
                if let Some(c2 @ 0x80..=0xBF) = p.get(2) {
                    if *c0 < 0xF0 {
                        // Three-byte UTF-8
                        let cp = ((*c0 as u16 & 0x0F) << 12)
                            | ((*c1 as u16 & 0x3F) << 6)
                            | (*c2 as u16 & 0x3F);
                        if cp < 0x800
                            || (cp >= 0xD800 && cp <= 0xDFFF)
                            || is_combining_char_3_byte(cp)
                            || is_wide_char_3_byte(cp)
                        {
                            return (3, -1);
                        }
                        return (3, 1);
                    }
                    if let Some(c3 @ 0x80..=0xBF) = p.get(3)
                        && *c0 < 0xF8
                    {
                        // Four-byte UTF-8
                        let cp = ((*c0 as u32 & 0x07) << 18)
                            | ((*c1 as u32 & 0x3F) << 12)
                            | ((*c2 as u32 & 0x3F) << 6)
                            | (*c3 as u32 & 0x3F);
                        if cp < 0x10000 || cp > 0x10FFFF || is_wide_char_4_byte(cp) {
                            return (4, -1);
                        }
                        return (4, 1);
                    }
                }
            }
        }
        // Any other problem, consume one byte and display a
        // replacement character.
        (1, -1)
    }
}
