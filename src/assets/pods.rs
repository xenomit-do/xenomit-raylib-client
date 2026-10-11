//! Ship pod overlays — `<resources><podOverlay>` on `<ship>`/`<design>`
//! patterns (`PatternManager.setShipPatternAttributes` ~PatternManager.as:631-658).
//!
//! ```xml
//! <ship type="ship_aegis" podType="1">
//!     <resources><podOverlay factionId="0" srcKey="ship49_pod" resKey="mc"/></resources>
//!     <designs><design ...>
//!         <resources><podOverlay factionId="0" srcKey="aegis_pod_frozen" resKey="mc"/></resources>
//!     </design></designs>
//! </ship>
//! ```
//!
//! A design's own `<podOverlay>` list *replaces* the cloned base list
//! (AS3 assigns `podResPatterns` unconditionally when the node carries
//! overlays and `podType != POD_TYPE_NONE`). `Pod2D` draws the pod while
//! the ship carries `PodCarrierTrait` — factory-added for `aegis`/
//! `citadel` pattern classes, or via `VisualModifierCommand.
//! VISUALMODIFIER_HEALING_POD` (id 7) on everyone else.

/// `VisualModifierCommand.VISUALMODIFIER_HEALING_POD` — the modifier id
/// that adds `PodCarrierTrait` (`VisualModifierAssembly.as:262-272`).
pub const HEALING_POD_MODIFIER: i32 = 7;

/// One `<podOverlay factionId srcKey resKey>` — `FullResourcePattern`.
#[derive(Clone, Debug, Default)]
pub struct PodPattern {
    /// `factionId` — matched against `ship.info.factionID`; 0 = catch-all.
    pub faction_id: i32,
    /// `srcKey` — resources.xml file id of the pod SWF
    /// (`pods`, `aegis_pod_frozen`, `ship49_pod`).
    pub src_key: String,
    /// `resKey` — exported class (`Pod2D` always loads `"mc"`).
    pub res_key: String,
}

/// `setShipPatternAttributes` lines 647-659: the node's own
/// `<resources><podOverlay>` children, or `None` when it declares none
/// (a design then keeps the cloned base's list).
pub fn parse_pod_overlays(node: &roxmltree::Node) -> Option<Vec<PodPattern>> {
    let resources = node
        .children()
        .find(|n| n.is_element() && n.has_tag_name("resources"))?;
    let overlays: Vec<_> = resources
        .children()
        .filter(|n| n.is_element() && n.has_tag_name("podOverlay"))
        .map(|o| PodPattern {
            faction_id: o
                .attribute("factionId")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or_default(),
            src_key: o.attribute("srcKey").unwrap_or_default().to_string(),
            res_key: o.attribute("resKey").unwrap_or_default().to_string(),
        })
        .collect();
    (!overlays.is_empty()).then_some(overlays)
}

/// `ShipPattern.getPodPattern(factionId)`: first entry whose id equals
/// the faction, else `podResPatterns[0]` (`None` on an empty list).
pub fn pod_pattern(patterns: &[PodPattern], faction_id: i32) -> Option<&PodPattern> {
    patterns
        .iter()
        .find(|p| p.faction_id == faction_id)
        .or_else(|| patterns.first())
}

#[cfg(test)]
mod tests {
    use crate::assets::GameXml;

    /// Fixture: `ship_aegis` podType=1 with `ship49_pod`; the
    /// `a-elite-frost` design override swaps in `aegis_pod_frozen`
    /// while sibling designs inherit the base's `ship49_pod`.
    #[test]
    fn pod_overlays_parse_from_fixture() {
        let xml = std::fs::read_to_string("tests/fixtures/game.xml").expect("fixture missing");
        let game = GameXml::parse(&xml).expect("game.xml parses");

        let aegis = game.ship("ship_aegis").expect("ship_aegis");
        assert_eq!(aegis.pod_type, 1);
        let pod = super::pod_pattern(&aegis.pod_res, 0).expect("pod pattern");
        assert_eq!(pod.src_key, "ship49_pod");
        assert_eq!(pod.res_key, "mc");

        // Design override replaces the pod resource list.
        let frozen = game
            .ship("ship_a-elite_design_a-elite-frost")
            .expect("frozen design");
        assert_eq!(frozen.pod_type, 1);
        assert_eq!(frozen.pod_res.len(), 1);
        assert_eq!(frozen.pod_res[0].src_key, "aegis_pod_frozen");

        // Sibling designs inherit the cloned base list.
        let poison = game
            .ship("ship_a-elite_design_a-elite-poison")
            .expect("poison design");
        assert_eq!(poison.pod_type, 1);
        assert_eq!(poison.pod_res[0].src_key, "ship49_pod");

        // Faction fallback: first entry when no id matches.
        let elite = game.ship("ship_a-elite").expect("a-elite");
        assert_eq!(super::pod_pattern(&elite.pod_res, 3).unwrap().src_key, "ship49_pod");

        // Ships without podType declare no overlays.
        let goliath = game.ship("ship_goliath").expect("goliath");
        assert_eq!(goliath.pod_type, 0);
        assert!(goliath.pod_res.is_empty());
    }
}
