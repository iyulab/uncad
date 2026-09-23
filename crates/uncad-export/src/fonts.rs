//! The font the package draws and measures text with: `Uncad Sans`, a
//! Noto Sans KR subset bundled in this crate (see `fonts/README.md` for its
//! coverage, the reason for it and how to rebuild it, and
//! `fonts/OFL-NotoSansKR.txt` for its licence).
//!
//! The renderer bundles no font: it draws text with whatever [`Fonts`] it
//! is handed, and needs to be told the face's capital height, since a CAD
//! text height is the height of the capitals rather than the em. [`bundled`] and [`UNCAD_SANS_CAP_HEIGHT`] are
//! those two things for the bundled face.

use std::sync::{Arc, OnceLock};

use iron_render_cad::Fonts;

/// The bundled face, as the OpenType file it is.
pub static UNCAD_SANS_OTF: &[u8] = include_bytes!("../fonts/UncadSans-Regular.otf");

/// The family name the bundled face is registered under.
pub const UNCAD_SANS_FAMILY: &str = "Uncad Sans";

/// The bundled face's capital height as a fraction of its em: OS/2
/// `sCapHeight` 733 over `head.unitsPerEm` 1000, read from the font's own
/// tables (a unit test checks the value against the embedded bytes).
///
/// Handed to the renderer as `ToSvgOptions::cap_height`, it makes a text of
/// CAD height `h` draw its capitals `h` tall, and it scales the renderer's
/// 0.6-em character advance and 0.2-em descender to what the text box
/// estimates of this crate's earlier form assumed.
pub const UNCAD_SANS_CAP_HEIGHT: f64 = 0.733;

/// [`Fonts::Custom`] holding the bundled face and nothing else: the same
/// pixels and the same text boxes on every machine. Built once per process;
/// the value is cheap to clone (the bytes are shared).
pub fn bundled() -> Fonts {
    static BUNDLED: OnceLock<Fonts> = OnceLock::new();
    BUNDLED
        .get_or_init(|| Fonts::Custom(vec![Arc::from(UNCAD_SANS_OTF)]))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One table of an OpenType font: its bytes, found through the table
    /// directory at the start of the file (a 12-byte header, then 16-byte
    /// records of tag, checksum, offset, length).
    fn otf_table<'a>(font: &'a [u8], tag: &[u8; 4]) -> &'a [u8] {
        let count = usize::from(u16::from_be_bytes([font[4], font[5]]));
        (0..count)
            .map(|i| &font[12 + 16 * i..12 + 16 * (i + 1)])
            .find(|record| &record[..4] == tag)
            .map(|record| {
                let offset = u32::from_be_bytes(record[8..12].try_into().unwrap()) as usize;
                let length = u32::from_be_bytes(record[12..16].try_into().unwrap()) as usize;
                &font[offset..offset + length]
            })
            .unwrap_or_else(|| panic!("the font has no {} table", String::from_utf8_lossy(tag)))
    }

    #[test]
    fn the_cap_height_is_the_one_the_font_states() {
        // head.unitsPerEm is at byte 18; OS/2 sCapHeight (version 2+) at 88.
        let head = otf_table(UNCAD_SANS_OTF, b"head");
        let units_per_em = u16::from_be_bytes([head[18], head[19]]);
        let os2 = otf_table(UNCAD_SANS_OTF, b"OS/2");
        assert!(u16::from_be_bytes([os2[0], os2[1]]) >= 2, "OS/2 v2+");
        let cap_height = i16::from_be_bytes([os2[88], os2[89]]);
        assert_eq!((cap_height, units_per_em), (733, 1000));
        assert_eq!(
            UNCAD_SANS_CAP_HEIGHT,
            f64::from(cap_height) / f64::from(units_per_em)
        );
    }

    #[test]
    fn the_face_is_registered_under_its_own_family_name() {
        // The name table's family records (name ID 1 and 16) say "Uncad
        // Sans", never the Noto name the OFL asks a Modified Version not to
        // pass as: a font database keyed by family must not pick a host's
        // Noto Sans KR over this face, or the other way round.
        let name = otf_table(UNCAD_SANS_OTF, b"name");
        let count = usize::from(u16::from_be_bytes([name[2], name[3]]));
        let storage = usize::from(u16::from_be_bytes([name[4], name[5]]));
        let mut families = Vec::new();
        for i in 0..count {
            let record = &name[6 + 12 * i..6 + 12 * (i + 1)];
            let field = |at: usize| u16::from_be_bytes([record[at], record[at + 1]]);
            let (platform, name_id, length, offset) = (field(0), field(6), field(8), field(10));
            if !(name_id == 1 || name_id == 16) {
                continue;
            }
            let bytes = &name[storage + usize::from(offset)..][..usize::from(length)];
            // Windows records are UTF-16BE; Macintosh ones are single-byte.
            let text = if platform == 3 {
                let units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                String::from_utf16_lossy(&units)
            } else {
                String::from_utf8_lossy(bytes).into_owned()
            };
            families.push(text);
        }
        assert!(!families.is_empty());
        assert!(
            families.iter().all(|f| f == UNCAD_SANS_FAMILY),
            "{families:?}"
        );
    }

    #[test]
    fn the_bundled_fonts_are_built_once_and_share_their_bytes() {
        let (a, b) = (bundled(), bundled());
        assert_eq!(a, b);
        let (Fonts::Custom(a), Fonts::Custom(b)) = (a, b) else {
            panic!("the bundled face is a custom font");
        };
        assert!(Arc::ptr_eq(&a[0], &b[0]), "one copy of the bytes");
        assert_eq!(&*a[0], UNCAD_SANS_OTF);
    }
}
