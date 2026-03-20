use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::PathBuf;

#[derive(Clone)]
pub struct Font {
    pub path: PathBuf,
    pub sx: usize,
    pub sy: usize,
    pub rule_bm: u64,
    pub chars: Vec<Char>,
}

#[derive(Clone)]
pub struct Char {
    pub cp: u32,
    pub form: Vec<bool>,
}

impl Font {
    /// Load font from simple text-based format
    pub fn load(path: PathBuf) -> Result<Self, String> {
        let data =
            std::fs::read_to_string(&path).map_err(|e| format!("Failed to read file: {e}"))?;
        Self::load_aux(path, data).ok_or_else(|| "Failed to parse file".into())
    }

    /// Load font from simple text-based format, or create blank font if missing
    pub fn load_or_create(path: PathBuf) -> Result<Self, String> {
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self::load_aux(path, EMPTY_FONT.into()).unwrap())
        }
    }

    /// Generate demo font
    pub fn demo() -> Self {
        Self::load_aux("demo-font.txt".into(), DEMO_FONT.into()).unwrap()
    }

    fn load_aux(path: PathBuf, data: String) -> Option<Self> {
        let mut lines = data.split('\n');
        let head = lines.next()?;
        let mut it = head.split_ascii_whitespace();
        if it.next()? != "stakker_tui::fontedit::font" {
            return None;
        }
        let sx = it.next()?.strip_prefix("sx=")?.parse::<usize>().ok()?;
        let sy = it.next()?.strip_prefix("sy=")?.parse::<usize>().ok()?;
        let rule_bm = it.next()?.strip_prefix("rule_bm=")?.parse::<u64>().ok()?;
        let mut chars: Vec<Char> = Vec::new();
        let mut form = Vec::new();
        let mut cp = !0;
        let mut y = 0;
        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if cp == !0 {
                if line == "+" {
                    cp = chars.last().unwrap().cp + 1;
                } else if let Some(hex) = line.strip_prefix("U+") {
                    cp = u32::from_str_radix(hex, 16).ok()?;
                } else {
                    cp = line.parse::<u32>().ok()?;
                }
                continue;
            }
            let mut x = 0;
            for ch in line.as_bytes() {
                let dot = match ch {
                    b'0' => true,
                    b'-' | b'=' => false,
                    _ => continue,
                };
                form.push(dot);
                x += 1;
                if x == sx {
                    break;
                }
            }
            form.resize(form.len() + (sx - x), false);
            y += 1;
            if y == sy {
                chars.push(Char { cp, form });
                form = Vec::new();
                cp = !0;
                y = 0;
            }
        }

        chars.sort_by_key(|c| c.cp);

        Some(Self {
            path,
            sx,
            sy,
            rule_bm,
            chars,
        })
    }

    /// Save font in simple text-based format and also in BDF
    pub fn save(&self) -> std::io::Result<()> {
        let mut buf = String::new();
        let out = &mut buf;

        let _ = writeln!(
            out,
            "stakker_tui::fontedit::font sx={} sy={} rule_bm={}",
            self.sx, self.sy, self.rule_bm,
        );

        for ch in &self.chars {
            let _ = writeln!(out, "U+{:04X}", ch.cp);
            let mut y = 0;
            let mut x = 0;
            for &dot in &ch.form {
                if dot {
                    out.push('0');
                } else if ((self.rule_bm >> y) & 1) != 0 {
                    out.push('=');
                } else {
                    out.push('-');
                }
                x += 1;
                if x == self.sx {
                    out.push('\n');
                    x = 0;
                    y += 1;
                }
            }
        }

        std::fs::write(&self.path, buf.as_bytes())?;

        // BDF output
        let mut buf = String::new();
        let out = &mut buf;
        let count = self.index_limit();
        let sx = self.sx;
        let sy = self.sy;
        let sx10 = sx * 10;
        let sy10 = sy * 10;

        let mut map = HashMap::new();
        let mut set = HashSet::new();
        for ch in &self.chars {
            map.insert(ch.cp, ch.clone());
            if ch.cp >= 0x20 {
                set.insert(ch.cp);
            }
        }
        for cp in 0x20..=0x7E {
            set.insert(cp);
        }
        for cp in 0xA0..=0x2E7F {
            set.insert(cp);
        }
        let hex = (0..16)
            .map(|i| map.get(&i).unwrap_or(&self.chars[0]).clone())
            .collect::<Vec<Char>>();
        let braille = map.get(&0x10).unwrap_or(&self.chars[0]).clone();
        let mut codepoints = set.iter().copied().collect::<Vec<_>>();
        codepoints.sort();

        // Measure font.  Assume that there are rules for
        // bottom-of-descender, baseline, and top-of lowercase without
        // ascender (x-height).
        let rules = self.rules();
        let (ascent, xheight) = if rules.len() >= 3 {
            let len = rules.len();
            (rules[len - 2], rules[len - 2] - rules[len - 3] + 1)
        } else {
            (sy * 3 / 4, sy / 2) // Rough estimate
        };
        let descent = sy - ascent;

        let _ = writeln!(out, "STARTFONT 2.1");
        let _ = writeln!(
            out,
            "FONT -myfont-Uazu-Medium-R-Normal--{sy}-{sy10}-75-75-C-{sx10}-ISO10646"
        );
        let _ = writeln!(out, "SIZE {sy} 75 75");
        let _ = writeln!(out, "FONTBOUNDINGBOX {sx} {sy} 0 0");
        let _ = writeln!(out, "STARTPROPERTIES 23");
        let _ = writeln!(out, "FONTNAME_REGISTRY \"\"");
        let _ = writeln!(out, "FOUNDRY \"MyFoundry\"");
        let _ = writeln!(out, "FAMILY_NAME \"Uazu\"");
        let _ = writeln!(out, "WEIGHT_NAME \"Medium\"");
        let _ = writeln!(out, "SLANT \"R\"");
        let _ = writeln!(out, "SETWIDTH_NAME \"Normal\"");
        let _ = writeln!(out, "ADD_STYLE_NAME \"\"");
        let _ = writeln!(out, "PIXEL_SIZE {sy}");
        let _ = writeln!(out, "POINT_SIZE {sy10}");
        let _ = writeln!(out, "RESOLUTION_X 75");
        let _ = writeln!(out, "RESOLUTION_Y 75");
        let _ = writeln!(out, "SPACING \"C\"");
        let _ = writeln!(out, "AVERAGE_WIDTH {sx10}");
        let _ = writeln!(out, "CHARSET_REGISTRY \"ISO10646\""); // Unicode
        let _ = writeln!(out, "CHARSET_ENCODING \"1\"");
        let _ = writeln!(out, "COPYRIGHT \"Copyright\"");
        let _ = writeln!(out, "WEIGHT 10");
        let _ = writeln!(out, "RESOLUTION 103");
        let _ = writeln!(out, "X_HEIGHT {xheight}");
        let _ = writeln!(out, "QUAD_WIDTH {sx}");
        let _ = writeln!(out, "FONT_ASCENT {ascent}");
        let _ = writeln!(out, "FONT_DESCENT {descent}");
        let _ = writeln!(out, "DEFAULT_CHAR 65533"); // Replacement character
        let _ = writeln!(out, "ENDPROPERTIES");
        let _ = writeln!(out, "CHARS {}", count);
        for cp in codepoints {
            let _ = writeln!(out, "STARTCHAR U+{:04X}", cp);
            let _ = writeln!(out, "ENCODING {}", cp);
            let _ = writeln!(out, "SWIDTH {} 0", self.sx * 1000 / self.sy);
            let _ = writeln!(out, "DWIDTH {} 0", self.sx);
            let _ = writeln!(out, "BBX {} {} 0 0", self.sx, self.sy);
            let _ = writeln!(out, "BITMAP");
            let chr;
            let ch = match map.get(&cp) {
                Some(ch) => ch,
                None => {
                    if cp >= 0x2800 && cp <= 0x28FF {
                        chr = self.generate_braille_glyph(&braille, cp);
                    } else {
                        chr = self.generate_hex_glyph(&hex, cp);
                    }
                    &chr
                }
            };
            for y in 0..self.sy {
                let base = y * self.sx;
                let mut bm = ch.form[base..base + self.sx].to_vec();
                bm.resize((bm.len() + 7) & !7, false);
                for p in bm.chunks_exact(4) {
                    out.push(
                        b"0123456789ABCDEF"[(((p[0] as u8) << 3)
                            + ((p[1] as u8) << 2)
                            + ((p[2] as u8) << 1)
                            + (p[3] as u8)) as usize] as char,
                    );
                }
                out.push('\n');
            }
            let _ = writeln!(out, "ENDCHAR");
        }
        let _ = writeln!(out, "ENDFONT");

        let mut path = self.path.clone();
        path.set_extension("bdf");
        std::fs::write(&path, buf.as_bytes())
    }

    /// Return y-positions of rules in order
    fn rules(&self) -> Vec<usize> {
        (0..self.sy)
            .filter(|i| ((self.rule_bm >> i) & 1) != 0)
            .collect()
    }

    /// Index limit (maximum index value +1)
    pub fn index_limit(&self) -> usize {
        self.chars.len()
    }

    /// Get pixel value.  Will panic or give random pixel if out of
    /// range.
    pub fn get(&self, index: usize, y: usize, x: usize) -> bool {
        self.chars[index].form[x + self.sx * y]
    }

    /// Set pixel value.  Will panic or change random pixel if out of
    /// range.
    pub fn set(&mut self, index: usize, y: usize, x: usize, val: bool) {
        self.chars[index].form[x + self.sx * y] = val;
    }

    /// Flood fill starting at the given point
    pub fn fill(&mut self, index: usize, y: usize, x: usize, vert: bool, horiz: bool) {
        let bg = self.get(index, y, x);
        let fg = !bg;
        let mut stack = vec![(y, x)];
        while let Some((y, x)) = stack.pop() {
            self.set(index, y, x, fg);
            if vert {
                if y > 0 && self.get(index, y - 1, x) == bg {
                    stack.push((y - 1, x));
                }
                if y + 1 < self.sy && self.get(index, y + 1, x) == bg {
                    stack.push((y + 1, x));
                }
            }
            if horiz {
                if x > 0 && self.get(index, y, x - 1) == bg {
                    stack.push((y, x - 1));
                }
                if x + 1 < self.sx && self.get(index, y, x + 1) == bg {
                    stack.push((y, x + 1));
                }
            }
        }
    }

    /// Roll font vertically
    pub fn roll_vert(&mut self, index: usize, dy: i32) {
        if dy < 0 {
            self.chars[index].form.rotate_left(self.sx);
        } else {
            self.chars[index].form.rotate_right(self.sx);
        }
    }

    /// Roll font horizontally
    pub fn roll_horiz(&mut self, index: usize, dx: i32) {
        let sx = self.sx;
        let sy = self.sy;
        let ch = &mut self.chars[index];
        for i in (0..sy * sx).step_by(sx) {
            if dx < 0 {
                ch.form[i..i + sx].rotate_left(1);
            } else {
                ch.form[i..i + sx].rotate_right(1);
            }
        }
    }

    /// Add a new glyph at the end
    pub fn add_glyph(&mut self) {
        let cp = self.chars.last().unwrap().cp + 1;
        self.chars.push(Char {
            cp,
            form: vec![false; self.sx * self.sy],
        });
    }

    /// Copy out the bitmap of the glyph at the given index
    pub fn get_glyph(&self, index: usize) -> Vec<bool> {
        self.chars[index].form.clone()
    }

    /// Overwrite the glyph at the given index with the bitmap provided
    pub fn set_glyph(&mut self, index: usize, data: &[bool]) {
        self.chars[index].form = data.to_vec();
    }

    // Given 16 glyphs (0-F) each with that hex digit repeated in the
    // four quadrants, generate glyph showing this 16-bit codepoint in
    // hex.  Used for undefined glyphs in a certain range.
    fn generate_hex_glyph(&self, hex: &[Char], cp: u32) -> Char {
        let mut form = vec![false; self.sx * self.sy];
        for (py, px, dig) in [
            (0, 0, (cp >> 12) & 15),
            (0, 1, (cp >> 8) & 15),
            (1, 0, (cp >> 4) & 15),
            (1, 1, cp & 15),
        ] {
            let sy = self.sy;
            let sx = self.sx;
            let sy2 = sy / 2;
            let sx2 = sx / 2;
            let y0 = py * sy2;
            let y1 = py * sy2 + sy2;
            let x0 = px * sx2;
            let x1 = px * sx2 + sx2;
            for y in y0..y1 {
                let o0 = y * sx + x0;
                let o1 = y * sx + x1;
                form[o0..o1].copy_from_slice(&hex[dig as usize].form[o0..o1]);
            }
        }

        Char { cp, form }
    }

    // Given a single braille glyph with all 8 points showing,
    // generate a braille glyph for the given braille codepoint
    fn generate_braille_glyph(&self, braille: &Char, cp: u32) -> Char {
        let mut form = vec![false; self.sx * self.sy];

        // Bit order is unusual: 0,3 / 1,4 / 2,5 / 6,7
        for (py, px, show) in [
            (0, 0, 0x01),
            (0, 1, 0x08),
            (1, 0, 0x02),
            (1, 1, 0x10),
            (2, 0, 0x04),
            (2, 1, 0x20),
            (3, 0, 0x40),
            (3, 1, 0x80),
        ] {
            if 0 != (show & cp) {
                let sy = self.sy;
                let sx = self.sx;
                let y0 = py * sy / 4;
                let y1 = (py + 1) * sy / 4;
                let x0 = px * sx / 2;
                let x1 = (px + 1) * sx / 2;
                for y in y0..y1 {
                    let o0 = y * sx + x0;
                    let o1 = y * sx + x1;
                    form[o0..o1].copy_from_slice(&braille.form[o0..o1]);
                }
            }
        }

        Char { cp, form }
    }
}

const EMPTY_FONT: &str = "stakker_tui::fontedit::font sx=8 sy=16 rule_bm=33797
32
========
--------
========
--------
--------
--------
--------
--------
--------
--------
========
--------
--------
--------
--------
========
";

const DEMO_FONT: &str = "stakker_tui::fontedit::font sx=8 sy=16 rule_bm=33797
48
========
--------
=00000==
00---00-
00---00-
00-0-00-
00-0-00-
00-0-00-
00---00-
00---00-
=00000==
--------
--------
--------
--------
========
49
========
--------
===00===
-0000---
---00---
---00---
---00---
---00---
---00---
---00---
=000000=
--------
--------
--------
--------
========
50
========
--------
=00000==
00---00-
00---00-
-----00-
---000--
-000----
00------
00---00-
0000000=
--------
--------
--------
--------
========
51
========
--------
=00000==
00---00-
-----00-
-----00-
--0000--
-----00-
-----00-
00---00-
=00000==
--------
--------
--------
--------
========
52
========
-----0--
====00==
---000--
--0000--
-00-00--
00--00--
0000000-
----00--
----00--
===0000=
--------
--------
--------
--------
========
53
========
--------
0000000=
00------
00------
000000--
-----00-
-----00-
-----00-
00---00-
=00000==
--------
--------
--------
--------
========
54
========
--------
=00000==
00---00-
00------
00------
000000--
00---00-
00---00-
00---00-
=00000==
--------
--------
--------
--------
========
55
========
--------
0000000=
00---00-
-----00-
-----00-
----00--
---00---
--00----
--00----
==00====
--------
--------
--------
--------
========
56
========
--------
=00000==
00---00-
00---00-
00---00-
-00000--
00---00-
00---00-
00---00-
=00000==
--------
--------
--------
--------
========
57
========
--------
=00000==
00---00-
00---00-
00---00-
-000000-
-----00-
-----00-
00---00-
=00000==
--------
--------
--------
--------
========
";
