//! A MULTILEADER's leader lines and what they point out, from every
//! release that has them.
//!
//! A leader line's own "type" is stored from R2010 on; before it the field
//! reads 0, which once dropped every line of an older drawing. The line is
//! geometry the file states whatever its type says about how it is drawn.

use uncad::model::{Entity, MultiLeaderContent, Ref};

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
                Entity::MultiLeader(m) => {
                    Some(m.leaders.iter().flat_map(|r| &r.lines).map(Vec::len).sum())
                }
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

/// The text a MULTILEADER points out, read from the DWG record's context
/// data the way the DXF twin states it: its text in MTEXT codes, its style
/// by name, where it is and how high.
#[test]
fn a_multileader_carries_its_text() {
    for (file, text) in [
        ("2000/Leader.dwg", "LEADER"),
        ("2018/Leader.dwg", "LEADER"),
        ("example_2000.dwg", r"xx\P\pxt1;xx"),
        ("example_2018.dwg", r"xx\P\pxt1;xx"),
    ] {
        let db = uncad::parse(corpus(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let contents: Vec<&MultiLeaderContent> = db
            .entities
            .iter()
            .filter_map(|e| match e {
                Entity::MultiLeader(m) => m.content.as_ref(),
                _ => None,
            })
            .collect();
        let [MultiLeaderContent::MText(t)] = contents.as_slice() else {
            panic!("{file}: one MULTILEADER with a text: {contents:?}");
        };
        assert_eq!(t.text, text, "{file}");
        assert_eq!(
            t.style_name,
            Ref::Resolved("Standard".to_string()),
            "{file}"
        );
        assert!(t.height > 0.0, "{file}: {}", t.height);
        assert!(t.attachment.is_some(), "{file}");
    }
}
