//! game.xml helpers for the 3D-only effect data
//! (`<fireworks>`/`<fireworkDescriptors>` — `FireworkBox`/`FireworksEffect3D`).
//! The scalar 3D attrs (`rocketSmoke fx3D`, `engine3DEffect`,
//! `pyroEffect has3D`/`shockwaveId`, `<shockwaves>`, `shield scale`) are
//! parsed inline in `gamexml.rs`.

use std::collections::HashMap;

/// `FireworkBoxPattern.getEventClass` — `eventModifier` is the class key;
/// an empty value is `"standart"`.
pub fn firework_class(event_modifier: &str) -> &str {
    if event_modifier.is_empty() {
        "standart"
    } else {
        event_modifier
    }
}

/// `FireworkBox.getResourceKeys3D`: the box's `fireworkDescriptor`
/// (`firework_descs[type_id]`) gives the ordered firework *size* ids;
/// each resolves through `fireworks[class][size][colour]` to a particle
/// group stem. Colour fallback per the file's own doc comment is
/// `color{code}` -> `color0` -> `color1`.
pub fn firework_groups(
    descs: &HashMap<i32, Vec<String>>,
    patterns: &HashMap<String, HashMap<String, HashMap<i32, String>>>,
    type_id: i32,
    event_modifier: &str,
    colour_code: i32,
) -> Vec<String> {
    let Some(order) = descs.get(&type_id) else {
        return Vec::new();
    };
    let sizes = patterns.get(firework_class(event_modifier));
    order
        .iter()
        .filter_map(|size| {
            let colours = sizes?.get(size)?;
            colours
                .get(&colour_code)
                .or_else(|| colours.get(&0))
                .or_else(|| colours.get(&1))
                .cloned()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::gamexml::GameXml;

    fn fixture() -> GameXml {
        let xml = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/game.xml"
        ))
        .unwrap();
        GameXml::parse(&crate::swf_extract::strip_html_comments(&xml)).unwrap()
    }

    #[test]
    fn firework_groups_from_fixture() {
        let g = fixture();
        // typeId 33 -> order "3,2,3"; SolarClash only has color0.
        let sc = firework_groups(&g.firework_descs, &g.fireworks, 33, "SolarClash", 2);
        assert_eq!(
            sc,
            vec![
                "firework_large_short_term_event",
                "firework_medium_short_term_event",
                "firework_large_short_term_event",
            ]
        );
        // standart colour 2 -> blue names; order "1,1" for typeId 31.
        let st = firework_groups(&g.firework_descs, &g.fireworks, 31, "", 2);
        assert_eq!(st, vec!["firework_small_blue", "firework_small_blue"]);
        // colour 0 -> color0 fallback then color1 ("0 than 1").
        let fb = firework_groups(&g.firework_descs, &g.fireworks, 31, "", 0);
        assert_eq!(fb, vec!["firework_small_red", "firework_small_red"]);
        assert!(firework_groups(&g.firework_descs, &g.fireworks, 99, "", 0).is_empty());
        assert!(firework_groups(&g.firework_descs, &g.fireworks, 31, "Nope", 0).is_empty());
    }

    #[test]
    fn fx3d_attrs_from_fixture() {
        let g = fixture();
        assert_eq!(g.rocket_smokes[&0].fx3d, "rocket_trail_1.zip");
        // `engine3DEffect` on the hyperion-cerevisia pet ship.
        let pet = g
            .ships_by_type
            .values()
            .find(|p| p.engine_3d == "engine_hyperion.zip")
            .expect("a ship with engine3DEffect=engine_hyperion.zip");
        assert!(pet.engine_3d.ends_with(".zip"));
        // Design-level override (wyvern design has its own attr).
        let wyvern = &g.ships_by_type["ship_retiarus-plus_design_retiarus-plus-wyvern"];
        assert_eq!(wyvern.engine_3d, "exhaust_flame_engine_trail_wyvern.zip");
        // Pyro: class-0 defaults has3D, class-2 laser damage opts out.
        assert!(g.pyro[&0][&0].has_3d);
        assert!(!g.pyro[&2][&0].has_3d);
        assert_eq!(g.pyro[&0][&6].shockwave_id, 2);
        let sw = &g.shockwaves[&3];
        assert_eq!(sw.res_key, "shockwave3");
        assert_eq!(sw.radius, 350.0);
        assert_eq!(sw.max_shockwaves, 30);
        assert!(sw.shake_screen);
        assert!(g.shields["default"].res_key.ends_with(".zip"));
        assert_eq!(g.shields["default"].scale, 1.0);
    }
}
