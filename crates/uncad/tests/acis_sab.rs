//! Regression guard for a parse-time mutation that corrupted every later
//! write of a drawing containing SAB-encoded (ACIS BinaryFile, `version == 2`)
//! 3DSOLID/REGION entities.
//!
//! `acis.rs` used to call LibreDWG's `dwg_convert_SAB_to_SAT1` on the *live*
//! entity to get SAT text for the wireframe. That function converts in place
//! (version 2 -> 1, unencrypted SAT into `encr_sat_data`), and because
//! `CadDatabase` keeps the same `Dwg_Data` alive for `write_dxf`/`write_dwg`,
//! both encoders then treated the plaintext as already-obfuscated SAT1 and
//! wrote it out as-is -- which readers "decrypt" into garbage. Fixed by doing
//! the conversion on a copy (`uncad_3dsolid_sab_to_sat_text` in
//! `libredwg-sys`'s shim); see `docs/CAVEATS.md`, "DWG/DXF 쓰기 지원".
//!
//! Fixture: `lib/libredwg/test/test-data/2007/ATMOS-DC22S.dwg`, the one file
//! in the bundled corpus whose solids are stored as SAB (58 of them).
//!
//! Both assertions are reference-free. A parsed database must write out
//! byte-for-byte what the parse-free file-to-file converter writes (the two
//! paths share the encoder; only a mutated `Dwg_Data` can make them differ),
//! and the solids that had a wireframe before a write/reparse round trip must
//! still have one after.

use std::fs;
use std::path::{Path, PathBuf};

const SAB_DWG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/2007/ATMOS-DC22S.dwg"
);

/// Removes its file on drop, so a failing assertion leaves nothing behind.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl TempFile {
    fn new(name: &str) -> Self {
        TempFile(std::env::temp_dir().join(format!("uncad-{}-{}", std::process::id(), name)))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

/// How many 3DSOLID/REGION entities the wireframe extractor got edges out of.
fn solids_with_wireframes(db: &uncad::CadDatabase) -> usize {
    db.entities
        .iter()
        .filter(|e| match e {
            uncad::RenderEntity::Solid3D(s) | uncad::RenderEntity::Region(s) => {
                !s.wireframe_edges.is_empty()
            }
            _ => false,
        })
        .count()
}

#[test]
fn parsing_does_not_change_what_write_dxf_emits_for_sab_solids() {
    let via_db = TempFile::new("sab-via-db.dxf");
    let via_file = TempFile::new("sab-via-file.dxf");

    let mut db = uncad::parse(SAB_DWG).expect("the SAB fixture should parse");
    assert!(
        solids_with_wireframes(&db) > 0,
        "the fixture should yield at least one SAB solid wireframe, or this test guards nothing"
    );
    db.write_dxf(via_db.path())
        .expect("write_dxf after parse should succeed");
    uncad::dwg_to_dxf(SAB_DWG, via_file.path())
        .expect("the parse-free file-to-file conversion should succeed");

    let from_db = fs::read(via_db.path()).expect("write_dxf should have produced a file");
    let from_file = fs::read(via_file.path()).expect("dwg_to_dxf should have produced a file");
    assert!(
        from_db == from_file,
        "write_dxf after parse() must match the parse-free conversion byte for byte \
         ({} vs {} bytes); a difference means parse() mutated the Dwg_Data it hands \
         to the encoder",
        from_db.len(),
        from_file.len()
    );
}

/// How many 3DSOLID/REGION entities there are at all, wireframe or not.
fn solids(db: &uncad::CadDatabase) -> usize {
    db.entities
        .iter()
        .filter(|e| {
            matches!(
                e,
                uncad::RenderEntity::Solid3D(_) | uncad::RenderEntity::Region(_)
            )
        })
        .count()
}

/// The round trip goes through `write_dwg`, not `write_dxf`, on purpose:
/// LibreDWG's DXF *reader* brings back 1 of this R2007 drawing's 60
/// entities (and no usable solid) even from the byte-identical `dwg_to_dxf`
/// output -- the test above proves the two are the same bytes -- so a DXF
/// round trip could not tell a fixed writer from a broken one; that is an
/// upstream DXF-import limit (`docs/CAVEATS.md`, "DXF 읽기"), not this
/// crate's writer. The DWG encoder does re-read its own output here.
///
/// "No loss", not "same count": the encoder writes this AC1021 source back
/// as AC1024, and LibreDWG's SAB-to-SAT conversion picks its SAT dialect
/// from the header version, so the minimal SAT reader in `acis.rs` gets
/// edges out of *more* solids after the trip (57) than before (26). The bug
/// made it 0 -- every solid's SAT data came back as garbage -- which is what
/// the lower bound catches without pinning a number that is really
/// LibreDWG's to choose.
#[test]
fn sab_solid_wireframes_survive_a_dwg_round_trip() {
    let mut db = uncad::parse(SAB_DWG).expect("the SAB fixture should parse");
    let solids_before = solids(&db);
    let wireframes_before = solids_with_wireframes(&db);
    assert!(
        wireframes_before > 0,
        "the fixture should yield at least one SAB solid wireframe"
    );

    let out = TempFile::new("sab-roundtrip.dwg");
    db.write_dwg(out.path())
        .expect("write_dwg after parse should succeed");
    let reparsed = uncad::parse(out.path()).expect("what this crate wrote, it should read back");

    assert_eq!(
        solids(&reparsed),
        solids_before,
        "the number of solids changed across a DWG write/reparse round trip"
    );
    let wireframes_after = solids_with_wireframes(&reparsed);
    assert!(
        wireframes_after >= wireframes_before,
        "solids lost their wireframes across a write/reparse round trip ({wireframes_before} \
         -> {wireframes_after}) -- the SAT data was written in a form the reader could not \
         decode"
    );
}
