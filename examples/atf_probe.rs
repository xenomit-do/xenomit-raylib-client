use std::path::PathBuf;
fn main() {
    for arg in std::env::args().skip(1) {
        let p = PathBuf::from(&arg);
        let bytes = std::fs::read(&p).unwrap();
        match xenomit_client::assets::atf::decode(&bytes) {
            Ok(img) => println!("{}: OK {}x{}", p.display(), img.width(), img.height()),
            Err(e) => println!("{}: ERR {e:#}", p.display()),
        }
    }
}
