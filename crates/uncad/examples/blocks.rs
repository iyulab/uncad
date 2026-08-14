fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: blocks <file.dwg> [block-name]");
    let db = uncad::parse(&path).expect("parse failed");
    println!("total block records: {}", db.tables.block_records.len());

    if let Some(name) = std::env::args().nth(2) {
        match db.tables.block_records.get(&name) {
            Some(b) => {
                for e in &b.entities {
                    println!("{}", e.type_name());
                }
            }
            None => println!("no block named {name:?}"),
        }
        return;
    }

    let mut names: Vec<_> = db.tables.block_records.keys().collect();
    names.sort();
    for name in names {
        println!(
            "{name}: {} entities",
            db.tables.block_records[name].entities.len()
        );
    }
}
