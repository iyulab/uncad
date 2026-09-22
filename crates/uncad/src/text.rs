//! Decoding of the inline codes AutoCAD stores in text: MTEXT's `\P`, `\S`
//! stacked fractions, `{...}` groups and `\A`/`\H`/`\f`-style format codes,
//! plus the `%%c` / `%%d` / `%%p` / `%%u` symbol and toggle codes TEXT,
//! ATTRIB and dimension labels use.
//!
//! The model keeps the raw string (`text`) and the decoded one
//! (`text_plain`) side by side: the raw one is what the file says, the plain
//! one is what a reader sees. The renderer draws the plain one. Getting this
//! right matters for numbers: `3{\H0.7x;\S1#2;}"` is three and a half
//! inches, and the earlier code-stripping turned it into `31/2"`.
//!
//! Not attempted: `\M+nXXXX` multibyte escapes (LibreDWG expands them for
//! pre-R2007 files; in an R2007+ file they are left as written), `\p...;`
//! paragraph properties beyond dropping them, and any visual formatting.

use serde::{Deserialize, Serialize};

/// A text decoration one of the toggle codes switched on somewhere in the
/// string (`\L`/`\l`, `%%u` underline; `\O`/`\o`, `%%o` overline; `\K`/`\k`
/// strike-through). Recorded per string, not per character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decoration {
    Underline,
    Overline,
    Strikethrough,
}

/// Which separator a `\S` stack used, i.e. how AutoCAD draws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StackKind {
    /// `\S1/2;` -- a horizontal fraction bar (`DIMFRAC` 0).
    Horizontal,
    /// `\S1#2;` -- a diagonal bar (`DIMFRAC` 1).
    Diagonal,
    /// `\S+0.1^-0.2;` -- one value over the other with no bar: a tolerance
    /// stack, or a super/subscript when one side is empty.
    Tolerance,
}

/// One `\S` stack found in an MTEXT string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fraction {
    pub numerator: String,
    pub denominator: String,
    pub kind: StackKind,
}

/// What a raw text string decodes to.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DecodedText {
    /// The readable text. Paragraph breaks (`\P`, `\X`, `\N`) are `\n`;
    /// a stack `\S a#b;` reads as `a/b` (`a^b` for a tolerance stack), with a
    /// space inserted when a digit precedes it so `3 1/2` cannot read as
    /// thirty-one halves.
    pub plain: String,
    pub decorations: Vec<Decoration>,
    pub fractions: Vec<Fraction>,
}

/// Decodes an MTEXT string (also what a dimension's cached label holds).
pub fn decode_mtext(raw: &str) -> DecodedText {
    decode(raw, true)
}

/// Decodes a TEXT / ATTRIB / TOLERANCE string: only the `%%` codes and
/// `\U+XXXX` escapes apply, a backslash is otherwise a backslash.
pub fn decode_text(raw: &str) -> DecodedText {
    decode(raw, false)
}

fn decode(raw: &str, mtext: bool) -> DecodedText {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = DecodedText::default();
    let mut plain = String::with_capacity(raw.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            let code = chars[i + 1];
            // \U+XXXX is the one backslash escape TEXT shares with MTEXT.
            if let Some((ch, len)) = unicode_escape(&chars[i..]) {
                plain.push(ch);
                i += len;
                continue;
            }
            if !mtext {
                plain.push(c);
                i += 1;
                continue;
            }
            match code {
                'P' | 'X' | 'N' => plain.push('\n'),
                '~' => plain.push(' '),
                '\\' | '{' | '}' => plain.push(code),
                'L' => add_decoration(&mut out, Decoration::Underline),
                'O' => add_decoration(&mut out, Decoration::Overline),
                'K' => add_decoration(&mut out, Decoration::Strikethrough),
                'l' | 'o' | 'k' => {}
                'S' => {
                    let end = chars[i + 2..]
                        .iter()
                        .position(|&ch| ch == ';')
                        .map(|p| i + 2 + p);
                    match end {
                        Some(end) => {
                            let body: String = chars[i + 2..end].iter().collect();
                            push_stack(&mut out, &mut plain, &body);
                            i = end + 1;
                            continue;
                        }
                        // Unterminated: nothing sensible to do but keep it.
                        None => plain.push_str("\\S"),
                    }
                }
                // Format codes with a `;` terminator: alignment, colour,
                // font, height, slant, tracking, width, paragraph properties.
                'A' | 'C' | 'c' | 'f' | 'F' | 'H' | 'Q' | 'T' | 'W' | 'p' => {
                    // No terminator: drop just the code letter.
                    if let Some(p) = chars[i + 2..].iter().position(|&ch| ch == ';') {
                        i += 2 + p + 1;
                        continue;
                    }
                }
                // Anything else: an unknown code, or a stray backslash in
                // text that was never escaped. Keep the character.
                other => plain.push(other),
            }
            i += 2;
            continue;
        }
        if mtext && (c == '{' || c == '}') {
            i += 1;
            continue;
        }
        if c == '%' && i + 2 < chars.len() && chars[i + 1] == '%' {
            if let Some((replacement, len)) = percent_code(&mut out, &chars[i..]) {
                plain.push_str(&replacement);
                i += len;
                continue;
            }
        }
        plain.push(c);
        i += 1;
    }
    out.plain = plain;
    out
}

fn add_decoration(out: &mut DecodedText, decoration: Decoration) {
    if !out.decorations.contains(&decoration) {
        out.decorations.push(decoration);
    }
}

/// `\U+XXXX` (four hex digits) at the start of `chars`, as the character and
/// the number of chars consumed.
fn unicode_escape(chars: &[char]) -> Option<(char, usize)> {
    if chars.len() < 7 || chars[0] != '\\' || chars[1] != 'U' || chars[2] != '+' {
        return None;
    }
    let hex: String = chars[3..7].iter().collect();
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let code = u32::from_str_radix(&hex, 16).ok()?;
    char::from_u32(code).map(|ch| (ch, 7))
}

/// The `%%` codes: `%%c` diameter, `%%d` degree, `%%p` plus-minus, `%%%` a
/// percent sign, `%%u`/`%%o` underline/overline toggles (recorded, nothing
/// emitted), `%%nnn` a character by code (Latin-1 for the upper half, which
/// matches the Western code pages these codes were written for). Returns
/// the replacement and the number of chars consumed, or `None` when the
/// `%%` is not a code.
fn percent_code(out: &mut DecodedText, chars: &[char]) -> Option<(String, usize)> {
    let code = chars.get(2)?;
    match code.to_ascii_lowercase() {
        'c' => Some(("\u{2205}".to_string(), 3)),
        'd' => Some(("\u{00B0}".to_string(), 3)),
        'p' => Some(("\u{00B1}".to_string(), 3)),
        '%' => Some(("%".to_string(), 3)),
        'u' => {
            add_decoration(out, Decoration::Underline);
            Some((String::new(), 3))
        }
        'o' => {
            add_decoration(out, Decoration::Overline);
            Some((String::new(), 3))
        }
        _ if code.is_ascii_digit() => {
            let digits: String = chars[2..].iter().take(3).collect();
            if digits.len() == 3 && digits.chars().all(|c| c.is_ascii_digit()) {
                let n: u32 = digits.parse().ok()?;
                let ch = if n < 256 {
                    char::from_u32(n)?
                } else {
                    '\u{FFFD}'
                };
                Some((ch.to_string(), 5))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Emits the plain form of a `\S` stack body (`1#2`, `1/2`, `+0.1^-0.2`,
/// `2^` ...) and records it.
fn push_stack(out: &mut DecodedText, plain: &mut String, body: &str) {
    let (kind, sep) = if let Some(p) = body.find('^') {
        (StackKind::Tolerance, p)
    } else if let Some(p) = body.find('#') {
        (StackKind::Diagonal, p)
    } else if let Some(p) = body.find('/') {
        (StackKind::Horizontal, p)
    } else {
        // No separator: AutoCAD shows the text unstacked.
        plain.push_str(body);
        return;
    };
    let numerator = body[..sep].trim().to_string();
    let denominator = body[sep + 1..].trim().to_string();

    // "3" immediately followed by "1/2" would read as thirty-one halves.
    if plain.chars().last().is_some_and(|c| c.is_ascii_digit()) {
        plain.push(' ');
    }
    match (numerator.is_empty(), denominator.is_empty()) {
        (true, true) => {}
        (false, true) => plain.push_str(&numerator),
        (true, false) => plain.push_str(&denominator),
        (false, false) => {
            plain.push_str(&numerator);
            plain.push(if kind == StackKind::Tolerance {
                '^'
            } else {
                '/'
            });
            plain.push_str(&denominator);
        }
    }
    out.fractions.push(Fraction {
        numerator,
        denominator,
        kind,
    });
}

// --- text boxes ---------------------------------------------------------

/// The advance the estimates assume per character, in text heights: 0.6 em
/// of the font size the renderer draws at, which is the CAD height (the cap
/// height) over the bundled face's cap-height ratio -- 0.6 / 0.733 = 0.8186
/// heights.
pub const CHAR_ADVANCE: f64 = 0.6 / crate::png::BUNDLED_CAP_HEIGHT;

/// AutoCAD's single line spacing for MTEXT (and the multi-line estimate):
/// 5/3 of the text height from one baseline to the next.
pub const LINE_SPACING: f64 = 5.0 / 3.0;

/// Estimated world box of a TEXT/ATTRIB: [`CHAR_ADVANCE`] heights per
/// character (the renderer's own guess; glyph metrics refine it later),
/// one text height tall per line (the cap height; descenders are not
/// counted), placed by its alignment (DXF 72 / 73) and rotated about its
/// anchor. The renderer's extents and the export's records both use this.
pub fn estimate_text_box(
    anchor: crate::model::Point2D,
    height: f64,
    rotation: f64,
    text: &str,
    width_factor: f64,
    h_align: u16,
    v_align: u16,
) -> crate::crop::Rect {
    let lines: Vec<&str> = text.split('\n').collect();
    let chars = lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0)
        .max(1) as f64;
    let width = CHAR_ADVANCE * height * width_factor.abs().max(0.1) * chars;
    let total_height = height * (1.0 + (lines.len().max(1) - 1) as f64 * LINE_SPACING);
    let (x0, x1) = match h_align {
        1 | 3 | 4 | 5 => (-width / 2.0, width / 2.0),
        2 => (-width, 0.0),
        _ => (0.0, width),
    };
    let (y0, y1) = match (h_align, v_align) {
        (4, _) | (_, 2) => (-total_height / 2.0, total_height / 2.0),
        (_, 3) => (-total_height, 0.0),
        _ => (0.0, total_height),
    };
    rotated_box(anchor, rotation, x0, y0, x1, y1)
}

/// MTEXT: attachment 1-9 (top/middle/bottom rows, left/center/right
/// columns) with the stored extents when the file has them.
pub fn estimate_mtext_box(
    anchor: crate::model::Point2D,
    height: f64,
    rotation: f64,
    text: &str,
    attachment: u16,
    extents_width: f64,
    extents_height: f64,
) -> crate::crop::Rect {
    let lines: Vec<&str> = text.split('\n').collect();
    let chars = lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0)
        .max(1) as f64;
    let width = if extents_width > 0.0 {
        extents_width
    } else {
        CHAR_ADVANCE * height * chars
    };
    let total_height = if extents_height > 0.0 {
        extents_height
    } else {
        height * (1.0 + (lines.len().max(1) - 1) as f64 * LINE_SPACING)
    };
    let column = (attachment.clamp(1, 9) - 1) % 3;
    let row = (attachment.clamp(1, 9) - 1) / 3;
    let (x0, x1) = match column {
        1 => (-width / 2.0, width / 2.0),
        2 => (-width, 0.0),
        _ => (0.0, width),
    };
    let (y0, y1) = match row {
        0 => (-total_height, 0.0),
        1 => (-total_height / 2.0, total_height / 2.0),
        _ => (0.0, total_height),
    };
    rotated_box(anchor, rotation, x0, y0, x1, y1)
}

pub(crate) fn rotated_box(
    anchor: crate::model::Point2D,
    rotation: f64,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
) -> crate::crop::Rect {
    let (c, s) = (rotation.cos(), rotation.sin());
    let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    let mut rect = crate::crop::Rect::new(
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for (x, y) in corners {
        let (wx, wy) = (anchor.x + c * x - s * y, anchor.y + s * x + c * y);
        rect.min_x = rect.min_x.min(wx);
        rect.min_y = rect.min_y.min(wy);
        rect.max_x = rect.max_x.max(wx);
        rect.max_y = rect.max_y.max(wy);
    }
    rect
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Point2D;

    #[test]
    fn text_boxes_follow_alignment_and_rotation() {
        let anchor = Point2D { x: 10.0, y: 20.0 };
        // Four characters of height 2: 4 x 0.6 em at font-size 2 / 0.733,
        // i.e. 4 x 0.6 x 2 / 0.733 = 6.5484 wide, one cap height (2) tall.
        let width: f64 = 4.0 * 0.6 * 2.0 / 0.733;
        assert!((width - 6.548431).abs() < 1e-6, "{width}");
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        // Left/baseline: from the anchor to the right and up.
        let b = estimate_text_box(anchor, 2.0, 0.0, "ABCD", 1.0, 0, 0);
        assert!(
            close(b.min_x, 10.0)
                && close(b.min_y, 20.0)
                && close(b.max_x, 10.0 + width)
                && close(b.max_y, 22.0),
            "{b:?}"
        );
        // Middle-centre.
        let b = estimate_text_box(anchor, 2.0, 0.0, "ABCD", 1.0, 4, 0);
        assert!(close(b.min_x, 10.0 - width / 2.0) && close(b.max_x, 10.0 + width / 2.0));
        assert!(close(b.min_y, 19.0) && close(b.max_y, 21.0));
        // Rotated 90 degrees: the width goes up, the height to the left.
        let b = estimate_text_box(anchor, 2.0, std::f64::consts::FRAC_PI_2, "ABCD", 1.0, 0, 0);
        assert!(close(b.max_y, 20.0 + width) && close(b.min_x, 8.0), "{b:?}");
        // MTEXT top-left attachment hangs below the anchor: two lines are
        // 1 + 5/3 heights = 5.333 tall.
        let b = estimate_mtext_box(anchor, 2.0, 0.0, "AB\nCD", 1, 0.0, 0.0);
        assert!(
            close(b.max_y, 20.0) && close(b.min_y, 20.0 - 2.0 * (1.0 + 5.0 / 3.0)),
            "{b:?}"
        );
        assert!(close(b.max_x, 10.0 + width / 2.0));
    }

    fn plain_mtext(raw: &str) -> String {
        decode_mtext(raw).plain
    }

    #[test]
    fn dimension_labels_keep_their_numbers_readable() {
        // The cases measured on the sample drawings.
        assert_eq!(plain_mtext(r#"\A1;7'-4""#), r#"7'-4""#);
        assert_eq!(plain_mtext(r#"3{\H0.7x;\S1#2;}""#), r#"3 1/2""#);
        assert_eq!(plain_mtext(r#"\A1;{\H0.750000x;\S1#2;}""#), r#"1/2""#);
        assert_eq!(
            plain_mtext(r#"\A1;1'-1{\H0.750000x;\S1#8;}""#),
            r#"1'-1 1/8""#
        );
        assert_eq!(plain_mtext(r#"\A1;2"%%C"#), "2\"\u{2205}");
        assert_eq!(plain_mtext(r#"\A1;R10'-0""#), r#"R10'-0""#);
    }

    #[test]
    fn every_stack_separator_is_understood() {
        let d = decode_mtext(r"\S1/2;");
        assert_eq!(d.plain, "1/2");
        assert_eq!(d.fractions[0].kind, StackKind::Horizontal);
        let d = decode_mtext(r"\S1#2;");
        assert_eq!(d.fractions[0].kind, StackKind::Diagonal);
        let d = decode_mtext(r"\S+0.1^-0.2;");
        assert_eq!(d.plain, "+0.1^-0.2");
        assert_eq!(
            d.fractions,
            vec![Fraction {
                numerator: "+0.1".into(),
                denominator: "-0.2".into(),
                kind: StackKind::Tolerance
            }]
        );
        // Super/subscripts are one-sided stacks.
        assert_eq!(plain_mtext(r"m\S2^;"), "m2");
        assert_eq!(plain_mtext(r"H\S^2;"), "H2");
        assert_eq!(plain_mtext(r"\Sabc;"), "abc");
    }

    #[test]
    fn paragraphs_groups_and_format_codes() {
        assert_eq!(plain_mtext(r"A\PB\XC\ND"), "A\nB\nC\nD");
        assert_eq!(plain_mtext(r"\~x"), " x");
        assert_eq!(
            plain_mtext(r"\pi102.25;{\f@Arial Unicode MS|b1|i0|c0|p34;A T M O S}"),
            "A T M O S"
        );
        assert_eq!(plain_mtext(r"\C1;red\C256;bylayer"), "redbylayer");
        assert_eq!(plain_mtext(r"\fArial|b0|i0;\W0.8;\Q10;\T1.2;\H2.5;x"), "x");
        assert_eq!(plain_mtext(r"a\\b\{c\}"), r"a\b{c}");
        assert_eq!(plain_mtext(r"\Lunder\l plain"), "under plain");
        let d = decode_mtext(r"\Lunder\l \Oover\o \Kstrike\k");
        assert_eq!(d.plain, "under over strike");
        assert_eq!(
            d.decorations,
            vec![
                Decoration::Underline,
                Decoration::Overline,
                Decoration::Strikethrough
            ]
        );
        // An unterminated format code drops only its letter.
        assert_eq!(plain_mtext(r"\Hoops"), "oops");
    }

    #[test]
    fn percent_codes_and_unicode_escapes() {
        assert_eq!(decode_text("%%c50").plain, "\u{2205}50");
        assert_eq!(decode_text("108%%d").plain, "108\u{00B0}");
        assert_eq!(decode_text("%%P0.5").plain, "\u{00B1}0.5");
        assert_eq!(decode_text("50%%%").plain, "50%");
        let d = decode_text("%%UBOOK RETURN%%U");
        assert_eq!(d.plain, "BOOK RETURN");
        assert_eq!(d.decorations, vec![Decoration::Underline]);
        assert_eq!(decode_text("%%176").plain, "\u{00B0}");
        assert_eq!(decode_text("%%065").plain, "A");
        assert_eq!(decode_text(r"\U+00B1 3").plain, "\u{00B1} 3");
        assert_eq!(decode_mtext(r"\U+2205 50").plain, "\u{2205} 50");
        // Not codes: a lone percent, a bad escape.
        assert_eq!(decode_text("50%").plain, "50%");
        assert_eq!(decode_text("a%%zb").plain, "a%%zb");
        assert_eq!(decode_text(r"\U+ZZZZ").plain, r"\U+ZZZZ");
    }

    #[test]
    fn text_strings_keep_backslashes_and_braces() {
        assert_eq!(decode_text(r"C:\Temp\{x}").plain, r"C:\Temp\{x}");
        assert_eq!(
            decode_text(r"\P is not a paragraph here").plain,
            r"\P is not a paragraph here"
        );
        assert_eq!(decode_text("").plain, "");
        assert_eq!(decode_mtext("trailing\\").plain, "trailing\\");
    }

    #[test]
    fn korean_text_passes_through() {
        assert_eq!(
            plain_mtext("방 101\\P면적 32.5\u{33A1}"),
            "방 101\n면적 32.5\u{33A1}"
        );
    }
}
