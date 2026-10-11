use std::path::PathBuf;
fn main() {
    for arg in std::env::args().skip(1) {
        let p = PathBuf::from(&arg);
        let bytes = std::fs::read(&p).unwrap();
        match xenomit_client::assets::awd::parse_aw2(&bytes) {
            Ok(m) => println!(
                "{}: OK {:?} {} verts {} tris",
                p.display(),
                m.name,
                m.positions.len(),
                m.triangles.len()
            ),
            Err(e) => println!("{}: ERR {e:#}", p.display()),
        }
    }
}
