//! Dumps a decoded save file: cargo run --release -p uk-assets --example save_dump -- lvl1progress.bepis [slot]
fn main() {
    let name = std::env::args().nth(1).unwrap_or("lvl1progress.bepis".into());
    let slot = std::env::args().nth(2).unwrap_or("1".into());
    let p = uk_assets::find_install().unwrap().join("Saves").join(format!("Slot{slot}")).join(name);
    let b = std::fs::read(&p).unwrap();
    match uk_assets::nrbf::decode(&b) {
        Ok(v) => println!("{v:?}"),
        Err(e) => println!("ERR {e}"),
    }
}
