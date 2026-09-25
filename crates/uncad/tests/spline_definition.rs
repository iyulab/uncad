//! A SPLINE read from a DXF carries the whole definition the file writes:
//! its knots, and its weights when it has them, go with its control points
//! whichever form the record is flagged as.
//!
//! A DWG record of the fit-point form stores no control points, knots or
//! weights. A DXF of that form writes the control points and knots its
//! program computed beside the fit points, and they define the curve all the
//! same; a reader that keeps the control points must keep the knots too, or
//! the spline arrives with a definition that does not add up.

use std::fs;
use std::path::{Path, PathBuf};
use uncad::model::SplineEntity;
use uncad::Entity;

/// Removes its file on drop.
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

fn splines(path: &Path) -> Vec<SplineEntity> {
    uncad::parse(path)
        .expect("the DXF should parse")
        .entities
        .into_iter()
        .filter_map(|e| match e {
            Entity::Spline(s) => Some(s),
            _ => None,
        })
        .collect()
}

fn complete(s: &SplineEntity) -> bool {
    s.knots.len() == s.control_points.len() + s.degree as usize + 1
}

/// A DXF from the LibreDWG corpus whose splines are fit-point splines with
/// the computed control points and knots written beside them.
const CORPUS_DXF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../lib/libredwg/test/test-data/example_2000.dxf"
);

#[test]
fn a_fit_point_spline_keeps_the_knots_its_dxf_writes() {
    let all = splines(Path::new(CORPUS_DXF));
    let with_control: Vec<_> = all
        .iter()
        .filter(|s| !s.control_points.is_empty())
        .collect();
    assert!(
        !with_control.is_empty(),
        "the corpus DXF has control-point splines"
    );
    for s in &with_control {
        assert!(
            !s.fit_points.is_empty(),
            "these are fit-point splines: {s:?}"
        );
        assert!(
            complete(s),
            "knots {} for {} control points",
            s.knots.len(),
            s.control_points.len()
        );
    }
}

/// A rational spline flagged as the fit-point form (group 70 bits 1024 and
/// 32), with one weight per control point in group 41.
fn rational_fit_form(weights: Option<[&str; 3]>) -> String {
    let mut pairs: Vec<(u16, String)> = vec![
        (0, "SECTION".into()),
        (2, "TABLES".into()),
        (0, "TABLE".into()),
        (2, "LAYER".into()),
        (0, "LAYER".into()),
        (2, "0".into()),
        (70, "0".into()),
        (0, "ENDTAB".into()),
        (0, "ENDSEC".into()),
        (0, "SECTION".into()),
        (2, "ENTITIES".into()),
        (0, "SPLINE".into()),
        (8, "0".into()),
        (70, (1024 + 32 + 8 + 4).to_string()),
        (71, "2".into()),
        (72, "6".into()),
        (73, "3".into()),
        (74, "0".into()),
    ];
    for k in ["0.0", "0.0", "0.0", "1.0", "1.0", "1.0"] {
        pairs.push((40, k.into()));
    }
    for (i, (x, y)) in [("0.0", "20.0"), ("-8.0", "10.0"), ("0.0", "0.0")]
        .into_iter()
        .enumerate()
    {
        pairs.push((10, x.into()));
        pairs.push((20, y.into()));
        pairs.push((30, "0.0".into()));
        if let Some(w) = weights {
            pairs.push((41, w[i].into()));
        }
    }
    pairs.push((0, "ENDSEC".into()));
    pairs.push((0, "EOF".into()));
    pairs
        .iter()
        .map(|(code, value)| format!("{code:>3}\n{value}\n"))
        .collect()
}

#[test]
fn a_fit_point_spline_keeps_the_weights_its_dxf_writes() {
    let file = TempFile::new("rational-fit-form-spline.dxf");
    fs::write(file.path(), rational_fit_form(Some(["1.0", "0.5", "1.0"])))
        .expect("temp dir writable");
    let all = splines(file.path());
    assert_eq!(all.len(), 1, "{all:?}");
    assert!(complete(&all[0]), "{:?}", all[0]);
    // Without its weights the curve would be a different one: the
    // unweighted parabola through the same control points.
    assert_eq!(all[0].weights, vec![1.0, 0.5, 1.0]);
}

#[test]
fn a_spline_whose_dxf_writes_no_weights_has_none() {
    let file = TempFile::new("unweighted-fit-form-spline.dxf");
    fs::write(file.path(), rational_fit_form(None)).expect("temp dir writable");
    let all = splines(file.path());
    assert_eq!(all.len(), 1, "{all:?}");
    assert!(complete(&all[0]), "{:?}", all[0]);
    assert!(all[0].weights.is_empty(), "{:?}", all[0].weights);
}
