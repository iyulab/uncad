//! One walk over the whole LibreDWG corpus, tallying every signal this crate
//! emits where a read could otherwise have come back empty without saying so:
//! how each file parsed, LibreDWG's non-fatal error bits, the state of every
//! reference, the ACIS edges that could not be read, and the block references
//! that drew nothing. The counts are printed so a change shows up as a number,
//! and pinned so that a change fails the build rather than passing in silence.
//!
//! The pinned values are measurements, not requirements: when the corpus or
//! the reader legitimately changes, re-measure and update them -- but say why.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use iron_render_cad::ToSvgOptions;
use uncad::model::Ref;
use uncad::{Entity, ParseError};

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data"
);

fn drawings_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("corpus directory should be readable") {
        let path = entry.expect("corpus entry should be readable").path();
        if path.is_dir() {
            drawings_under(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("dwg") || e.eq_ignore_ascii_case("dxf"))
        {
            out.push(path);
        }
    }
}

fn dir_name(path: &Path) -> String {
    path.parent()
        .and_then(Path::file_name)
        .and_then(|d| d.to_str())
        .unwrap_or("")
        .to_string()
}

#[derive(Default)]
struct Sweep {
    files: usize,
    dwg_parsed: usize,
    dxf_parsed: usize,
    dxf_refused: usize,
    critical: usize,
    /// LibreDWG error-bit name -> number of DWG files carrying it.
    diagnostics: BTreeMap<String, usize>,
    clean_dwg: usize,
    /// (directory, field, state) -> count, over top-level and block entities.
    refs: BTreeMap<(String, &'static str, &'static str), usize>,
    /// (file, payload, size of that file's layer table) per unresolved reference.
    unresolved: Vec<(String, String, usize)>,
    skipped_edges: usize,
    solids: usize,
    files_with_empty_blocks: usize,
    /// (file, blocks that drew nothing), one row per file that had any.
    empty_blocks: Vec<(String, Vec<String>)>,
    /// (file, reference ID) for every ID two different entities of one
    /// file share -- the model requires none.
    duplicate_ids: Vec<(String, u64)>,
    /// Entities whose ID had to be minted from the object index because the
    /// file gave them no handle.
    handleless: usize,
}

fn sweep() -> Sweep {
    let mut files = Vec::new();
    drawings_under(Path::new(CORPUS), &mut files);
    files.sort();
    let mut s = Sweep {
        files: files.len(),
        ..Sweep::default()
    };
    for path in &files {
        let is_dxf = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dxf"));
        let db = match uncad::parse(path) {
            Ok(db) => db,
            Err(ParseError::UnsupportedDxfVersion(_)) => {
                s.dxf_refused += 1;
                continue;
            }
            Err(_) => {
                s.critical += 1;
                continue;
            }
        };
        if is_dxf {
            s.dxf_parsed += 1;
        } else {
            s.dwg_parsed += 1;
            if db.read_diagnostics.is_clean() {
                s.clean_dwg += 1;
            }
            for name in &db.read_diagnostics.warnings {
                *s.diagnostics.entry(name.clone()).or_default() += 1;
            }
        }

        let dir = dir_name(path);
        let layer_table = db.tables.layers.len();

        // Reference IDs: unique within the file, across the top level and
        // every block (the same entity may appear in both -- that is not a
        // duplicate, so IDs are compared by entity value).
        let mut by_id: BTreeMap<u64, &Entity> = BTreeMap::new();
        let all = db.entities.iter().chain(
            db.tables
                .block_records
                .values()
                .flat_map(|b| b.entities.iter()),
        );
        for e in all {
            let id = e.common().id.value();
            if e.common().source_handle == Ref::Absent {
                s.handleless += 1;
            }
            match by_id.get(&id) {
                Some(other) if *other != e => {
                    s.duplicate_ids.push((path.display().to_string(), id))
                }
                _ => {
                    by_id.insert(id, e);
                }
            }
        }
        let in_blocks = db
            .tables
            .block_records
            .values()
            .flat_map(|b| b.entities.iter());
        for e in db.entities.iter().chain(in_blocks) {
            let mut note = |field: &'static str, r: &Ref<String>| {
                let state = match r {
                    Ref::Resolved(_) => "resolved",
                    Ref::Absent => "absent",
                    Ref::Unresolved(h) => {
                        s.unresolved
                            .push((path.display().to_string(), h.clone(), layer_table));
                        "unresolved"
                    }
                };
                *s.refs.entry((dir.clone(), field, state)).or_default() += 1;
            };
            note("layer", &e.common().layer);
            match e {
                Entity::Insert(i) => note("block", &i.block_name),
                Entity::Dimension(d) => note("block", &d.block_name),
                Entity::AcadTable(t) => note("block", &t.block_name),
                Entity::MLine(m) => note("mlinestyle", &m.mlinestyle_name),
                Entity::Solid3D(solid) | Entity::Region(solid) => {
                    s.solids += 1;
                    s.skipped_edges += solid.skipped_edges;
                }
                _ => {}
            }
        }

        let svg = iron_render_cad::to_svg(&db, ToSvgOptions::default());
        if !svg.empty_blocks.is_empty() {
            s.files_with_empty_blocks += 1;
            s.empty_blocks
                .push((path.display().to_string(), svg.empty_blocks.clone()));
        }
    }
    s
}

#[test]
fn the_corpus_distribution_is_what_it_was_when_last_measured() {
    let started = std::time::Instant::now();
    let s = sweep();
    println!(
        "files {} · dwg parsed {} · dxf parsed {} · dxf refused {} · critical {} · {:.1?}",
        s.files,
        s.dwg_parsed,
        s.dxf_parsed,
        s.dxf_refused,
        s.critical,
        started.elapsed()
    );
    println!(
        "clean dwg {} · diagnostics {:?}",
        s.clean_dwg, s.diagnostics
    );
    for ((dir, field, state), n) in &s.refs {
        println!("{dir:>14} {field:<10} {state:<10} {n}");
    }
    println!(
        "solids {} · skipped edges {} · files with empty blocks {}",
        s.solids, s.skipped_edges, s.files_with_empty_blocks
    );
    for (file, blocks) in &s.empty_blocks {
        println!("empty blocks in {file}: {blocks:?}");
    }

    // --- how the files read ---
    // (31, 32, 4) while every R2007+ DXF was refused. Read now: 27 of the 32
    // parse, and the other 5 (2013/gh109_1, 2018/Constraints, Dynblocks,
    // LiveSection1, TS1) fail inside LibreDWG's importer with critical
    // error 2048. None trips the guard against an R2007+ DXF whose entities
    // all go missing (`dxf_refused`).
    assert_eq!((s.files, s.dwg_parsed), (208, 141));
    assert_eq!((s.dxf_parsed, s.dxf_refused, s.critical), (58, 0, 9));

    // --- LibreDWG's non-fatal error bits, which used to be discarded ---
    assert_eq!(s.clean_dwg, 100);
    assert_eq!(s.diagnostics.get("UNHANDLEDCLASS"), Some(&17));
    assert_eq!(s.diagnostics.get("VALUEOUTOFBOUNDS"), Some(&31));

    // --- references: no bare zero handle; the only unresolved ones are the
    // 22 layers of the R1.4 drawing, whose LAYER table LibreDWG does not read;
    // a null block handle from R13 on is absent, not unresolved ---
    assert_eq!(s.unresolved.len(), 22, "{:?}", s.unresolved);
    for (file, payload, layer_table) in &s.unresolved {
        assert!(file.contains("r1.4"), "{file}");
        assert_eq!(payload, "idx:1", "{file}");
        assert_eq!(
            *layer_table, 0,
            "{file}: an index only goes unresolved when the table is missing"
        );
    }
    assert_eq!(s.refs[&("2018".to_string(), "block", "absent")], 18);
    let layers: usize = s
        .refs
        .iter()
        .filter(|((_, field, _), _)| *field == "layer")
        .map(|(_, n)| n)
        .sum();
    // 64,697 before the attribute-chain fix: 36 more entities (all ATTDEFs in
    // blocks with several of them, in the R2000 and R13/R14 files) are read
    // now that this crate walks the R13..R2000 block chain itself instead of
    // through the library's walker, which skipped them. 64,733 then; 1,331
    // more since the 27 readable R2007+ DXFs are read instead of refused --
    // all resolved but the 21 of 2010/gh209_1.dxf, whose entities LibreDWG's
    // importer leaves without a layer handle (absent, as the model says).
    assert_eq!(layers, 66_064);

    // --- reference IDs: the handle-derived scheme yields no duplicate in any
    // file, and the index fallback is measured, not assumed ---
    assert!(s.duplicate_ids.is_empty(), "{:?}", s.duplicate_ids);
    println!("handle-less entities (index-derived IDs): {}", s.handleless);

    // --- ACIS edges that could not be read (the SAB-to-SAT texts whose
    // pointers run past their records) and block references that drew nothing.
    // The empty-block count is six higher than it was before pre-R13 block
    // references resolved: a reference that could not be looked up was never
    // reported as empty, so resolving it made the empty definitions visible. ---
    assert_eq!(s.skipped_edges, 1_034);
    assert_eq!(s.files_with_empty_blocks, 20);
}

/// Which of this library's point fields is which DXF group cannot be read off
/// the field names for a two-line angular dimension: `def_pt` holds group 16
/// there, and `xline2end_pt` holds group 10. The mapping in the reader follows
/// a measurement against the same drawing in both formats, so the measurement
/// is pinned here -- if the library's field layout changes, this fails rather
/// than the model quietly holding the wrong point.
///
/// The second drawing is what settles group 10: in the first, groups 10 and
/// 13 are the same point, so a reading of either looks like the other.
#[test]
fn a_two_line_angular_dimensions_groups_are_the_ones_the_dxf_twin_states() {
    // Two unrelated drawings, so the mapping is not one file's accident.
    angular_groups_match_the_twin(
        "example_2000.dwg",
        (490.6216519543077, 4118.24274338716),
        (490.6216519543077, 4118.24274338716),
        (-276.8548009664508, 4701.847034571434),
        (172.7442546208081, 3207.985617767696),
        (3.542714605046057, 4128.871501442696),
    );
    angular_groups_match_the_twin(
        "2000/TS1.dwg",
        (28.3894217039915, 46.63480213521191),
        (24.13153389940095, 44.46327921516783),
        (28.70342522096354, 44.46327921516783),
        (24.13153389940095, 44.46327921516783),
        (27.19496793714468, 44.94887381898933),
    );
}

fn angular_groups_match_the_twin(
    drawing: &str,
    g10: (f64, f64),
    g13: (f64, f64),
    g14: (f64, f64),
    g15: (f64, f64),
    g16: (f64, f64),
) {
    let path = Path::new(CORPUS).join(drawing);
    let db = uncad::parse(&path).expect("the corpus drawing parses");
    let dim = db
        .entities
        .iter()
        .find_map(|e| match e {
            Entity::Dimension(d) if d.kind == Some(uncad::model::DimensionKind::Angular2Line) => {
                Some(d)
            }
            _ => None,
        })
        .expect("the drawing has a two-line angular dimension");

    // The values the DXF twin writes for groups 10, 13, 14, 15 and 16.
    let near = |got: Option<uncad::model::Point3D>, want: (f64, f64), group: u32| {
        let got = got.unwrap_or_else(|| panic!("group {group} should be read"));
        assert!(
            (got.x - want.0).abs() < 1e-6 && (got.y - want.1).abs() < 1e-6,
            "group {group}: got ({}, {}), the file states ({}, {})",
            got.x,
            got.y,
            want.0,
            want.1
        );
    };
    near(dim.points.extension1, g13, 13);
    near(dim.points.extension2, g14, 14);
    near(dim.points.radial, g15, 15);
    near(dim.points.arc, g16, 16);
    near(dim.definition_point, g10, 10);
}
