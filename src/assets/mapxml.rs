//! Static per-map descriptors (`maps-config` resource) —
//! `Map.getStaticMapDescriptor` equivalent.
//!
//! The `maps-config` file (id `"maps-config"` in resources.xml) holds
//! `<map id name scaleFactor poiColor>` entries with `<backgrounds>` and
//! `<planets>` children. Attribute semantics follow `Background2D.as`
//! (`loadView` — lines ~139-210) and `PlanetVO`:
//!
//! - `isMain` — explicit `isMain` attr, else `true` when the map has exactly
//!   one background.
//! - `region` — `"w,h"` (two values) or `"x,y,w,h"` (four); pixel crop rect.
//! - `pFactor` — parallax divisor, default 10 (int for backgrounds,
//!   Number for planets).
//! - `layer`/`shiftX`/`shiftY`/`scale`/`maskID` — defaults 0/0/0/1/-1.
//! - `<planet typeID x y rotation pFactor layer scale quality_settings/>` —
//!   `typeID` is a STRING key into `GameXml.planets` (e.g. `"1"`,
//!   `planetPatterns[typeID].resKey` is what `Planet2D` loads);
//!   `scale` defaults to 1 (`Number(@scale) || 1`).

use anyhow::{Context, Result};
use std::collections::HashMap;

/// One `<background>` element of a `<map>` (a `BackgroundVO` in AS3).
#[derive(Clone, Debug, Default)]
pub struct MapBackground {
    /// `typeID` — key into `GameXml.backgrounds`.
    pub type_id: i32,
    /// `isMain` — explicit attr, else `backgrounds.len() == 1`.
    pub is_main: bool,
    /// `pFactor` — parallax divisor (0 pins the layer; default 10).
    pub parallax_factor: i32,
    /// `layer` — draw order (default 0).
    pub layer: i32,
    /// `shiftX`/`shiftY` — constant pixel offsets (default 0).
    pub shift: (i32, i32),
    /// `scale` — content scale (default 1).
    pub scale: f32,
    /// `maskID` — `backgroundMasks` lookup for cut-outs (-1 = none).
    pub mask_id: i32,
    /// `region` — pixel crop `(x,y,w,h)`; `"w,h"` maps to `(0,0,w,h)`.
    pub region: Option<(i32, i32, i32, i32)>,
}

/// One `<lensflare>` element of a `<map>` — `Background2D`'s
/// `LensFlareVO`: parallax sun flare drawn on top of the world.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapLensflare {
    /// `id` — flare identifier (shared with the 3D scene list).
    pub id: i32,
    /// `x`/`y` — parallax-space anchor.
    pub pos: (f32, f32),
    /// `pFactor` — parallax divisor (0 pins the flare to the screen).
    pub p_factor: f32,
    /// `star` — adds the rotating `star` clip.
    pub star: bool,
    /// `typeID` — selects the `lensflare{typeID}` SWF (default 0).
    pub type_id: i32,
    /// `blendMode` — child blend mode (`"add"` → additive).
    pub blend: String,
}

/// One `<planet>` element of a `<map>` (a `PlanetVO` in AS3).
#[derive(Clone, Debug, Default)]
pub struct MapPlanet {
    /// `typeID` — string key into `GameXml.planets` (`planetPatterns`).
    pub type_id: String,
    /// `x`/`y` — parallax-space position.
    pub pos: (i32, i32),
    /// `rotation` — degrees.
    pub rotation: f32,
    /// `pFactor` — parallax divisor (`Number`, default 0 → screen-pinned).
    pub parallax_factor: f32,
    /// `layer` — draw order vs backgrounds.
    pub layer: i32,
    /// `scale` — `Number(@scale) || 1`.
    pub scale: f32,
    /// `quality_settings` — quality gate token.
    pub quality: String,
}

/// One `<map>` element.
#[derive(Clone, Debug, Default)]
pub struct MapDescriptor {
    /// `id`.
    pub id: i32,
    /// `name`.
    pub name: String,
    /// `scaleFactor` — world size multiplier (Map.as:229, default 1).
    pub scale_factor: f32,
    /// `poiColor` — decimal RGB uint (InGameCatalog.as:1462,
    /// default `0xFFFFFF`).
    pub poi_color: u32,
    /// `backgroundColor` — hex string parsed as `0x{v}`
    /// (`DarkOrbitMapView2D.loadView` → `stage.color`, the deep-space
    /// fill under the whole map; default black).
    pub background_color: u32,
    /// `music` — comma-separated `trackId[,delaySeconds,trackId…]` list
    /// consumed by `MapAudioManager.update`; "" = no map music.
    pub music: String,
    /// `loopLastTrack` — AS3 `String(attr) !== "false"` (default true).
    pub loop_last_track: bool,
    /// `disableCombatMusic` — AS3 `String(attr) !== "false"` — the quirk
    /// is stock-faithful: an absent attribute disables combat music, so
    /// only an explicit `"false"` enables it.
    pub disable_combat_music: bool,
    /// `minimap` — `MiniMapBackground` art key: the `minimap-{v}` /
    /// `minimap-{v}-{300,700}` file ids are tried before `minimap-{id}`.
    pub minimap: String,
    pub backgrounds: Vec<MapBackground>,
    pub planets: Vec<MapPlanet>,
    /// `<lensflares>` — the 2D provider's `LensFlareVO` list.
    pub lensflares: Vec<MapLensflare>,
    /// `starfield` — raw `Starfield3D` group id (`star_dust_chaotic`,
    /// `star_dust`…). `""` = absent (stock then defaults to
    /// `star_dust_chaotic`); `"none"` disables the 3D starfield.
    pub starfield: String,
}

#[derive(Default)]
pub struct MapDb {
    pub maps: HashMap<i32, MapDescriptor>,
}

fn attr_i32(node: &roxmltree::Node, name: &str) -> i32 {
    node.attribute(name)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_default()
}

fn attr_f32(node: &roxmltree::Node, name: &str, default: f32) -> f32 {
    node.attribute(name)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn attr_bool(node: &roxmltree::Node, name: &str) -> bool {
    node.attribute(name).is_some_and(|v| v == "true")
}

impl MapDb {
    /// Parse the `maps-config` XML document. The document may carry
    /// template/references markup — it is reference-resolved by the caller
    /// (`MapManagerProxy` resolves before handing the config to `Map`).
    pub fn parse(xml: &str) -> Result<Self> {
        let doc = roxmltree::Document::parse(xml).context("invalid maps-config")?;
        let mut db = MapDb::default();
        for map in doc
            .descendants()
            .filter(|n| n.is_element() && n.has_tag_name("map"))
        {
            let mut desc = MapDescriptor {
                id: attr_i32(&map, "id"),
                name: map.attribute("name").unwrap_or_default().to_string(),
                scale_factor: attr_f32(&map, "scaleFactor", 1.0),
                poi_color: {
                    let v = map.attribute("poiColor").unwrap_or_default();
                    if let Some(hex) = v.strip_prefix("0x") {
                        u32::from_str_radix(hex, 16).unwrap_or(0xFFFFFF)
                    } else {
                        v.parse().unwrap_or(0xFFFFFF)
                    }
                },
                background_color: map
                    .attribute("backgroundColor")
                    .map(|v| {
                        // Stock: `uint("0x" + backgroundColor)` — the
                        // attribute is a hex string without a prefix.
                        u32::from_str_radix(v.trim_start_matches("0x"), 16).unwrap_or(0)
                    })
                    .unwrap_or(0),
                music: map.attribute("music").unwrap_or_default().to_string(),
                loop_last_track: map.attribute("loopLastTrack") != Some("false"),
                disable_combat_music: map.attribute("disableCombatMusic") != Some("false"),
                minimap: map.attribute("minimap").unwrap_or_default().to_string(),
                starfield: map.attribute("starfield").unwrap_or_default().to_string(),
                ..Default::default()
            };
            for section in map.children().filter(|n| n.is_element()) {
                if section.has_tag_name("backgrounds") {
                    let bgs: Vec<_> = section
                        .children()
                        .filter(|n| n.is_element() && n.has_tag_name("background"))
                        .collect();
                    let single = bgs.len() == 1;
                    for bg in bgs {
                        desc.backgrounds.push(MapBackground {
                            type_id: attr_i32(&bg, "typeID"),
                            is_main: if bg.attribute("isMain").is_some() {
                                attr_bool(&bg, "isMain")
                            } else {
                                single
                            },
                            parallax_factor: if bg.attribute("pFactor").is_some() {
                                attr_i32(&bg, "pFactor")
                            } else {
                                10
                            },
                            layer: attr_i32(&bg, "layer"),
                            shift: (attr_i32(&bg, "shiftX"), attr_i32(&bg, "shiftY")),
                            scale: if bg.attribute("scale").is_some() {
                                attr_f32(&bg, "scale", 1.0)
                            } else {
                                1.0
                            },
                            mask_id: if bg.attribute("maskID").is_some() {
                                attr_i32(&bg, "maskID")
                            } else {
                                -1
                            },
                            region: bg.attribute("region").and_then(|r| {
                                let v: Vec<i32> =
                                    r.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                                match v.as_slice() {
                                    [x, y, w, h] => Some((*x, *y, *w, *h)),
                                    [w, h] => Some((0, 0, *w, *h)),
                                    _ => None,
                                }
                            }),
                        });
                    }
                } else if section.has_tag_name("planets") {
                    for pl in section
                        .children()
                        .filter(|n| n.is_element() && n.has_tag_name("planet"))
                    {
                        desc.planets.push(MapPlanet {
                            type_id: pl.attribute("typeID").unwrap_or_default().to_string(),
                            pos: (attr_i32(&pl, "x"), attr_i32(&pl, "y")),
                            rotation: attr_f32(&pl, "rotation", 0.0),
                            parallax_factor: attr_f32(&pl, "pFactor", 0.0),
                            layer: attr_i32(&pl, "layer"),
                            scale: {
                                let s = attr_f32(&pl, "scale", 0.0);
                                if s == 0.0 {
                                    1.0
                                } else {
                                    s
                                }
                            },
                            quality: pl
                                .attribute("quality_settings")
                                .unwrap_or_default()
                                .to_string(),
                        });
                    }
                } else if section.has_tag_name("lensflares") {
                    for flare in section
                        .children()
                        .filter(|n| n.is_element() && n.has_tag_name("lensflare"))
                    {
                        desc.lensflares.push(MapLensflare {
                            id: attr_i32(&flare, "id"),
                            pos: (attr_f32(&flare, "x", 0.0), attr_f32(&flare, "y", 0.0)),
                            p_factor: attr_f32(&flare, "pFactor", 0.0),
                            star: attr_bool(&flare, "star"),
                            type_id: attr_i32(&flare, "typeID"),
                            blend: flare
                                .attribute("blendMode")
                                .unwrap_or_default()
                                .to_string(),
                        });
                    }
                }
            }
            db.maps.insert(desc.id, desc);
        }
        Ok(db)
    }

    pub fn get(&self, map_id: i32) -> Option<&MapDescriptor> {
        self.maps.get(&map_id)
    }
}

impl MapDescriptor {
    /// `Starfield3D.reloadEffect`: an explicit `starfield` id wins,
    /// `"none"` disables the starfield, absent falls back to
    /// `star_dust_chaotic`.
    pub fn starfield_group(&self) -> Option<String> {
        match self.starfield.as_str() {
            "none" => None,
            "" => Some("star_dust_chaotic".to_string()),
            id => Some(id.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MapDb;

    /// `MiniMapBackground` reads `mapXml.@minimap` — the art key prefix
    /// for `minimap-{v}` / `minimap-{v}-{300,700}` file ids.
    #[test]
    fn minimap_attr_parses() {
        let db = MapDb::parse(
            r#"<maps>
                <map id="16" minimap="101" scaleFactor="1"/>
                <map id="42"/>
            </maps>"#,
        )
        .unwrap();
        assert_eq!(db.get(16).unwrap().minimap, "101");
        assert_eq!(db.get(42).unwrap().minimap, "");
    }

    /// `Starfield3D.reloadEffect` — `<map starfield="…">` is a group id
    /// string; absent = stock `star_dust_chaotic` default, `"none"` = off.
    #[test]
    fn starfield_attr_parses() {
        let db = MapDb::parse(
            r#"<maps>
                <map id="16" starfield="star_dust_2"/>
                <map id="17" starfield="none"/>
                <map id="42"/>
            </maps>"#,
        )
        .unwrap();
        assert_eq!(db.get(16).unwrap().starfield, "star_dust_2");
        assert_eq!(db.get(17).unwrap().starfield, "none");
        assert_eq!(db.get(42).unwrap().starfield, "");
    }

    /// Resolved group id: absent -> `star_dust_chaotic`, `none` -> `None`.
    #[test]
    fn starfield_group_resolves() {
        let db = MapDb::parse(
            r#"<maps>
                <map id="16" starfield="star_dust_2"/>
                <map id="17" starfield="none"/>
                <map id="42"/>
            </maps>"#,
        )
        .unwrap();
        assert_eq!(
            db.get(16).unwrap().starfield_group(),
            Some("star_dust_2".to_string())
        );
        assert_eq!(db.get(17).unwrap().starfield_group(), None);
        assert_eq!(
            db.get(42).unwrap().starfield_group(),
            Some("star_dust_chaotic".to_string())
        );
    }

    /// `Background2D.lensFlares` — `<lensflares><lensflare>` rows.
    #[test]
    fn lensflares_parse() {
        let db = MapDb::parse(
            r#"<maps>
                <map id="1">
                    <lensflares>
                        <lensflare id="1" x="1740" y="1106" pFactor="10" star="true"/>
                        <lensflare id="2" x="-500" y="200" pFactor="0" typeID="3" blendMode="add"/>
                    </lensflares>
                </map>
            </maps>"#,
        )
        .unwrap();
        let flares = &db.get(1).unwrap().lensflares;
        assert_eq!(flares.len(), 2);
        assert_eq!(flares[0].pos, (1740.0, 1106.0));
        assert_eq!(flares[0].p_factor, 10.0);
        assert!(flares[0].star);
        assert_eq!(flares[1].type_id, 3);
        assert_eq!(flares[1].blend, "add");
        assert_eq!(db.get(2).map(|d| d.lensflares.len()), None);
    }
}
