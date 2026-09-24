//! A MULTILEADER's leader lines, from every release that has them.
//!
//! A leader line's own "type" is stored from R2010 on; before it the field
//! reads 0, which once dropped every line of an older drawing. The line is
//! geometry the file states whatever its type says about how it is drawn.

use uncad::model::Entity;

fn corpus(file: &str) -> String {
    format!(
        "{}/../../lib/libredwg/test/test-data/{file}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn a_multileader_keeps_its_leader_line_before_r2010_too() {
    for file in [
        "2000/Leader.dwg",
        "2004/Leader.dwg",
        "2007/Leader.dwg",
        "2010/Leader.dwg",
    ] {
        let db = uncad::parse(corpus(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let lines: Vec<usize> = db
            .entities
            .iter()
            .filter_map(|e| match e {
                Entity::MultiLeader(m) => Some(m.lines.iter().map(Vec::len).sum()),
                _ => None,
            })
            .collect();
        assert!(!lines.is_empty(), "{file}: a MULTILEADER");
        assert!(
            lines.iter().all(|&points| points > 0),
            "{file}: every MULTILEADER has a leader line with points: {lines:?}"
        );
    }
}
