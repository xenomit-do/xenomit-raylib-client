//! Drone asset resolution (`Drone.uberAssetDescriptorId` → `LIB_DRONES`).

use super::{AssetManager, Display2D};

/// `Drone.STANDARD_ASSET` — the design every drone has unless skinned.
pub const STANDARD_ASSET: &str = "standard";

/// Ordered `LIB_DRONES` candidates for one drone (`Drone.as`):
///
/// ```text
/// {type}_{assetId}_{level}, {type}_{assetId}, {assetId}
/// ```
///
/// built from the *visual* design's `assetId`, unless the stats design is
/// a real skin while the visual one is `standard` — then the stats
/// design's chain is used instead.
pub fn drone_asset_candidates(
    drone_type: i32,
    level: i32,
    visual_asset: &str,
    stats_asset: &str,
) -> Vec<String> {
    let chain = |asset: &str| {
        vec![
            format!("{drone_type}_{asset}_{level}"),
            format!("{drone_type}_{asset}"),
            asset.to_owned(),
        ]
    };
    if stats_asset != STANDARD_ASSET && visual_asset == STANDARD_ASSET {
        chain(stats_asset)
    } else {
        chain(visual_asset)
    }
}

impl AssetManager {
    /// `DroneDesign.getDesign(id).assetId`; ids missing from the
    /// `<designs>` table resolve to `standard`.
    pub fn drone_design_asset(&self, design_id: i32) -> &str {
        self.game_xml
            .drone_designs
            .get(&design_id)
            .map(|d| d.asset_id.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(STANDARD_ASSET)
    }

    /// Resolve a drone to its `lib_drones` recipe plus the asset id that
    /// matched (`Drone.uberAssetDescriptor`). `stats_design` /
    /// `visual_design` are the `<design id>`s the server sent.
    pub fn drone_display(
        &self,
        drone_type: i32,
        level: i32,
        stats_design: i32,
        visual_design: i32,
    ) -> Option<(Display2D, String)> {
        let candidates = drone_asset_candidates(
            drone_type,
            level,
            self.drone_design_asset(visual_design),
            self.drone_design_asset(stats_design),
        );
        candidates
            .into_iter()
            .find_map(|id| self.uber_lib_display("lib_drones", &id).map(|d| (d, id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{mapxml, FileIndex, GameXml, UberAssets};
    use std::path::{Path, PathBuf};

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new("tests/fixtures").join(name))
            .unwrap_or_else(|e| panic!("cannot read {name}: {e}"))
    }

    fn manager() -> AssetManager {
        let game_xml = GameXml::parse(&fixture("game.xml")).expect("game.xml parses");
        let drones = fixture("lib_drones.xml");
        let uber =
            UberAssets::parse(&[("lib_drones", drones.as_str())]).expect("lib_drones parses");
        AssetManager::new(
            PathBuf::from("target/asset-test-cache"),
            FileIndex::default(),
            game_xml,
            uber,
            mapxml::MapDb::default(),
        )
    }

    #[test]
    fn candidate_chain_order() {
        assert_eq!(
            drone_asset_candidates(1, 3, "havok", "standard"),
            ["1_havok_3", "1_havok", "havok"]
        );
        // Visual `standard` + skinned stats -> the stats chain.
        assert_eq!(
            drone_asset_candidates(2, 6, "standard", "hercules"),
            ["2_hercules_6", "2_hercules", "hercules"]
        );
        // Both standard -> the (visual) standard chain.
        assert_eq!(
            drone_asset_candidates(4, 1, "standard", "standard")[0],
            "4_standard_1"
        );
    }

    #[test]
    fn resolves_drone_recipes_from_lib_drones() {
        let mgr = manager();
        // design 0 = standard, 1 = havok, 2 = hercules.
        let (d, id) = mgr.drone_display(1, 3, 0, 0).expect("flax standard 3");
        assert_eq!(id, "1_standard_3");
        assert_eq!(d.src_key, "drones");
        assert_eq!(d.res_key, "drone_flax_2");

        let (d, id) = mgr.drone_display(1, 4, 0, 1).expect("flax havok 4");
        assert_eq!(id, "1_havok_4");
        assert_eq!(d.res_key, "drone_flax_design_havok_3");

        // visual standard + stats hercules -> hercules skin.
        let (_, id) = mgr.drone_display(2, 2, 2, 0).expect("iris hercules 2");
        assert_eq!(id, "2_hercules_2");

        // Unknown design id -> standard; unknown type -> miss.
        assert_eq!(mgr.drone_display(3, 5, 99, 99).unwrap().1, "3_standard_5");
        assert!(mgr.drone_display(77, 1, 0, 0).is_none());
    }
}
