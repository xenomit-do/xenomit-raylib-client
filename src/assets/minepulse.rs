//! `<minePulseColors><minePulseColor>` — `MinePulsePattern` rows
//! (`PatternManager.minePulsePatterns`).
//!
//! ```xml
//! <minePulseColor id="1" resKey="minePulse" color="ff0000" alpha="0.7"
//!                 scale="2" resKey3D="glow_fast_flickering_red"/>
//! ```
//!
//! `AddMineCommand.maxSkill1` sets the collectable's `pulseColorId`
//! (`AddMineHandler.as:17`); `MineDecorator2D` then loads the `minePulse`
//! SWF's `mc`, scales it by `scale` and applies a `ColorTransform` whose
//! RGB is *replaced* by `color` at `alpha` opacity — a looping tinted
//! pulse over the mine.

/// One `<minePulseColor>` row (`MinePulsePattern(id, resKey, color,
/// alpha, scale, resKey3D)`).
#[derive(Clone, Debug, Default)]
pub struct MinePulsePattern {
    /// `id` — `Mine.pulseColorId` lookup key (`0` disables the decorator).
    pub id: i32,
    /// `resKey` — resources.xml file id of the pulse SWF (`minePulse`,
    /// `graphics_mines`); the `mc` class loops.
    pub res_key: String,
    /// `color` — RGB the clip is replaced with (`ColorTransform.color`;
    /// hex string in the file, e.g. `"ff0000"`).
    pub color: u32,
    /// `alpha` — `ColorTransform.alphaMultiplier` (default 1).
    pub alpha: f32,
    /// `scale` — clip `scaleX`/`scaleY` (default 1).
    pub scale: f32,
    /// `resKey3D` — `MineDecorator3D` effect id (`glow_fast_flickering_*`).
    pub res_key_3d: String,
}

/// Parse `<minePulseColors>` under the `<patterns>` element.
pub fn parse_mine_pulses(
    patterns: &roxmltree::Node,
    out: &mut std::collections::HashMap<i32, MinePulsePattern>,
) {
    for section in patterns
        .children()
        .filter(|n| n.is_element() && n.has_tag_name("minePulseColors"))
    {
        for row in section
            .children()
            .filter(|n| n.is_element() && n.has_tag_name("minePulseColor"))
        {
            let pat = MinePulsePattern {
                id: row
                    .attribute("id")
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or_default(),
                res_key: row.attribute("resKey").unwrap_or_default().to_string(),
                // AS3: `uint("0x" + color)` — bare hex string.
                color: row
                    .attribute("color")
                    .map(|v| {
                        u32::from_str_radix(v.trim_start_matches("0x"), 16).unwrap_or(0)
                    })
                    .unwrap_or(0),
                alpha: row
                    .attribute("alpha")
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(1.0),
                scale: row
                    .attribute("scale")
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(1.0),
                res_key_3d: row.attribute("resKey3D").unwrap_or_default().to_string(),
            };
            out.insert(pat.id, pat);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::assets::GameXml;

    /// Fixture: three `minePulse` rows — colour hex, alpha/scale and the
    /// 3D resKey come through verbatim.
    #[test]
    fn mine_pulses_parse_from_fixture() {
        let xml = std::fs::read_to_string("tests/fixtures/game.xml").expect("fixture missing");
        let game = GameXml::parse(&xml).expect("game.xml parses");

        let red = &game.mine_pulses[&1];
        assert_eq!(red.res_key, "minePulse");
        assert_eq!(red.color, 0xFF0000);
        assert!((red.alpha - 0.7).abs() < 1e-6);
        assert!((red.scale - 2.0).abs() < 1e-6);
        assert_eq!(red.res_key_3d, "glow_fast_flickering_red");
        assert_eq!(game.mine_pulses[&2].color, 0x00FF00);
        assert_eq!(game.mine_pulses[&3].color, 0xFFFFFF);
    }
}
