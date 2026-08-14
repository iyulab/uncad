fn main() {
    let path = std::env::args().nth(1).expect("usage: dump <file.dwg>");
    let db = uncad::parse(&path).expect("parse failed");
    for e in &db.entities {
        println!("{e:?}");
    }
}
