//! The common properties an entity can state instead of taking them from
//! its layer: linetype (6), linetype scale (48), lineweight (370) and
//! transparency (440) -- and what an entity that states none of them has,
//! which depends on whether the drawing's version has the property at all.

use uncad::model::{EntityCommon, EntityLinetype, Ref, Transparency};
use uncad::Format;

/// A DXF stamped `version` holding one LINE whose
/// `AcDbEntity` part carries `groups` after its layer, and an LTYPE table
/// declaring `HIDDEN`.
fn line(version: Option<&str>, groups: &[(i32, &str)]) -> EntityCommon {
    let mut text = String::new();
    if let Some(v) = version {
        text.push_str(&format!(
            "  0\nSECTION\n  2\nHEADER\n  9\n$ACADVER\n  1\n{v}\n  0\nENDSEC\n"
        ));
    }
    text.push_str(
        "  0\nSECTION\n  2\nTABLES\n  0\nTABLE\n  2\nLTYPE\n 70\n1\n  0\nLTYPE\n  2\nHIDDEN\n 70\n0\n  0\nENDTAB\n  0\nENDSEC\n",
    );
    text.push_str("  0\nSECTION\n  2\nENTITIES\n  0\nLINE\n  5\n2A\n100\nAcDbEntity\n  8\n0\n");
    for (code, value) in groups {
        text.push_str(&format!("{code:>3}\n{value}\n"));
    }
    text.push_str(
        "100\nAcDbLine\n 10\n0\n 20\n0\n 30\n0\n 11\n1\n 21\n0\n 31\n0\n  0\nENDSEC\n  0\nEOF\n",
    );
    let db = uncad::parse_bytes(text.as_bytes(), Format::Dxf)
        .unwrap_or_else(|e| panic!("{version:?}: {e:?}"));
    db.entities[0].common().clone()
}

#[test]
fn stated_properties_are_carried_as_the_file_states_them() {
    let c = line(
        Some("AC1018"),
        &[(6, "HIDDEN"), (370, "35"), (48, "2.5"), (440, "33554636")],
    );
    assert_eq!(
        c.linetype,
        EntityLinetype::Named(Ref::Resolved("HIDDEN".into()))
    );
    assert_eq!(c.linetype_scale, 2.5);
    assert_eq!(c.lineweight, Some(35));
    assert_eq!(c.transparency, Some(0x0200_00CC));
    assert_eq!(
        c.transparency.and_then(Transparency::from_code),
        Some(Transparency::Alpha(0xCC))
    );
}

#[test]
fn bylayer_and_byblock_are_not_table_entries() {
    assert_eq!(
        line(Some("AC1018"), &[(6, "ByBlock")]).linetype,
        EntityLinetype::ByBlock
    );
    assert_eq!(
        line(Some("AC1018"), &[(6, "BYLAYER")]).linetype,
        EntityLinetype::ByLayer
    );
    // A name the file never declares: the DXF importer drops the name it
    // could not look up, so the reference arrives absent rather than
    // unresolved (docs/CAVEATS.md -- the same as for blocks and styles).
    assert_eq!(
        line(Some("AC1018"), &[(6, "DASHED")]).linetype,
        EntityLinetype::Named(Ref::Absent)
    );
    assert_eq!(
        line(Some("AC1018"), &[(440, "16777216")])
            .transparency
            .and_then(Transparency::from_code),
        Some(Transparency::ByBlock)
    );
}

/// An unstated property is BYLAYER where the version has it: lineweight from
/// R2000, transparency from R2004. Before that the file cannot state it,
/// and the model says so. (A DXF with no header is refused by the importer,
/// so that case has nothing to read here.)
#[test]
fn an_unstated_property_is_bylayer_only_where_the_version_has_it() {
    let r2004 = line(Some("AC1018"), &[]);
    assert_eq!(r2004.linetype, EntityLinetype::ByLayer);
    assert_eq!(r2004.linetype_scale, 1.0);
    assert_eq!((r2004.lineweight, r2004.transparency), (Some(-1), Some(0)));

    let r2000 = line(Some("AC1015"), &[]);
    assert_eq!((r2000.lineweight, r2000.transparency), (Some(-1), None));

    let r14 = line(Some("AC1014"), &[]);
    assert_eq!((r14.lineweight, r14.transparency), (None, None));
}
