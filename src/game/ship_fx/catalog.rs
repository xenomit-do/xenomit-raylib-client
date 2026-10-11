//! Per-effect-id view descriptors, distilled from the
//! `map/view2D/effects/impl/*2D.as` classes the stock `EffectsManager2D`
//! `MAP` instantiates. Where a class is a 60-line `start()` stub the
//! defaults apply: `(pattern.resKey, "mc")`, looping `playAnimation`
//! with the 0.5 s `fadeInClip`.
//!
//! Layers follow `AbstractMapObjectEffect2D` ctor flags: a view child
//! of the ship renders below (`addChildAt 0`) or above (`addChild`) the
//! hull; non-child effects live on the fx layer and only follow the
//! entity position.

/// `param4`/`param5` of `AbstractMapObjectEffect2D(view, effect, _,
/// above, child)`: `child=true` parents the view to the ship view
/// (`above` picks over/under the hull); the default `child=false`
/// parents it to `mapView.fxLayer` where it follows the entity
/// position. Nearly every ported view uses the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// Child of the ship view, under the hull (`addChildAt(view, 0)`).
    BelowHull,
    /// Child of the ship view, over hull + drones (`addChild`).
    AboveHull,
    /// `mapView.fxLayer` — above everything, follows the entity.
    Fx,
}

/// `TweenClip.playAnimation(loop, …)` — loop vs play-once-dispose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anim {
    Loop,
    Once,
}

/// Clip rotation modes the ported views use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rot {
    None,
    /// `clip.rotation = ship.rotation` (slow-down, speed-buff engines).
    Ship,
    /// `rotation = hero.moveTrait.rotation - 180` (speed burst).
    HeroMinus180,
    /// `rotation = shipView clipRotation - 180` (solace-style).
    ShipMinus180,
    /// Random once at spawn (insta-shield).
    Random,
}

/// Where the clip bytes come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Res {
    /// game.xml `<effect id>` `resKey`; `""` means the pattern carries
    /// no art — nothing is drawn (stock loads nothing too).
    Pattern,
    /// The class hardcodes the resources.xml file id.
    Fixed(&'static str),
    /// Pattern first, this id when the row's `resKey` is empty.
    PatternOr(&'static str),
}

/// Non-plain-clip behaviour a descriptor can carry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Plain clip drawn centred on the ship.
    Clip,
    /// `ShipGlowEffect2D` — `GlowFilter` on the ship view; approximated
    /// as a pulsing coloured ring around the hull. `a` = 0xRRGGBB.
    Glow,
    /// `MapObjectGlowRingEffect2D` / `GlowingRingEffect2D` — expanding
    /// radial ring, params from `AddFx::ring`.
    Ring,
    /// `DecloakEffect2D` — `("shockwaves","shockring0")` clips scaled
    /// 0.5 -> 3 alpha 0.7 -> 0 on a 2 s repeat.
    Shockring,
    /// `StickyBombEffect2D` — rotating bomb clip by stage (`a`:
    /// 0=`stickyBomb`, 1=`emperorStickyBomb`, 2=`explosiveChargingBlob`)
    /// + a blinking light for the emperor stage.
    StickyBomb,
    /// `SphereShieldEffect2D` — `shield0` clip + inner/outer ring; `a`
    /// = shield percent 0..10.
    SphereShield,
    /// `RepairRobotEffect2D` — `robotsNum` `repair_*` clips orbiting the
    /// hull; `a` selects the `repairRobots` `loot_id` row (0/1).
    RepairRobots,
    /// `SpeedBuffEffect2D`/`TartarusSpeedBoostEffect2D` — engine overlay
    /// clips shown only while the ship moves; `a == 1` also shakes.
    Turbo,
    /// `GhostNPCEffect2D` — tweens the ship view alpha toward `alpha`.
    HullAlpha,
    /// `PlagueEffect2D`/`SpreadEffect2D`/`MayhemWinnerEffect2D` — clip
    /// uniformly scaled so it covers the hull (+`hull_extra`).
    HullFitClip,
    /// `ChainBoltEffect2D` — `Lightning` bolt from the owner to the
    /// chain target (`a` = entity id); params from `bolt`.
    ChainLink,
}

/// `ChainBoltEffect2D.createBoltGraphics` — the `Lightning` bolt's
/// per-link-type shape; the core line is always `colorChainBolt`, this
/// `color` is the `BOLT_FILTER` glow (drawn as the bolt color).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bolt {
    pub color: u32,
    /// `steps` — polyline segment count.
    pub steps: u32,
    pub thickness: f32,
    /// Perlin offset amplitude as a fraction of the bolt length
    /// (`boff = perlin * len * amplitude`; unset cases default to 0.5).
    pub amplitude: f32,
}

/// One `EffectIDList` id -> 2D view descriptor.
#[derive(Clone, Copy, Debug)]
pub struct FxDesc {
    pub id: i32,
    pub res: Res,
    /// Embedded symbol — `"mc"` default; `"level{n}"` resolves through
    /// `game_xml.boss_shields`; `"glow_*"` variants pick by `a`.
    pub symbol: &'static str,
    pub layer: Layer,
    pub anim: Anim,
    /// `playAnimation` rate multiplier (0 => 1).
    pub rate: f32,
    /// Extra uniform scale (`effectMc.scaleX`).
    pub scale: f32,
    /// Divisor for a runtime scale (`HeatReleaseEffect2D`: `a/150`).
    pub scale_a: f32,
    /// `effect` offset in world units.
    pub offset: (f32, f32),
    pub rotate: Rot,
    /// Ship-view jitter amplitude in px (berserk/reversal/rapid-fire 1.5).
    pub shake: f32,
    /// Hull alpha target for `HullAlpha` (ghost 0.4, scramble 0.2).
    pub hull_alpha: f32,
    /// Extra scale for `HullFitClip`.
    pub hull_extra: f32,
    /// Alpha yoyo `(lo, hi, secs)` — deflector 0.2..1/3 s, mark-target
    /// 0.2..1/1 s, energy-leech ~10 s breathing.
    pub alpha_cycle: Option<(f32, f32, f32)>,
    /// Grow `(from, to, secs)` on the clip scale — `to <= 0` grows to
    /// `a * 2 / clip_width` (boss EMP shock range).
    pub grow: Option<(f32, f32, f32)>,
    /// Static glow color for `Kind::Glow`; `a` wins when the model
    /// carries a color (BigPumpkin/Wargame/ShipGlow).
    pub glow: u32,
    /// `Lightning` shape for `Kind::ChainLink`.
    pub bolt: Option<Bolt>,
    pub kind: Kind,
}

const fn d(
    id: i32,
    res: Res,
    symbol: &'static str,
    layer: Layer,
    anim: Anim,
    kind: Kind,
) -> FxDesc {
    FxDesc {
        id,
        res,
        symbol,
        layer,
        anim,
        rate: 1.0,
        scale: 1.0,
        scale_a: 0.0,
        offset: (0.0, 0.0),
        rotate: Rot::None,
        shake: 0.0,
        hull_alpha: 0.0,
        hull_extra: 0.0,
        alpha_cycle: None,
        grow: None,
        glow: 0,
        bolt: None,
        kind,
    }
}

/// The ported slice of `EffectsManager2D.MAP`.
#[rustfmt::skip]
pub static CATALOG: &[FxDesc] = &[
    // --- simple loop clips -------------------------------------------
    // SkillEffect2D subclasses — `start()` = looping `playAnimation`;
    // default `AbstractMapObjectEffect2D` ctor = `fxLayer` child.
    d(4,   Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // diminisher
    FxDesc { rotate: Rot::Ship,
             ..d(9, Res::PatternOr("slow-effect"), "mc", Layer::Fx, Anim::Loop, Kind::Clip) },
    FxDesc { alpha_cycle: Some((0.0, 1.0, 10.0)),
             ..d(11, Res::PatternOr("ela0"), "mc", Layer::Fx, Anim::Loop, Kind::Clip) },
    d(16,  Res::PatternOr("invincibilityShield"), "mc", Layer::Fx, Anim::Loop, Kind::Clip),
    d(18,  Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // skull
    d(37,  Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // protection
    // `ProtectedEffectView2D` hardcodes `("shockwaves","protectionBuffRing")`.
    d(38,  Res::Fixed("shockwaves"), "protectionBuffRing", Layer::Fx, Anim::Loop, Kind::Clip),
    // `DrawFireEffect2D` — looping vortex + `drawFireRing` spawns.
    d(39,  Res::Fixed("shockwaves"), "drawFireVortex", Layer::Fx, Anim::Loop, Kind::Clip),
    d(47,  Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // underConstruction
    d(54,  Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // install_module
    d(55,  Res::Pattern, "malusCloud", Layer::Fx, Anim::Loop, Kind::Clip),
    d(79,  Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // icyCube
    d(89,  Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // disruption
    FxDesc { shake: 1.5, ..d(93, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip) },
    FxDesc { hull_alpha: 0.2,
             ..d(94, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip) },
    FxDesc { alpha_cycle: Some((0.2, 1.0, 3.0)),
             ..d(45, Res::Pattern, "mc", Layer::AboveHull, Anim::Loop, Kind::Clip) },
    FxDesc { scale: 0.6,
             ..d(351, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip) },
    FxDesc { scale: 1.8,
             ..d(328, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip) },
    FxDesc { scale: 3.3,
             ..d(327, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip) },
    FxDesc { scale: 8.0,
             ..d(342, Res::PatternOr("frost-field"), "mc", Layer::Fx, Anim::Loop, Kind::Clip) },
    FxDesc { offset: (0.0, -20.0),
             ..d(336, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip) },
    d(337, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // assimilate
    d(345, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // prismatic_endurance
    d(347, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // stockpile
    d(349, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // JAMX damage
    d(353, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // stun
    d(354, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // voltageDischarge
    d(355, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // voltageBackup
    FxDesc { shake: 1.5,
             ..d(344, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip) },
    d(343, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // reversal + white glow
    FxDesc { shake: 1.5,
             ..d(324, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip) },  // berserk
    d(325, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // revenge
    FxDesc { rotate: Rot::HeroMinus180, scale: 1.8,
             ..d(333, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip) },
    d(334, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // anti-mine
    d(290, Res::PatternOr("invincibilityShieldPvp"), "mc", Layer::Fx, Anim::Loop, Kind::Clip),
    d(310, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // killstreak_cloud
    d(312, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // killstreak_trail
    d(338, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),
    d(339, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),
    FxDesc { scale_a: 150.0,
             ..d(348, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip) },
    d(320, Res::Fixed("ui"), "redirect_shield", Layer::Fx, Anim::Loop, Kind::Clip),
    d(321, Res::Fixed("ui"), "shieldDisarray", Layer::Fx, Anim::Loop, Kind::Clip),
    d(322, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),   // ddol
    // --- `ChainBoltEffect2D` lightning links (no clip — procedural
    // bolt, `a` = the chain target's entity id) ------------------------
    FxDesc { bolt: Some(Bolt { color: 16200448, steps: 30, thickness: 2.5, amplitude: 0.5 }),
             ..d(303, Res::Fixed(""), "", Layer::Fx, Anim::Loop, Kind::ChainLink) },
    FxDesc { bolt: Some(Bolt { color: 5084598, steps: 100, thickness: 3.5, amplitude: 0.05 }),
             ..d(323, Res::Fixed(""), "", Layer::Fx, Anim::Loop, Kind::ChainLink) },
    FxDesc { bolt: Some(Bolt { color: 8183666, steps: 50, thickness: 2.0, amplitude: 0.1 }),
             ..d(326, Res::Fixed(""), "", Layer::Fx, Anim::Loop, Kind::ChainLink) },
    FxDesc { bolt: Some(Bolt { color: 10732, steps: 50, thickness: 2.0, amplitude: 0.1 }),
             ..d(352, Res::Fixed(""), "", Layer::Fx, Anim::Loop, Kind::ChainLink) },
    FxDesc { bolt: Some(Bolt { color: 5084598, steps: 30, thickness: 2.5, amplitude: 0.5 }),
             ..d(412, Res::Fixed(""), "", Layer::Fx, Anim::Loop, Kind::ChainLink) },
    // `TargetEffect2D` — lock-on brackets in `ui`.
    d(80,  Res::Fixed("ui"), "markTarget", Layer::Fx, Anim::Loop, Kind::Clip),
    d(416, Res::Fixed("escape_zone"), "mc", Layer::Fx, Anim::Loop, Kind::Clip),
    d(420, Res::Fixed("cannon_skill_corrosive"), "mc", Layer::Fx, Anim::Loop, Kind::Clip),
    FxDesc { scale: 2.0,
             ..d(409, Res::Fixed("bay_emp_shock"), "mc", Layer::Fx, Anim::Loop, Kind::Clip) },
    d(408, Res::Fixed("boss_bay_activate"), "mc", Layer::Fx, Anim::Loop, Kind::Clip),
    // BossShield: `effectMovieClipName` =
    // `bossShieldPatterns[level].resKey` ("level1".."level4"), plays
    // once to the last frame, scale 4 on level 4 else 2.
    FxDesc { symbol: "level", scale: 2.0,
             ..d(414, Res::Fixed("boss_shield"), "level", Layer::Fx, Anim::Once, Kind::Clip) },
    // CHSP/SPCP buffs — `BuffEffect2D`/`LastPersistantEffect2D`.
    FxDesc { grow: Some((0.0, 1.0, 10.0)),
             ..d(2001, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip) },
    FxDesc { grow: Some((0.0, 1.0, 10.0)),
             ..d(3001, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip) },
    d(2003, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),
    d(3003, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),
    // Drive-area rings (`DriveAreaEffect2D`) + start/end one-shots
    // (`DriveShieldEffect2D` — `(true,true,false)` = fx layer).
    d(151, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),
    d(152, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip),
    d(1510, Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip),
    d(1511, Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip),
    d(1520, Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip),
    d(1521, Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip),
    d(1530, Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip),
    d(1531, Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip),

    // --- one-shot clips ----------------------------------------------
    FxDesc { offset: (0.0, -50.0),
             ..d(13, Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip) },   // levelUp
    d(19,  Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip),               // spawn0
    d(96,  Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip),               // hologram
    d(71,  Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip),               // lordakium spawn
    // `ShipWarpEffect2D` is the one view parented *below* the hull
    // (`addChildAt(view, 0)`).
    FxDesc { rate: 0.8,
             ..d(41, Res::Pattern, "mc", Layer::BelowHull, Anim::Once, Kind::Clip) }, // shipWarp
    d(46,  Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip),        // healEffect
    d(72,  Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip),        // lordakium heal
    d(35,  Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip),        // fortify
    d(95,  Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip),        // phaseOut
    // `InstaShieldEffect2D` — `(true,true,true)` = child above hull.
    FxDesc { rotate: Rot::Random,
             ..d(84, Res::PatternOr("instaShield"), "mc", Layer::AboveHull, Anim::Once, Kind::Clip) },
    FxDesc { grow: Some((0.3, 1.0, 1.8)),
             ..d(67, Res::Pattern, "blitzUltimate", Layer::Fx, Anim::Once, Kind::Clip) },
    FxDesc { grow: Some((0.3, 1.0, 2.5)),
             ..d(68, Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip) },
    FxDesc { scale: 1.5, offset: (0.0, -10.0), rate: 24.0 / 30.0,
             ..d(313, Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip) },
    FxDesc { scale: 1.5, offset: (0.0, -10.0), rate: 24.0 / 30.0,
             ..d(340, Res::Pattern, "mc", Layer::Fx, Anim::Once, Kind::Clip) },
    FxDesc { rate: 60.0 / 30.0,
             ..d(404, Res::Fixed("lastStand"), "mc", Layer::Fx, Anim::Once, Kind::Clip) },
    FxDesc { rate: 25.0 / 30.0,
             ..d(405, Res::Fixed("ripperEffect"), "mc", Layer::Fx, Anim::Once, Kind::Clip) },
    FxDesc { rate: 10.0 / 30.0,
             ..d(406, Res::Fixed("boss_laser_charge"), "mc", Layer::Fx, Anim::Once, Kind::Clip) },
    FxDesc { rate: 25.0 / 30.0, grow: Some((2.0, -1.0, 2.0)),
             ..d(407, Res::Fixed("boss_emp_shock"), "mc", Layer::Fx, Anim::Once, Kind::Clip) },
    FxDesc { rate: 15.0 / 30.0,
             ..d(413, Res::Fixed("boss_escape"), "mc", Layer::Fx, Anim::Once, Kind::Clip) },

    // --- glow filters -------------------------------------------------
    // `ShipGlowEffect2D`: GlowFilter on the ship view, pulsing while the
    // model carries `pulse`. `a` supplies the color when non-zero.
    FxDesc { glow: 16711680, ..d(17,  Res::Pattern, "", Layer::Fx, Anim::Loop, Kind::Glow) },
    FxDesc { glow: 65535,    ..d(29,  Res::Pattern, "", Layer::Fx, Anim::Loop, Kind::Glow) },
    FxDesc { glow: 0,        ..d(52,  Res::Pattern, "", Layer::Fx, Anim::Loop, Kind::Glow) },
    FxDesc { glow: 16758049, ..d(287, Res::Pattern, "", Layer::Fx, Anim::Loop, Kind::Glow) },
    FxDesc { glow: 0,        ..d(289, Res::Pattern, "", Layer::Fx, Anim::Loop, Kind::Glow) },
    FxDesc { glow: 0,        ..d(335, Res::Pattern, "", Layer::Fx, Anim::Loop, Kind::Glow) },
    FxDesc { glow: 15903743, ..d(356, Res::Pattern, "", Layer::Fx, Anim::Loop, Kind::Glow) },

    // --- special behaviours -------------------------------------------
    d(331, Res::Pattern, "", Layer::Fx, Anim::Loop, Kind::Ring),
    d(58,  Res::Pattern, "", Layer::Fx, Anim::Once, Kind::Ring),
    d(64,  Res::Fixed("shockwaves"), "shockring0", Layer::Fx, Anim::Loop, Kind::Shockring),
    d(56,  Res::Pattern, "", Layer::Fx, Anim::Loop, Kind::StickyBomb),
    d(305, Res::Fixed("explosiveChargingBlob"), "", Layer::Fx, Anim::Loop, Kind::StickyBomb),
    // `SphereShieldEffect2D` — `(true,true,true)` = child above hull.
    d(99,  Res::Fixed("shield0"), "mc", Layer::AboveHull, Anim::Loop, Kind::SphereShield),
    d(12,  Res::Pattern, "", Layer::Fx, Anim::Loop, Kind::RepairRobots),
    // `REP_BOT_EFFECT` — the `BeaconCommand` repair bot (effect 76 has
    // no `<effects>` row; `robot_patterns` supplies the res).
    d(76,  Res::Pattern, "", Layer::Fx, Anim::Loop, Kind::RepairRobots),
    d(302, Res::Pattern, "", Layer::Fx, Anim::Loop, Kind::RepairRobots),
    d(14,  Res::Fixed("speed-buff-effect"), "mc", Layer::Fx, Anim::Loop, Kind::Turbo),
    d(92,  Res::Fixed("speed-buff-effect"), "mc", Layer::Fx, Anim::Loop, Kind::Turbo),
    FxDesc { hull_alpha: 0.4, ..d(51, Res::Pattern, "", Layer::Fx, Anim::Loop, Kind::HullAlpha) },
    FxDesc { hull_extra: 0.4,
             ..d(85, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::HullFitClip) },
    FxDesc { hull_extra: 0.4,
             ..d(330, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::HullFitClip) },
    d(288, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::HullFitClip),
    FxDesc { grow: Some((0.0, 1.0, 30.0)),
             ..d(332, Res::Pattern, "mc", Layer::Fx, Anim::Loop, Kind::Clip) },
];

/// Descriptor for an `EffectIDList` id — `None` when no ported view
/// exists (the effect is then tracked but draws nothing).
pub fn lookup(id: i32) -> Option<&'static FxDesc> {
    CATALOG.iter().find(|d| d.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_effects_have_descriptors() {
        for id in [
            16, 84, 13, 19, 14, 92, 56, 99, 331, 64, 38, 41, 45, 47, 35, 80, 405, 414, 420, 2001,
        ] {
            assert!(lookup(id).is_some(), "missing catalog row for {id}");
        }
    }

    #[test]
    fn hardcoded_resources() {
        assert_eq!(lookup(99).unwrap().res, Res::Fixed("shield0"));
        assert_eq!(lookup(14).unwrap().res, Res::Fixed("speed-buff-effect"));
        assert_eq!(lookup(414).unwrap().res, Res::Fixed("boss_shield"));
        assert_eq!(lookup(16).unwrap().res, Res::PatternOr("invincibilityShield"));
        assert_eq!(lookup(38).unwrap().res, Res::Fixed("shockwaves"));
        assert_eq!(lookup(38).unwrap().symbol, "protectionBuffRing");
        assert_eq!(lookup(39).unwrap().symbol, "drawFireVortex");
    }

    #[test]
    fn layers_and_anim_modes() {
        assert_eq!(lookup(13).unwrap().layer, Layer::Fx);
        assert_eq!(lookup(13).unwrap().anim, Anim::Once);
        // The four hull-children: ship-warp below; deflector/sphere/
        // insta-shield above (`AbstractMapObjectEffect2D` param5=true).
        assert_eq!(lookup(41).unwrap().layer, Layer::BelowHull);
        assert_eq!(lookup(45).unwrap().layer, Layer::AboveHull);
        assert_eq!(lookup(99).unwrap().layer, Layer::AboveHull);
        assert_eq!(lookup(84).unwrap().layer, Layer::AboveHull);
        assert_eq!(lookup(18).unwrap().layer, Layer::Fx);
        assert_eq!(lookup(84).unwrap().anim, Anim::Once);
    }

    #[test]
    fn special_kinds() {
        assert_eq!(lookup(17).unwrap().kind, Kind::Glow);
        assert_eq!(lookup(56).unwrap().kind, Kind::StickyBomb);
        assert_eq!(lookup(99).unwrap().kind, Kind::SphereShield);
        assert_eq!(lookup(331).unwrap().kind, Kind::Ring);
        assert_eq!(lookup(51).unwrap().kind, Kind::HullAlpha);
    }
}
