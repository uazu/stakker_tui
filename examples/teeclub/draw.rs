use super::Card;
use stakker_tui::Region;
use std::io::Write;

pub const CARD_SX: i32 = 6;
pub const CARD_SY: i32 = 5;

/// Draw a card at the given location on the given page
pub fn card(r: &mut Region, y: i32, x: i32, card: Card) {
    let mut r = r.region(y, x, CARD_SY, CARD_SX);
    r.hfb(card.hfb).clear_all();
    r.at(0, 2);
    write!(r, "{}{}", card.num_ch, card.suit_ch).unwrap();
    r.at(CARD_SY - 1, 2);
    write!(r, "{}{}", card.num_ch, card.suit_ch).unwrap();
}

/// Draw the back of a card
pub fn card_back(r: &mut Region, y: i32, x: i32) {
    let mut r = r.region(y, x, CARD_SY, CARD_SX);
    r.hfb(27);
    for y in 0..CARD_SY {
        r.at(y, 0).text("||||||");
    }
}

/// Draw the space where a card could go
pub fn card_space(r: &mut Region, y: i32, x: i32) {
    let mut r = r.region(y, x, CARD_SY, CARD_SX);
    r.hfb(71).clear_all();
}

/// Draw "more cards" indicator
pub fn card_dots(r: &mut Region, y: i32, x: i32) {
    let mut r = r.region(y, x, CARD_SY, CARD_SX);
    r.hfb(70);
    for y in 0..CARD_SY {
        r.at(y, 0).text("::::::");
    }
}

#[cfg(false)]
mod unused {
    use crate::Region;
    const FONT: [&[u8]; 5] = [
        b" 0000    00   00000  00000  00  00 00000   0000  000000  0000   0000        ",
        b"00  00  000       00     00 00  00 00     00         00 00  00 00  00   00  ",
        b"00  00   00    0000    000  000000 00000  00000     00   0000   00000       ",
        b"00  00   00   00         00     00     00 00  00   00   00  00     00   00  ",
        b" 0000   0000  000000 00000      00 00000   0000   00     0000   0000        ",
    ];
    const FONT_CX: usize = 7;
    const FONT_SY: usize = 5;

    /// Draw big numbers to the right of the given start-position.
    /// `glyph` is what is written for each lit cell of the numbers
    pub fn bignum_str(region: &mut Region, y: i32, mut x: i32, hfb: u16, glyph: &str, text: &str) {
        for ch in text.as_bytes() {
            let (off, wid) = match ch {
                b'0'..=b'9' => ((ch - 48) as usize * FONT_CX, FONT_CX),
                b':' => (10 * FONT_CX, FONT_CX - 2),
                _ => (0, 0),
            };
            let mut r = region.region(y, x, wid as i32, FONT_SY as i32);
            r.hfb(hfb);
            for i in 0..FONT_SY {
                let slice = &FONT[i][off..off + wid];
                for j in 0..wid {
                    if slice[j] != b' ' {
                        r.at(i as i32, j as i32).text(glyph);
                    }
                }
            }
            x += wid as i32;
        }
    }

    /// Draw big numbers to the left of the given start-position
    pub fn bignum_str_left(r: &mut Region, y: i32, mut x: i32, hfb: u16, glyph: &str, text: &str) {
        for ch in text.as_bytes() {
            x -= (match ch {
                b'0'..=b'9' => FONT_CX,
                b':' => FONT_CX - 2,
                _ => 0,
            }) as i32;
        }

        bignum_str(r, y, x, hfb, glyph, text);
    }
}
