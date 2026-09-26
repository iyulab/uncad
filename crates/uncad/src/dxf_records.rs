//! What an ASCII DXF states about its own entity records, read from the bytes
//! without LibreDWG -- a count the importer's result can be checked against.

/// Records that belong to the entity before them rather than standing on
/// their own: a polyline's vertices, an insert's attributes, and the
/// `SEQEND` that closes either run.
const OWNED_RECORDS: [&[u8]; 3] = [b"VERTEX", b"ATTRIB", b"SEQEND"];

/// How many entity records an ASCII DXF's ENTITIES section holds, not
/// counting the records owned by the entity before them (`VERTEX`, `ATTRIB`,
/// `SEQEND`). Read as (group code, value) line pairs; `Some(0)` for a file
/// with no ENTITIES section, `None` for a binary DXF, which is not scanned.
///
/// Each counted record is one top-level entity the file states, so a read
/// whose model holds fewer top-level entities than this lost some on the way.
/// The converse does not hold: a model may hold more (paper-space content
/// that later versions keep in the BLOCKS section), so only "fewer" means
/// anything.
pub(crate) fn entities_section_records(bytes: &[u8]) -> Option<usize> {
    if bytes.starts_with(b"AutoCAD Binary DXF") {
        return None;
    }
    let mut lines = bytes.split(|&b| b == b'\n').map(<[u8]>::trim_ascii);
    let mut count = 0;
    let mut section_opened = false;
    let mut in_entities = false;
    while let (Some(code), Some(value)) = (lines.next(), lines.next()) {
        match (code, value) {
            (b"0", b"SECTION") => section_opened = true,
            (b"2", name) if section_opened => {
                section_opened = false;
                in_entities = name == b"ENTITIES";
            }
            (b"0", b"ENDSEC") => in_entities = false,
            (b"0", b"EOF") => break,
            (b"0", kind) if in_entities && !OWNED_RECORDS.contains(&kind) => count += 1,
            _ => section_opened = false,
        }
    }
    Some(count)
}

#[cfg(test)]
mod tests {
    use super::entities_section_records;

    fn dxf(body: &str) -> Vec<u8> {
        format!(
            "  0\nSECTION\n  2\nHEADER\n  9\n$ACADVER\n  1\nAC1014\n  0\nENDSEC\n{body}  0\nEOF\n"
        )
        .into_bytes()
    }

    #[test]
    fn counts_top_level_entities_and_skips_the_records_they_own() {
        let bytes = dxf(concat!(
            "  0\nSECTION\n  2\nENTITIES\n",
            "  0\nLINE\n  8\n0\n",
            "  0\nPOLYLINE\n  8\n0\n  0\nVERTEX\n  8\n0\n  0\nVERTEX\n  8\n0\n  0\nSEQEND\n",
            "  0\nINSERT\n  2\nB\n  0\nATTRIB\n  2\nTAG\n  0\nSEQEND\n",
            "  0\nENDSEC\n",
        ));
        assert_eq!(entities_section_records(&bytes), Some(3));
    }

    #[test]
    fn entities_in_blocks_are_not_counted() {
        let bytes = dxf(concat!(
            "  0\nSECTION\n  2\nBLOCKS\n",
            "  0\nBLOCK\n  2\nB\n  0\nCIRCLE\n  8\n0\n  0\nENDBLK\n",
            "  0\nENDSEC\n",
            "  0\nSECTION\n  2\nENTITIES\n  0\nCIRCLE\n  8\n0\n  0\nENDSEC\n",
        ));
        assert_eq!(entities_section_records(&bytes), Some(1));
    }

    #[test]
    fn a_group_2_value_named_entities_outside_a_section_header_is_not_a_section() {
        let bytes =
            dxf("  0\nSECTION\n  2\nTABLES\n  0\nTABLE\n  2\nENTITIES\n  0\nENDTAB\n  0\nENDSEC\n");
        assert_eq!(entities_section_records(&bytes), Some(0));
    }

    #[test]
    fn crlf_line_ends_read_the_same() {
        let bytes = dxf("  0\nSECTION\n  2\nENTITIES\n  0\nLINE\n  8\n0\n  0\nENDSEC\n");
        let crlf = String::from_utf8(bytes)
            .unwrap()
            .replace('\n', "\r\n")
            .into_bytes();
        assert_eq!(entities_section_records(&crlf), Some(1));
    }

    #[test]
    fn a_binary_dxf_is_not_scanned() {
        assert_eq!(
            entities_section_records(b"AutoCAD Binary DXF\r\n\x1a\0"),
            None
        );
    }
}
