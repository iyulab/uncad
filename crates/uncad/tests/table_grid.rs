//! A table in a DWG: the rows, columns and cells its record states, and
//! nothing beyond what it states.
//!
//! The corpus drawing holds one table, saved in six releases. The records up
//! to R2007 state its rows, columns and cells (the DXF twin of each says the
//! same, cell for cell); from R2010 the entity states none of them -- they
//! sit in a separate content object this crate does not read -- so the grid
//! is "not read", which is not the same answer as an empty table.

use uncad::model::{AcadTableEntity, Entity, TableCellKind, TableGrid};

fn corpus(file: &str) -> String {
    format!(
        "{}/../../lib/libredwg/test/test-data/{file}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn table_of(file: &str) -> (AcadTableEntity, Vec<String>) {
    let db = uncad::parse(corpus(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
    let tables: Vec<&AcadTableEntity> = db
        .entities
        .iter()
        .filter_map(|e| match e {
            Entity::AcadTable(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(tables.len(), 1, "{file}: one table");
    (tables[0].clone(), db.read_diagnostics.warnings)
}

fn shape_of(grid: &TableGrid) {
    assert_eq!(grid.rows.len(), 20);
    assert_eq!(grid.column_widths, vec![63.5; 9]);
    assert!(grid.rows.iter().all(|r| r.cells.len() == 9));
    assert!(grid
        .rows
        .iter()
        .all(|r| r.height == 9.0 || r.height == 11.0));
    assert!(grid
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .all(|c| c.kind == Some(TableCellKind::Text)));
    // The first row is one cell spanning all nine columns; the eight under
    // its span show nothing of their own.
    let first = &grid.rows[0].cells;
    assert_eq!((first[0].span_columns, first[0].span_rows), (9, 1));
    assert!(!first[0].covered);
    assert!(first[1..].iter().all(|c| c.covered));
    assert!(grid
        .rows
        .iter()
        .skip(1)
        .flat_map(|r| &r.cells)
        .all(|c| !c.covered && (c.span_columns, c.span_rows) == (1, 1)));
}

#[test]
fn a_table_up_to_r2004_carries_every_row_column_and_cell_text() {
    for file in ["example_2000.dwg", "example_2004.dwg"] {
        let (table, warnings) = table_of(file);
        let grid = table
            .grid
            .as_ref()
            .unwrap_or_else(|| panic!("{file}: a grid"));
        shape_of(grid);
        // The text of the first column: two rows of one word, then five of
        // another; every other cell says nothing.
        let texts: Vec<(usize, usize, &str)> = grid
            .rows
            .iter()
            .enumerate()
            .flat_map(|(r, row)| {
                row.cells
                    .iter()
                    .enumerate()
                    .filter_map(move |(c, cell)| cell.text.as_deref().map(|t| (r, c, t)))
            })
            .collect();
        assert_eq!(
            texts,
            vec![
                (0, 0, "test"),
                (1, 0, "test"),
                (2, 0, "xx"),
                (3, 0, "xx"),
                (4, 0, "xx"),
                (5, 0, "xx"),
                (6, 0, "xx"),
            ],
            "{file}"
        );
        assert!(
            !warnings.iter().any(|w| w.starts_with("TABLE_")),
            "{file}: {warnings:?}"
        );
    }
}

/// R2007 keeps a cell's text in its value, and the backend returns the
/// string of a string value empty. A cell whose text cannot be told from
/// "none" would read as an empty cell to a consumer, so the table's grid is
/// not read at all -- and the warning says how many cells it could not read
/// the text of.
#[test]
fn a_table_in_r2007_is_not_read_when_it_cannot_read_the_text_of_its_cells() {
    let (table, warnings) = table_of("example_2007.dwg");
    assert!(table.grid.is_none(), "no grid that leaves out stated text");
    assert!(
        warnings
            .iter()
            .any(|w| w.starts_with("TABLE_CELL_TEXT") && w.contains("has 7 cells")),
        "{warnings:?}"
    );
}

#[test]
fn a_table_from_r2010_is_there_with_its_grid_not_read() {
    for file in ["example_2010.dwg", "example_2013.dwg", "example_2018.dwg"] {
        let (table, _) = table_of(file);
        assert!(
            table.grid.is_none(),
            "{file}: no rows are stated, none are made up"
        );
        assert!(table.flow.is_none(), "{file}");
        assert!(
            matches!(table.block_name, uncad::model::Ref::Resolved(_)),
            "{file}: the block that draws it is still named: {:?}",
            table.block_name
        );
    }
}
