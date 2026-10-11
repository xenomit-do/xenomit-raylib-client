use xenomit_client::assets::uberlib::UberAssets;

fn main() {
    let xml = std::fs::read_to_string(
        r"C:\Users\Yagiz\Desktop\repos\game_resources\scripts\_assets\191_net.bigpoint.darkorbit.mvc.common.model.assets.UberAssetsLib_LIB_COLLECTABLES_XML.bin",
    ).unwrap();
    let uber = UberAssets::parse(&[("lib_collectables", &xml)]).unwrap();
    for id in ["ore_0", "ore_1", "ore_2", "ore_8", "ore_17"] {
        match uber.display_3d(id) {
            Some(d) => {
                println!(
                    "== {id}: scale={} visual_size={} meshes={} particles={}",
                    d.scale,
                    d.visual_size,
                    d.meshes.len(),
                    d.particles.len()
                );
                for m in &d.meshes {
                    println!("   mesh id={} geo={:?} tex={:?} texset={:?} pos={:?} rot={:?} scale={} mat={:?}",
                        m.id, m.geometry, m.texture, m.texture_settings, m.position, m.rotation, m.scale, m.material);
                }
                for p in &d.particles {
                    println!(
                        "   particles res={:?} pos={:?} scale={:?}",
                        p.resource, p.position, p.scale
                    );
                }
            }
            None => println!("== {id}: NO display3D"),
        }
    }
}
