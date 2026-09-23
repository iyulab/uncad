//! Text fields as UTF-8 `String`s, decoded here from what LibreDWG holds.
//!
//! How a drawing's strings sit in LibreDWG's memory depends on where they
//! came from, and the library's text accessors (`dwg_dynapi_*_utf8text`,
//! `dwg_dynapi_handle_name`) convert only one of the cases:
//!
//! - **R2007 and later, from DWG:** stored as UTF-16 (`TU`); the accessors
//!   convert them to UTF-8 themselves.
//! - **Before R2007:** stored as the file's own 8-bit bytes (`TV`) in the
//!   drawing's codepage -- `header.codepage`, read from the DWG header or
//!   from DXF `$DWGCODEPAGE` -- and handed out unchanged, whatever the
//!   `utf8` in the accessor's name says. Read as UTF-8, every non-ASCII
//!   character of a CP949 or CP1252 drawing came out as mojibake or U+FFFD,
//!   with clean diagnostics.
//! - **R2007 and later, from DXF:** the DXF importer writes every string it
//!   sets through the library's field setter as UTF-16, exactly as for a DWG
//!   -- but the accessors convert only for a DWG, so they hand the UTF-16
//!   out as if it were an 8-bit string, which stops at its first NUL byte
//!   (`*Model_Space` read as `*`). The exceptions are the strings the
//!   importer copies byte for byte: MTEXT's text (groups 1 and 3) and the
//!   HEADER variables, parsed before the importer knows the version. Those
//!   are the file's own bytes -- UTF-8, since an R2007+ DXF is UTF-8 -- and
//!   must never be read as UTF-16, which would look for a 16-bit NUL past
//!   their allocation.
//!
//! [`TextDecoder`] is the one place any of this becomes a `String`. It knows
//! which case the drawing is, reads each stored string in the width it was
//! stored in (`crate::dynapi::StoredWidth`), decodes 8-bit bytes through the
//! library's own codepage tables, and reports whatever it could not decode
//! in `read_diagnostics` (`TEXT_ENCODING: ...`) rather than replacing it in
//! silence: a string that is wrong is never presented as one that is right.
//! Escape sequences in the text (`\U+XXXX`, `\M+nXXXX`) are text as far as
//! the model is concerned and are passed through as written.

use std::cell::RefCell;
use std::ffi::{c_void, CStr};

use crate::dynapi::{self, RawText, StoredWidth};

/// `Dwg_Codepage` values this module treats specially; the numbers are the
/// `header.codepage` values LibreDWG's `codepages.h` fixes them to.
const CP_UTF8: u16 = 0;
/// DOS Shift-JIS: double-byte, although `dwg_codepage_isasian` leaves it out.
const CP_CP932: u16 = 22;
/// DOS Big5: an EUC-style encoding whose lead and trail bytes are all >= 0x80.
const CP_BIG5: u16 = 24;
/// EUC-CN: like Big5, and the library's table for it is indexed by the 7-bit
/// ISO-2022 form of a pair (`0x2121..0x777E`), not by the bytes in the file.
const CP_GB2312: u16 = 31;
const CP_UTF16: u16 = 43;
/// The last codepage the library has a table for (`CP_ANSI_1258`).
const CP_LAST: u16 = 44;

/// Whether the library can decode `codepage` at all. `CP_UNDEFINED` (0xFF,
/// "mostly R11") and anything past the table range must never reach the
/// table lookup, which indexes an array by the codepage number unchecked.
fn has_codepage_tables(codepage: u16) -> bool {
    (1..=CP_LAST).contains(&codepage) && codepage != CP_UTF16
}

/// LibreDWG's `$DWGCODEPAGE`-style name of a codepage (`ANSI_1252`), or
/// `None` when it has none -- a value outside its table, `CP_UNDEFINED`.
pub fn codepage_name(codepage: u16) -> Option<String> {
    // SAFETY: dwg_codepage_dxfstr returns a static string or NULL for every
    // value; it bounds-checks the number itself.
    let ptr = unsafe { libredwg_sys::dwg_codepage_dxfstr(codepage as libredwg_sys::Dwg_Codepage) };
    if ptr.is_null() {
        return None;
    }
    let name = unsafe { CStr::from_ptr(ptr) }.to_string_lossy();
    (!name.is_empty() && name != "undefined").then(|| name.into_owned())
}

/// The name a diagnostic gives a codepage: `ANSI_1252 (30)`, or the bare
/// number when the library has no name for it.
fn codepage_label(codepage: u16) -> String {
    match codepage_name(codepage) {
        Some(name) => format!("{name} ({codepage})"),
        None if codepage == 0xFF => format!("undefined ({codepage})"),
        None => format!("{codepage}"),
    }
}

/// Whether `lead` (a byte >= 0x80) opens a two-byte sequence in the
/// double-byte `codepage`.
///
/// The library's `dwg_codepage_is_twobyte` answers "always" for the DOS-era
/// Big5 and GB2312 pages, ASCII included. Both are EUC-style encodings whose
/// lead and trail bytes are all >= 0x80, so the high bit is the rule there;
/// ASCII never reaches this function at all (see [`decode_codepage`]).
fn opens_pair(codepage: u16, lead: u8) -> bool {
    if codepage == CP_GB2312 || codepage == CP_BIG5 {
        return true;
    }
    // SAFETY: a pure table predicate on a codepage number with tables.
    unsafe { libredwg_sys::dwg_codepage_is_twobyte(codepage as libredwg_sys::Dwg_Codepage, lead) }
}

/// Decodes `bytes` in `codepage` through the library's tables: one byte or,
/// in a double-byte codepage, a lead byte plus its trail byte per
/// character. Returns the text and how many input bytes had no mapping;
/// each of those became U+FFFD in the text.
///
/// The library's own converter (`bit_TV_to_utf8_codepage`) is not used
/// because it writes a NUL for an unmapped character -- which cuts the
/// string short at that point -- and returns its input aliased rather than
/// copied in some cases. The loop here is the same walk with the unmapped
/// count kept, and with three corrections to how the library reads the
/// DOS-era double-byte pages:
///
/// - **ASCII is never looked up.** No double-byte page has a lead byte below
///   0x80, and the tables' only differences there -- 0x5C as a yen sign in
///   CP932 and a won sign in JOHAB -- would turn the backslash of MTEXT's
///   `\P` and of every `\U+XXXX` escape into a currency sign. So a byte
///   below 0x80 is itself, in every codepage.
/// - **Big5 and GB2312 pair only bytes >= 0x80** (see [`opens_pair`]). The
///   library pairs every byte there, which consumed `*Model_Space` as `*M`,
///   `od`, ... and left such a drawing with no model space and no entities.
/// - **CP932 (DOS Shift-JIS) is double-byte**, although
///   `dwg_codepage_isasian` leaves it out, so its kanji are not looked up
///   one byte at a time in a single-byte table. And a GB2312 pair is
///   masked to its 7-bit form before the lookup, because that is how the
///   library's table is indexed while the file holds EUC-CN bytes.
pub fn decode_codepage(bytes: &[u8], codepage: u16) -> (String, usize) {
    debug_assert!(has_codepage_tables(codepage));
    let cp = codepage as libredwg_sys::Dwg_Codepage;
    // SAFETY: pure table predicates on a codepage number within the range
    // has_codepage_tables admits.
    let two_byte_codepage =
        unsafe { libredwg_sys::dwg_codepage_isasian(cp) } || codepage == CP_CP932;
    let mut text = String::with_capacity(bytes.len());
    let mut unmapped = 0;
    let mut i = 0;
    while i < bytes.len() {
        let lead = bytes[i];
        i += 1;
        if lead < 0x80 {
            text.push(char::from(lead));
            continue;
        }
        let code = if two_byte_codepage {
            let mut c = u16::from(lead);
            if opens_pair(codepage, lead) {
                match bytes.get(i) {
                    Some(&trail) => {
                        c = (c << 8) | u16::from(trail);
                        i += 1;
                        if codepage == CP_GB2312 {
                            c &= 0x7F7F;
                        }
                    }
                    None => {
                        // A lead byte at the very end: nothing to pair it with.
                        unmapped += 1;
                        text.push('\u{FFFD}');
                        continue;
                    }
                }
            }
            // SAFETY: a pure table lookup (see above).
            unsafe { libredwg_sys::dwg_codepage_uwc(cp, c) as u32 }
        } else {
            // SAFETY: a pure table lookup (see above).
            unsafe { libredwg_sys::dwg_codepage_uc(cp, lead) as u32 }
        };
        // The tables answer 0 for a code they do not contain.
        match char::from_u32(code).filter(|ch| *ch != '\0') {
            Some(ch) => text.push(ch),
            None => {
                unmapped += 1;
                text.push('\u{FFFD}');
            }
        }
    }
    (text, unmapped)
}

/// How one drawing's strings sit in LibreDWG's memory -- see the module doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Storage {
    /// Read from an R2007+ DWG: the accessors convert from UTF-16 themselves.
    WideDwg,
    /// Read from an R2007+ DXF: the strings the importer set are UTF-16 the
    /// accessors do not convert; those it copied are the file's UTF-8.
    WideDxf,
    /// Before R2007: 8-bit bytes in the drawing's codepage. `utf8_first` for
    /// a DXF, whose importer keeps the file's bytes and assumes UTF-8 --
    /// LibreDWG's own DXF writer emits UTF-8 text whatever `$DWGCODEPAGE` it
    /// declares -- so bytes that are valid UTF-8 are taken as UTF-8 first and
    /// the codepage is tried only for the rest. A DWG never holds UTF-8 in a
    /// codepage string, so there the codepage always applies.
    Narrow { utf8_first: bool },
}

/// The fields LibreDWG's DXF importer copies from the file byte for byte
/// whatever its version, so that in an R2007+ DXF they are 8-bit while every
/// other string is UTF-16: `in_dxf.c` special-cases MTEXT's group 1/3 text
/// chunks (`strdup`, then `realloc` + `memcpy`, no `bit_utf8_to_TU`).
/// Everything else this crate reads goes through the library's field
/// setter, which widens once `header.version` is R2007 or later. (The HEADER
/// variables are the other byte-for-byte case; they are read through
/// [`TextDecoder::header_text`].)
fn stored_8bit_in_dxf(dxfname: &str, field: &str) -> bool {
    matches!((dxfname, field), ("MTEXT", "text"))
}

/// Decodes every string of one drawing and collects what it could not
/// decode. One per `parse()`, created once the drawing is read (its codepage
/// and version are known then) and consumed after the last string.
pub struct TextDecoder {
    codepage: u16,
    storage: Storage,
    warnings: RefCell<Vec<String>>,
}

impl TextDecoder {
    /// # Safety
    /// `dwg` must be a live, successfully read `Dwg_Data`.
    pub unsafe fn new(dwg: *mut libredwg_sys::Dwg_Data) -> Self {
        // SAFETY: the shim reads header fields of a live Dwg_Data (caller
        // contract) and null-checks it itself.
        let (codepage, wide_dwg, from_dxf, version) = unsafe {
            (
                libredwg_sys::uncad_dwg_codepage(dwg),
                libredwg_sys::uncad_dwg_is_wide_string(dwg) != 0,
                libredwg_sys::uncad_dwg_from_dxf(dwg) != 0,
                libredwg_sys::uncad_dwg_version(dwg),
            )
        };
        // The library's own rule for when its field setter widens a string:
        // the drawing's version (`header.version`, which the DXF importer
        // sets from `$ACADVER` once the HEADER section is read), not where
        // the data came from. Cast the constant, not the version: see
        // convert.rs's DWG_OBJECT_TYPE comment for why the enum's width
        // differs between targets.
        #[allow(clippy::unnecessary_cast)]
        let r2007_or_later = version >= libredwg_sys::DWG_VERSION_TYPE_R_2007 as i32;
        let storage = if wide_dwg {
            Storage::WideDwg
        } else if from_dxf && r2007_or_later {
            Storage::WideDxf
        } else {
            Storage::Narrow {
                utf8_first: from_dxf,
            }
        };
        TextDecoder {
            codepage,
            storage,
            warnings: RefCell::new(Vec::new()),
        }
    }

    /// The codepage 8-bit strings are decoded with (`header.codepage`).
    pub fn codepage(&self) -> u16 {
        self.codepage
    }

    /// The warnings, in the order the strings were met, each once -- the
    /// entities of a block are converted twice (once for the entity list,
    /// once for the block record), and a warning is about a string, not
    /// about a visit.
    pub fn into_warnings(self) -> Vec<String> {
        self.warnings.into_inner()
    }

    fn warn(&self, message: String) {
        let mut warnings = self.warnings.borrow_mut();
        if !warnings.contains(&message) {
            warnings.push(message);
        }
    }

    /// The width of a `dxfname.field` string the library hands out
    /// unconverted. For an R2007+ DWG that happens only when its accessor
    /// could not find the object the pointer belongs to (a struct embedded
    /// in another object), and the string is then the stored UTF-16.
    fn stored_width(&self, dxfname: &str, field: &str) -> StoredWidth {
        match self.storage {
            Storage::WideDwg => StoredWidth::Wide,
            Storage::WideDxf if !stored_8bit_in_dxf(dxfname, field) => StoredWidth::Wide,
            Storage::WideDxf | Storage::Narrow { .. } => StoredWidth::Bytes,
        }
    }

    /// A text field (`BITCODE_T`/`TV`/`TU`) of an entity or object, decoded.
    /// `None` only when the field does not exist or is a null string; a
    /// string that is present always comes back, decoded as far as it can be.
    pub fn field(&self, entity: *mut c_void, dxfname: &str, field: &str) -> Option<String> {
        let raw = dynapi::get_text(entity, dxfname, field, self.stored_width(dxfname, field))?;
        Some(self.decode_raw(raw, || {
            // SAFETY: entity is the type-specific struct pointer of a live
            // object (get_text just read a field through it); the accessor
            // answers 0 when it cannot find the owning object.
            let handle = unsafe { libredwg_sys::dwg_obj_generic_handlevalue(entity) };
            format!("{dxfname}.{field} (handle {handle:X})")
        }))
    }

    /// A text field read as a raw pointer rather than through the library's
    /// text accessor -- a field of a struct embedded in another object
    /// (LAYOUT's `plotsettings`), which the accessor cannot place --
    /// decoded by the same rules as [`field`](Self::field). `None` for a null
    /// pointer.
    ///
    /// # Safety
    /// `ptr` must be null or the value of the `dxfname.field` string field
    /// of an object of the drawing this decoder was made for, still live.
    #[allow(
        dead_code,
        reason = "the reader for embedded structs (LAYOUT's plot settings) is ported separately"
    )]
    pub unsafe fn stored_field(
        &self,
        ptr: *const std::os::raw::c_char,
        dxfname: &str,
        field: &str,
    ) -> Option<String> {
        if ptr.is_null() {
            return None;
        }
        // Nothing has converted this one: an R2007+ DWG's string is its
        // stored UTF-16 here, which stored_width already answers.
        let width = self.stored_width(dxfname, field);
        // SAFETY: a live string field of this drawing (caller contract),
        // read in the width this drawing stores it in.
        let raw = unsafe { dynapi::read_stored(ptr, width) };
        Some(self.decode_raw(raw, || format!("{dxfname}.{field}")))
    }

    /// The name of whatever a handle reference points at, decoded -- see
    /// `dynapi::handle_name`. Such a name is a table record's or an
    /// object's, which the importer always sets through the field setter.
    pub fn handle_name(
        &self,
        dwg: *mut libredwg_sys::Dwg_Data,
        handle: *mut libredwg_sys::Dwg_Object_Ref,
    ) -> Option<String> {
        let raw = dynapi::handle_name(dwg, handle, self.stored_width("", "name"))?;
        Some(self.decode_raw(raw, || {
            // SAFETY: handle is non-null (handle_name returned) and a
            // Dwg_Object_Ref the live Dwg_Data owns.
            let value = unsafe { (*handle).absolute_ref };
            format!("name of handle {value:X}")
        }))
    }

    /// The name of the table entry a reference points at, decoded -- see
    /// `dynapi::table_entry_name_bytes`. The library converts the name itself
    /// for every R2007+ drawing (for a DXF through the vendored `dwg.c`
    /// patch), so only a pre-R2007 one is left as codepage bytes.
    pub fn table_entry_name(
        &self,
        dwg: *mut libredwg_sys::Dwg_Data,
        handle: *mut libredwg_sys::Dwg_Object_Ref,
        table: &CStr,
    ) -> Option<String> {
        let bytes = dynapi::table_entry_name_bytes(dwg, handle, table)?;
        let raw = match self.storage {
            Storage::WideDwg | Storage::WideDxf => RawText::Converted(bytes),
            Storage::Narrow { .. } => RawText::Bytes(bytes),
        };
        Some(self.decode_raw(raw, || {
            let value = unsafe { (*handle).absolute_ref };
            format!(
                "{} entry of handle {value:X}",
                table.to_str().unwrap_or("table")
            )
        }))
    }

    /// A text header variable (`DIMPOST`, ...), decoded. The DXF importer
    /// stores these 8-bit whatever the version: it reads the HEADER section
    /// before `$ACADVER` has told it to widen anything.
    pub fn header_text(&self, dwg: *const libredwg_sys::Dwg_Data, name: &str) -> Option<String> {
        let raw = dynapi::get_header_text(dwg, name, StoredWidth::Bytes)?;
        Some(self.decode_raw(raw, || format!("header variable ${name}")))
    }

    /// Turns what the library handed out into a `String` by this drawing's
    /// rule; `what` names the string for a diagnostic.
    fn decode_raw(&self, raw: RawText, what: impl FnOnce() -> String) -> String {
        match raw {
            RawText::Converted(bytes) => self.decode_utf8(&bytes, what),
            RawText::Wide(units) => match String::from_utf16(&units) {
                Ok(text) => text,
                Err(_) => {
                    self.warn(format!(
                        "TEXT_ENCODING: {}: not valid UTF-16; unpaired surrogates replaced \
                         with U+FFFD",
                        what()
                    ));
                    String::from_utf16_lossy(&units)
                }
            },
            RawText::Bytes(bytes) => self.decode(&bytes, what),
        }
    }

    /// Bytes that are UTF-8 by the drawing's own account, checked.
    fn decode_utf8(&self, bytes: &[u8], what: impl FnOnce() -> String) -> String {
        match std::str::from_utf8(bytes) {
            Ok(text) => text.to_string(),
            Err(_) => {
                self.warn(format!(
                    "TEXT_ENCODING: {}: not valid UTF-8 although the drawing's strings are \
                     UTF-8; invalid sequences replaced with U+FFFD",
                    what()
                ));
                String::from_utf8_lossy(bytes).into_owned()
            }
        }
    }

    /// The decoding rule for an 8-bit string. `what` names the string for a
    /// diagnostic and is only evaluated when one is written.
    pub fn decode(&self, bytes: &[u8], what: impl FnOnce() -> String) -> String {
        if bytes.is_ascii() {
            // ASCII is the same in every codepage this crate meets.
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let utf8_first = match self.storage {
            // An R2007+ drawing's 8-bit strings are UTF-8: the converted
            // ones by construction, the ones an R2007+ DXF keeps as they
            // are because such a file is UTF-8.
            Storage::WideDwg | Storage::WideDxf => return self.decode_utf8(bytes, what),
            Storage::Narrow { .. } if self.codepage == CP_UTF8 => {
                return self.decode_utf8(bytes, what)
            }
            Storage::Narrow { utf8_first } => utf8_first,
        };
        if utf8_first {
            if let Ok(text) = std::str::from_utf8(bytes) {
                return text.to_string();
            }
        }
        if !has_codepage_tables(self.codepage) {
            return match std::str::from_utf8(bytes) {
                Ok(text) => text.to_string(),
                Err(_) => {
                    self.warn(format!(
                        "TEXT_ENCODING: {}: codepage {} cannot be decoded and the bytes are \
                         not UTF-8; invalid sequences replaced with U+FFFD",
                        what(),
                        codepage_label(self.codepage)
                    ));
                    String::from_utf8_lossy(bytes).into_owned()
                }
            };
        }
        let (text, unmapped) = decode_codepage(bytes, self.codepage);
        if unmapped > 0 {
            self.warn(format!(
                "TEXT_ENCODING: {}: {unmapped} byte(s) have no character in codepage {}; \
                 replaced with U+FFFD",
                what(),
                codepage_label(self.codepage)
            ));
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSI_949: u16 = 40;
    const ANSI_1252: u16 = 30;
    const ANSI_936: u16 = 39;
    const ANSI_950: u16 = 41;
    const ANSI_932: u16 = 38;

    fn decoder(codepage: u16, storage: Storage) -> TextDecoder {
        TextDecoder {
            codepage,
            storage,
            warnings: RefCell::new(Vec::new()),
        }
    }

    fn narrow(codepage: u16, utf8_first: bool) -> TextDecoder {
        decoder(codepage, Storage::Narrow { utf8_first })
    }

    #[test]
    fn cp949_hangul_decodes_to_the_unicode_syllables() {
        // "drawing number" in Korean: four syllables, two bytes each.
        let bytes = b"\xB5\xB5\xB8\xE9\xB9\xF8\xC8\xA3 BP-1042";
        let (text, unmapped) = decode_codepage(bytes, ANSI_949);
        assert_eq!(text, "\u{B3C4}\u{BA74}\u{BC88}\u{D638} BP-1042");
        assert_eq!(unmapped, 0);
    }

    #[test]
    fn cp1252_accented_letters_decode_one_byte_each() {
        let (text, unmapped) = decode_codepage(b"caf\xE9 \xFCber", ANSI_1252);
        assert_eq!(text, "caf\u{E9} \u{FC}ber");
        assert_eq!(unmapped, 0);
    }

    #[test]
    fn single_byte_codepages_keep_every_character_of_a_non_ascii_string() {
        // CP1251 "Стена" (all five bytes non-ASCII) and CP1252 "€€€" (three
        // bytes of UTF-8 each): the library's own converter sizes its output
        // at 1.5x the input and stops reading when it is full.
        assert_eq!(
            decode_codepage(b"\xD1\xF2\xE5\xED\xE0", 29),
            ("Стена".to_string(), 0)
        );
        assert_eq!(
            decode_codepage(b"\x80\x80\x80", ANSI_1252),
            ("€€€".to_string(), 0)
        );
    }

    #[test]
    fn an_unmapped_byte_is_replaced_and_counted() {
        // 0x81 is unassigned in Windows-1252.
        let (text, unmapped) = decode_codepage(b"a\x81b", ANSI_1252);
        assert_eq!(text, "a\u{FFFD}b");
        assert_eq!(unmapped, 1);
        // A CP949 lead byte with nothing after it.
        let (text, unmapped) = decode_codepage(b"ok\xB5", ANSI_949);
        assert_eq!(text, "ok\u{FFFD}");
        assert_eq!(unmapped, 1);
    }

    #[test]
    fn the_dos_era_double_byte_codepages_pair_only_high_bytes() {
        // Byte sequences from Python: '中国 AB'.encode('gb2312') (EUC-CN),
        // '中文 AB'.encode('big5'), '日本 AB'.encode('shift_jis'). The
        // Windows twins 936/950/932 read the same bytes and are the control.
        // Before these rules GB2312 gave four U+FFFD (the ASCII paired up and
        // lost), Big5 '中文' plus two U+FFFD, CP932 U+FFFD for each kanji.
        let cases: [(u16, &[u8], &str); 6] = [
            (CP_GB2312, b"\xD6\xD0\xB9\xFA AB", "中国 AB"),
            (ANSI_936, b"\xD6\xD0\xB9\xFA AB", "中国 AB"),
            (CP_BIG5, b"\xA4\xA4\xA4\xE5 AB", "中文 AB"),
            (ANSI_950, b"\xA4\xA4\xA4\xE5 AB", "中文 AB"),
            (CP_CP932, b"\x93\xFA\x96\x7B AB", "日本 AB"),
            (ANSI_932, b"\x93\xFA\x96\x7B AB", "日本 AB"),
        ];
        for (codepage, bytes, expected) in cases {
            assert_eq!(
                decode_codepage(bytes, codepage),
                (expected.to_string(), 0),
                "codepage {codepage}"
            );
        }
        // ASCII is never paired: a block name survives whole.
        for codepage in [CP_GB2312, CP_BIG5, CP_CP932] {
            assert_eq!(
                decode_codepage(b"*Model_Space", codepage),
                ("*Model_Space".to_string(), 0)
            );
        }
    }

    #[test]
    fn ascii_is_never_looked_up_so_a_backslash_stays_a_backslash() {
        // The library's CP932 and JOHAB tables map 0x5C to a currency sign;
        // in a drawing it is the backslash of \P and \U+XXXX.
        for codepage in [CP_CP932, ANSI_932] {
            assert_eq!(
                decode_codepage(b"\x93\xFA\\P\\U+00B1", codepage),
                ("日\\P\\U+00B1".to_string(), 0)
            );
        }
        const JOHAB: u16 = 26;
        let (text, _) = decode_codepage(b"\xC8\\P", JOHAB);
        assert!(text.ends_with("\\P"), "{text}");
    }

    #[test]
    fn the_decoder_reports_what_it_could_not_decode_once() {
        let d = narrow(ANSI_1252, false);
        assert_eq!(d.decode(b"plain", || unreachable!()), "plain");
        assert_eq!(d.decode(b"caf\xE9", || unreachable!()), "caf\u{E9}");
        assert_eq!(
            d.decode(b"\x81", || "TEXT.text_value (handle 1A)".into()),
            "\u{FFFD}"
        );
        assert_eq!(
            d.decode(b"\x81", || "TEXT.text_value (handle 1A)".into()),
            "\u{FFFD}"
        );
        let warnings = d.into_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].starts_with("TEXT_ENCODING: TEXT.text_value (handle 1A): 1 byte(s)"),
            "{}",
            warnings[0]
        );
        assert!(warnings[0].contains("ANSI_1252 (30)"), "{}", warnings[0]);
    }

    #[test]
    fn a_dxf_string_that_is_valid_utf8_is_read_as_utf8_first() {
        // The same bytes are CP949 in a DWG and UTF-8 in a DXF import.
        let utf8 = "\u{B3C4}\u{BA74}".as_bytes();
        assert_eq!(
            narrow(ANSI_949, true).decode(utf8, || unreachable!()),
            "\u{B3C4}\u{BA74}"
        );
        let (as_cp949, _) = decode_codepage(utf8, ANSI_949);
        assert_eq!(
            narrow(ANSI_949, false).decode(utf8, || "x".into()),
            as_cp949
        );
        assert_ne!(as_cp949, "\u{B3C4}\u{BA74}");
    }

    #[test]
    fn an_undefined_or_corrupt_codepage_never_reaches_the_tables() {
        for (codepage, label) in [(0xFF, "undefined (255)"), (100, "codepage 100 ")] {
            let d = narrow(codepage, false);
            assert_eq!(
                d.decode("caf\u{E9}".as_bytes(), || unreachable!()),
                "caf\u{E9}"
            );
            assert_eq!(
                d.decode(b"caf\xE9", || "LAYER.name (handle 2)".into()),
                "caf\u{FFFD}"
            );
            let warnings = d.into_warnings();
            assert_eq!(warnings.len(), 1);
            assert!(warnings[0].contains(label), "{}", warnings[0]);
        }
    }

    #[test]
    fn a_wide_string_drawing_is_utf8_whatever_its_codepage_says() {
        for storage in [Storage::WideDwg, Storage::WideDxf] {
            let d = decoder(ANSI_949, storage);
            assert_eq!(
                d.decode("\u{B3C4}".as_bytes(), || unreachable!()),
                "\u{B3C4}"
            );
            assert!(d.into_warnings().is_empty());
            // Bytes that are not UTF-8 are reported, never guessed at.
            let d = decoder(ANSI_949, storage);
            assert_eq!(
                d.decode(b"\xB5\xB5", || "MTEXT.text".into()),
                "\u{FFFD}\u{FFFD}"
            );
            assert_eq!(d.into_warnings().len(), 1);
        }
    }

    #[test]
    fn an_r2007_dxf_reads_its_stored_strings_in_the_width_the_importer_wrote() {
        let d = decoder(CP_UTF16, Storage::WideDxf);
        assert_eq!(d.stored_width("LAYER", "name"), StoredWidth::Wide);
        assert_eq!(d.stored_width("TEXT", "text_value"), StoredWidth::Wide);
        assert_eq!(d.stored_width("MTEXT", "text"), StoredWidth::Bytes);
        let d = narrow(ANSI_1252, true);
        assert_eq!(d.stored_width("LAYER", "name"), StoredWidth::Bytes);
    }

    #[test]
    fn a_stored_utf16_string_is_read_to_its_16_bit_nul() {
        // "가A" then a 16-bit NUL: the first byte of U+AC00 in memory is 0x00,
        // where an 8-bit read would have stopped with nothing.
        let units: [u16; 3] = [0xAC00, 0x0041, 0];
        let d = decoder(CP_UTF16, Storage::WideDxf);
        let got = unsafe { d.stored_field(units.as_ptr().cast(), "LAYOUT", "layout_name") };
        assert_eq!(got.as_deref(), Some("\u{AC00}A"));
        // An unpaired surrogate is replaced and reported.
        let units: [u16; 2] = [0xD800, 0];
        let got = unsafe { d.stored_field(units.as_ptr().cast(), "LAYOUT", "layout_name") };
        assert_eq!(got.as_deref(), Some("\u{FFFD}"));
        assert_eq!(d.into_warnings().len(), 1);
    }

    #[test]
    fn codepage_names_are_the_librarys_and_unknown_ones_have_none() {
        assert_eq!(codepage_name(ANSI_1252).as_deref(), Some("ANSI_1252"));
        assert_eq!(codepage_name(CP_GB2312).as_deref(), Some("GB2312"));
        assert_eq!(codepage_name(100), None);
        assert_eq!(codepage_name(0xFF), None);
    }
}
