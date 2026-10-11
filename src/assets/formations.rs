//! Drone formation tables (`game.xml` `<drones><formations><formation>`).
//!
//! `PatternManager.parseDroneFormationPatterns` builds a
//! `DroneFormation(locations, locationsByDroneCount, scale, id, name)`
//! per `<formation>`: `positionsList@data` is a `;`-separated list of
//! `x,y[,z]` ship-local offsets and each `<usedPositions droneCount=N
//! positionIDlist="a,b,c">` picks which of those (1-based) the N drones
//! take, by drone index. `DroneFormation.positionToRelativePoint(count,
//! index)` = `locations[usedPositions[count][index] - 1]`.

use roxmltree::Node;
use std::f32::consts::TAU;

use super::particles::{euler_quat, quat_axis, quat_mul, quat_rotate};

/// One parsed `<formation>`.
#[derive(Clone, Debug, Default)]
pub struct DroneFormationPattern {
    /// `id` (0..14, 42).
    pub id: i32,
    /// `name` (`standard`, `turtle`, ...).
    pub name: String,
    /// `<scale>` — only the 3D view uses it; kept for completeness.
    pub scale: f32,
    /// `positionsList@data` — ship-local `(x, y)` offsets (z dropped).
    pub locations: Vec<(f32, f32)>,
    /// `locations_by_count[droneCount]` — 1-based location ids; `None`
    /// where the table has no `usedPositions` row for that count.
    pub locations_by_count: Vec<Option<Vec<i32>>>,
    /// The `<display3D>` child (`standardFormations3D`) — richer 3D
    /// positions and rotation groups; absent for the plain 2D formations.
    pub display3d: Option<Formation3D>,
}

impl DroneFormationPattern {
    /// `positionToRelativePoint(count, index)` — the ship-local offset of
    /// drone `index` in a loadout of `count` drones, `None` when the
    /// table has no row/entry (the stock client would throw there).
    pub fn position(&self, count: usize, index: usize) -> Option<(f32, f32)> {
        let ids = self.locations_by_count.get(count)?.as_ref()?;
        let loc = *ids.get(index)?;
        if loc < 1 {
            return None;
        }
        self.locations.get(loc as usize - 1).copied()
    }

    /// `DronesFormation3D.updateObj`: the drone's ship-local Away3D target
    /// `(x, y, z)` at time `t` — the authored `(x, y, z)` slot mapped to
    /// `(x, -z, -y)`, pushed through its spinning `rotationGroup` container,
    /// then `userScale · formationYaw · groupBaseEuler(Z,X,Y)` applied (the
    /// appended order of the shared `mtx`). `formation_yaw_deg` is the
    /// eased formation rotation (`_formationRotation`).
    pub fn target3d(
        &self,
        count: usize,
        index: usize,
        user_scale: f32,
        formation_yaw_deg: f32,
        t: f32,
    ) -> [f32; 3] {
        let (x, y, z) = self
            .display3d
            .as_ref()
            .and_then(|d| d.position(count, index))
            .or_else(|| self.position(count, index).map(|(x, y)| (x, y, 0.0)))
            .unwrap_or_else(|| {
                let (x, y) = fallback_ring_position(count, index);
                (x, y, 0.0)
            });
        let mut v = [x, -z, -y];
        let group = self.display3d.as_ref().and_then(|d| d.group_for(index));
        let q_yaw = quat_axis(&[0.0, 1.0, 0.0], formation_yaw_deg.to_radians());
        let q = if let Some(g) = group {
            // The animated container transform applies first
            // (`rotationParent.transform.transformVectors`), then
            // scale → yaw → the group's base euler appended Rz, Rx, Ry.
            let (euler, offset, scale) = g.eval(t);
            let spun = quat_rotate(&euler_quat(&euler), &v);
            for k in 0..3 {
                v[k] = spun[k] * scale + offset[k];
            }
            quat_mul(
                &quat_axis(&[0.0, 1.0, 0.0], g.base_euler.1.to_radians()),
                &quat_mul(
                    &quat_axis(&[1.0, 0.0, 0.0], g.base_euler.0.to_radians()),
                    &quat_mul(
                        &quat_axis(&[0.0, 0.0, 1.0], g.base_euler.2.to_radians()),
                        &q_yaw,
                    ),
                ),
            )
        } else {
            q_yaw
        };
        quat_rotate(
            &q,
            &[v[0] * user_scale, v[1] * user_scale, v[2] * user_scale],
        )
    }

    /// Parse one `<formation>` node. `display2D` child (when present)
    /// carries the tables, else the node itself does.
    pub(crate) fn parse(node: &Node) -> Self {
        let tables = node
            .children()
            .find(|c| c.is_element() && c.has_tag_name("display2D"))
            .unwrap_or(*node);
        let child = |tag: &str| {
            tables
                .children()
                .find(|c| c.is_element() && c.has_tag_name(tag))
        };
        let locations = child("positionsList")
            .and_then(|p| p.attribute("data"))
            .map(parse_positions)
            .unwrap_or_default();
        let mut by_count: Vec<Option<Vec<i32>>> = Vec::new();
        parse_used(tables, &mut by_count);
        let display3d = node
            .children()
            .find(|c| c.is_element() && c.has_tag_name("display3D"))
            .map(|d| Formation3D::parse(&d));
        Self {
            id: node
                .attribute("id")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0),
            name: node.attribute("name").unwrap_or("").to_owned(),
            scale: child("scale")
                .and_then(|s| s.text())
                .and_then(|t| t.trim().parse().ok())
                .unwrap_or(1.0),
            locations,
            locations_by_count: by_count,
            display3d,
        }
    }
}

/// A formation's authored 3D layout — the `<display3D>` child
/// (`PatternManager.parseDroneFormationPatterns3D` /
/// `standardFormations3D`). Its `positionsList` may carry a third
/// component (height), and `<rotationGroup droneIds>` nodes attach
/// spinning/tilting `ObjectContainer3D` parents to subsets of the drones.
#[derive(Clone, Debug, Default)]
pub struct Formation3D {
    /// `<scale>` — the 3D view's formation spread (`updateObj` uses the
    /// server-sent `userScale` instead; kept for completeness).
    pub scale: f32,
    /// `positionsList@data` — authored `(x, y, z)` ship-local offsets.
    pub locations: Vec<(f32, f32, f32)>,
    /// `locations_by_count[droneCount]` — same shape as the 2D table.
    pub locations_by_count: Vec<Option<Vec<i32>>>,
    pub rotation_groups: Vec<RotationGroup3D>,
}

impl Formation3D {
    /// `positionToRelativePoint(count, index)` on the 3D table.
    pub fn position(&self, count: usize, index: usize) -> Option<(f32, f32, f32)> {
        let ids = self.locations_by_count.get(count)?.as_ref()?;
        let loc = *ids.get(index)?;
        if loc < 1 {
            return None;
        }
        self.locations.get(loc as usize - 1).copied()
    }

    /// The `rotationGroup` containing drone `index` (0-based loadout slot;
    /// `droneIds` are 1-based: `drones[id - 1]`).
    pub fn group_for(&self, index: usize) -> Option<&RotationGroup3D> {
        self.rotation_groups
            .iter()
            .find(|g| g.drone_ids.contains(&(index as i32 + 1)))
    }

    fn parse(node: &Node) -> Self {
        let child = |tag: &str| {
            node.children()
                .find(|c| c.is_element() && c.has_tag_name(tag))
        };
        let locations = child("positionsList")
            .and_then(|p| p.attribute("data"))
            .map(parse_positions3)
            .unwrap_or_default();
        let mut by_count: Vec<Option<Vec<i32>>> = Vec::new();
        parse_used(*node, &mut by_count);
        let f = |n: &Node, attr: &str| {
            n.attribute(attr)
                .and_then(|v| v.trim().parse::<f32>().ok())
                .unwrap_or(0.0)
        };
        let rotation_groups = node
            .children()
            .filter(|c| c.is_element() && c.has_tag_name("rotationGroup"))
            .map(|g| RotationGroup3D {
                drone_ids: g
                    .attribute("droneIds")
                    .unwrap_or("")
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect(),
                base_euler: (f(&g, "rotationX"), f(&g, "rotationY"), f(&g, "rotationZ")),
                tweens: g
                    .children()
                    .filter(|c| c.is_element() && c.has_tag_name("tween"))
                    .map(|tw| GroupTween {
                        duration: f(&tw, "duration").max(1e-5),
                        delay: f(&tw, "delay"),
                        rotate: (
                            f(&tw, "rotationX"),
                            f(&tw, "rotationY"),
                            f(&tw, "rotationZ"),
                        ),
                        offset: (f(&tw, "x"), f(&tw, "y"), f(&tw, "z")),
                        scale: tw
                            .attribute("scale")
                            .and_then(|v| v.trim().parse::<f32>().ok()),
                        repeat: tw
                            .attribute("repeat")
                            .and_then(|v| v.trim().parse::<i32>().ok())
                            .unwrap_or(0),
                        yoyo: tw.attribute("yoyo").is_some_and(|v| v == "true"),
                        ease: Ease::parse(tw.attribute("ease").unwrap_or("")),
                    })
                    .collect(),
            })
            .collect();
        Self {
            scale: child("scale")
                .and_then(|s| s.text())
                .and_then(|t| t.trim().parse().ok())
                .unwrap_or(1.0),
            locations,
            locations_by_count: by_count,
            rotation_groups,
        }
    }
}

/// One `<rotationGroup droneIds rotationX/Y/Z>` — the container the
/// member drones' formation points pass through (`transformVectors`)
/// before scale, yaw and the group's base euler are applied.
#[derive(Clone, Debug, Default)]
pub struct RotationGroup3D {
    /// 1-based drone indices (`_loc6_ - 1` indexes the drone vector).
    pub drone_ids: Vec<i32>,
    /// `rotationGroup@rotationX/Y/Z` — static base euler, degrees.
    pub base_euler: (f32, f32, f32),
    /// Each `<tween>` is a self-contained `TweenMax` on the container —
    /// `repeat`/`yoyo`/`ease`/`delay` ride the attribute list into props.
    pub tweens: Vec<GroupTween>,
}

impl RotationGroup3D {
    /// The container's animated euler `(x, y, z)` degrees, offset and
    /// scale at time `t` — every `<tween>` contributes its eased share.
    pub fn eval(&self, t: f32) -> ([f32; 3], [f32; 3], f32) {
        let mut euler = [0.0; 3];
        let mut offset = [0.0; 3];
        let mut scale = 1.0;
        for tw in &self.tweens {
            let p = tw.phase(t);
            for k in 0..3 {
                euler[k] += [tw.rotate.0, tw.rotate.1, tw.rotate.2][k] * p;
                offset[k] += [tw.offset.0, tw.offset.1, tw.offset.2][k] * p;
            }
            if let Some(s) = tw.scale {
                scale *= 1.0 + (s - 1.0) * p;
            }
        }
        (euler, offset, scale)
    }
}

/// One `<tween>` — the props of a `TweenMax` on the group container:
/// `duration`, `delay`, any of `rotationX/Y/Z`, `x/y/z`, `scale`,
/// `repeat`, `yoyo`, `ease` (absolute targets eased from 0/current).
#[derive(Clone, Debug)]
pub struct GroupTween {
    pub duration: f32,
    pub delay: f32,
    /// `rotationX/Y/Z` targets, degrees.
    pub rotate: (f32, f32, f32),
    /// `x/y/z` position targets.
    pub offset: (f32, f32, f32),
    /// `scale` → container scaleX/Y/Z.
    pub scale: Option<f32>,
    /// `repeat` — `-1` forever, else `repeat + 1` plays.
    pub repeat: i32,
    pub yoyo: bool,
    pub ease: Ease,
}

impl GroupTween {
    /// Eased progress at time `t`: `delay` first, then `ease(t mod
    /// duration)` per cycle; `yoyo` reverses alternate cycles and a
    /// finite `repeat` parks at the last reached value.
    pub fn phase(&self, t: f32) -> f32 {
        let t = t - self.delay;
        if t < 0.0 {
            return 0.0;
        }
        let cycle = (t / self.duration).floor() as i64;
        if self.repeat >= 0 && cycle > self.repeat as i64 {
            return if self.yoyo && self.repeat % 2 == 1 {
                0.0
            } else {
                1.0
            };
        }
        let p = (t / self.duration).fract();
        let p = if self.yoyo && cycle % 2 == 1 {
            1.0 - p
        } else {
            p
        };
        self.ease.eval(p)
    }
}

/// `ease` attribute names (`EaseLookup.find`) — `Quad.easeOut`,
/// `Sine.easeInOut`, `Linear.easeNone`, ... Unknown names fall back to
/// linear, matching TweenMax's default-ease behaviour for bad lookups.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Ease {
    #[default]
    Linear,
    QuadIn,
    QuadOut,
    QuadInOut,
    SineIn,
    SineOut,
    SineInOut,
}

impl Ease {
    fn parse(name: &str) -> Self {
        match name {
            "Quad.easeIn" => Self::QuadIn,
            "Quad.easeOut" => Self::QuadOut,
            "Quad.easeInOut" => Self::QuadInOut,
            "Sine.easeIn" => Self::SineIn,
            "Sine.easeOut" => Self::SineOut,
            "Sine.easeInOut" => Self::SineInOut,
            _ => Self::Linear,
        }
    }

    /// `Ease.getRatio(p)` — the eased fraction of the target reached at
    /// progress `p` of a cycle.
    pub fn eval(&self, p: f32) -> f32 {
        let p = p.clamp(0.0, 1.0);
        match self {
            Self::Linear => p,
            Self::QuadIn => p * p,
            Self::QuadOut => p * (2.0 - p),
            Self::QuadInOut => {
                if p < 0.5 {
                    2.0 * p * p
                } else {
                    p * (4.0 - 2.0 * p) - 1.0
                }
            }
            Self::SineIn => 1.0 - (p * std::f32::consts::FRAC_PI_2).cos(),
            Self::SineOut => (p * std::f32::consts::FRAC_PI_2).sin(),
            Self::SineInOut => -0.5 * ((p * std::f32::consts::PI).cos() - 1.0),
        }
    }
}

fn parse_used(tables: Node, by_count: &mut Vec<Option<Vec<i32>>>) {
    for used in tables
        .children()
        .filter(|c| c.is_element() && c.has_tag_name("usedPositions"))
    {
        let count = used
            .attribute("droneCount")
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let ids: Vec<i32> = used
            .attribute("positionIDlist")
            .unwrap_or("")
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        if by_count.len() <= count {
            by_count.resize(count + 1, None);
        }
        by_count[count] = Some(ids);
    }
}

/// `x,y,z;x,y,z;...` -> `(x, y, z)` triples (the 3D table's third
/// component is the height; `int()` in stock, floats kept as authored).
fn parse_positions3(data: &str) -> Vec<(f32, f32, f32)> {
    data.split(';')
        .filter(|s| !s.trim().is_empty())
        .map(|pt| {
            let mut it = pt
                .split(',')
                .map(|v| v.trim().parse::<f32>().unwrap_or(0.0));
            (
                it.next().unwrap_or(0.0),
                it.next().unwrap_or(0.0),
                it.next().unwrap_or(0.0),
            )
        })
        .collect()
}

/// `x,y[,z];x,y[,z];...` -> `(x, y)` pairs (`int()` in the stock parser,
/// but the shipped data carries `187.5` etc., so floats are kept).
fn parse_positions(data: &str) -> Vec<(f32, f32)> {
    data.split(';')
        .filter(|s| !s.trim().is_empty())
        .map(|pt| {
            let mut it = pt
                .split(',')
                .map(|v| v.trim().parse::<f32>().unwrap_or(0.0));
            (it.next().unwrap_or(0.0), it.next().unwrap_or(0.0))
        })
        .collect()
}

/// Radius of the fallback ring, world units (the STANDARD table's inner
/// ring sits at 150).
pub const FALLBACK_RING_RADIUS: f32 = 150.0;

/// Ship-local offset of drone `index` of `count` on an even ring — the
/// fallback when a formation id has no table row (or the table lacks a
/// row for this drone count). Drone 0 sits behind the ship (+x, the
/// STANDARD table's "back group" side) and the rest follow clockwise.
pub fn fallback_ring_position(count: usize, index: usize) -> (f32, f32) {
    if count == 0 {
        return (0.0, 0.0);
    }
    let a = TAU * index as f32 / count as f32;
    (
        FALLBACK_RING_RADIUS * a.cos(),
        FALLBACK_RING_RADIUS * a.sin(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formation(xml: &str) -> DroneFormationPattern {
        let doc = roxmltree::Document::parse(xml).unwrap();
        DroneFormationPattern::parse(&doc.root_element())
    }

    #[test]
    fn parses_positions_and_used_rows() {
        let f = formation(
            r#"<formation id="0" name="standard">
                 <positionsList data="150,0;225,0;187.5,37.5;187.5,-37.5"/>
                 <scale>1.5</scale>
                 <usedPositions droneCount="0" positionIDlist=""/>
                 <usedPositions droneCount="2" positionIDlist="3,4"/>
               </formation>"#,
        );
        assert_eq!(f.id, 0);
        assert_eq!(f.name, "standard");
        assert_eq!(f.scale, 1.5);
        assert_eq!(f.locations.len(), 4);
        assert_eq!(f.position(2, 0), Some((187.5, 37.5)));
        assert_eq!(f.position(2, 1), Some((187.5, -37.5)));
        // No row for 1 drone; out-of-range index; empty row.
        assert_eq!(f.position(1, 0), None);
        assert_eq!(f.position(2, 2), None);
        assert_eq!(f.position(0, 0), None);
    }

    #[test]
    fn display2d_wrapper_is_preferred() {
        let f = formation(
            r#"<formation id="7" name="diamond">
                 <display2D>
                   <positionsList data="1,2,3;4,5"/>
                   <usedPositions droneCount="2" positionIDlist="2,1"/>
                 </display2D>
               </formation>"#,
        );
        assert_eq!(f.position(2, 0), Some((4.0, 5.0)));
        assert_eq!(f.position(2, 1), Some((1.0, 2.0)));
    }

    #[test]
    fn fallback_ring_is_even() {
        let (x, y) = fallback_ring_position(4, 0);
        assert!((x - FALLBACK_RING_RADIUS).abs() < 1e-3 && y.abs() < 1e-3);
        let (x, y) = fallback_ring_position(4, 1);
        assert!(x.abs() < 1e-3 && (y - FALLBACK_RING_RADIUS).abs() < 1e-3);
        let (x, _) = fallback_ring_position(4, 2);
        assert!((x + FALLBACK_RING_RADIUS).abs() < 1e-3);
        assert_eq!(fallback_ring_position(0, 0), (0.0, 0.0));
    }

    fn near3(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-3)
    }

    /// The real `formations3D` shape: `<display3D>` with 3-component
    /// positions plus `<rotationGroup>` tweens.
    #[test]
    fn parses_display3d_tables_and_rotation_groups() {
        let f = formation(
            r#"<formation id="16" name="Veteran 3D">
                 <display3D>
                   <positionsList data="0,150;88.2,121.4,30"/>
                   <scale>1.0</scale>
                   <usedPositions droneCount="2" positionIDlist="1,2"/>
                   <rotationGroup rotationY="45" droneIds="1,3">
                     <tween duration="5" delay="0.5" rotationZ="360" yoyo="false" ease="Linear.easeNone" repeat="-1"/>
                   </rotationGroup>
                   <rotationGroup droneIds="2">
                     <tween duration="1.5" y="-30" repeat="-1" yoyo="true" ease="Sine.easeInOut"/>
                   </rotationGroup>
                 </display3D>
                 <display2D><positionsList data="1,2"/></display2D>
               </formation>"#,
        );
        let d = f.display3d.as_ref().expect("display3D parsed");
        assert_eq!(d.position(2, 0), Some((0.0, 150.0, 0.0)));
        assert_eq!(d.position(2, 1), Some((88.2, 121.4, 30.0)));
        assert_eq!(d.rotation_groups.len(), 2);
        // `droneIds` are 1-based: drone index 0 and 2 in group 0.
        assert!(d.group_for(0).is_some_and(|g| g.base_euler.1 == 45.0));
        assert!(d
            .group_for(1)
            .is_some_and(|g| g.tweens[0].offset.1 == -30.0));
        assert!(d.group_for(2).is_some_and(|g| g.base_euler.1 == 45.0));
        let g0 = &d.rotation_groups[0];
        assert_eq!(g0.tweens[0].duration, 5.0);
        assert_eq!(g0.tweens[0].delay, 0.5);
        assert_eq!(g0.tweens[0].rotate.2, 360.0);
        assert_eq!(g0.tweens[0].repeat, -1);
        assert!(!g0.tweens[0].yoyo);
        assert_eq!(g0.tweens[0].ease, Ease::Linear);
        let g1 = &d.rotation_groups[1].tweens[0];
        assert!(g1.yoyo && g1.ease == Ease::SineInOut);
    }

    #[test]
    fn tween_phase_eases_repeats_and_yoyos() {
        let lin = GroupTween {
            duration: 5.0,
            delay: 0.5,
            rotate: (360.0, 0.0, 0.0),
            offset: (0.0, 0.0, 0.0),
            scale: None,
            repeat: -1,
            yoyo: false,
            ease: Ease::Linear,
        };
        // Before the delay: nothing; after: a 1.25 s quarter at t = 1.75.
        assert_eq!(lin.phase(0.25), 0.0);
        assert!((lin.phase(0.5 + 1.25) - 0.25).abs() < 1e-6);
        // Infinite linear spin is periodic.
        assert!((lin.phase(5.5 + 2.5) - 0.5).abs() < 1e-6);

        let mut yoyo = lin.clone();
        yoyo.yoyo = true;
        yoyo.duration = 1.5;
        yoyo.delay = 0.0;
        // Second cycle (1.5..3 s) runs back: progress counts down.
        assert!((yoyo.phase(0.75) - 0.5).abs() < 1e-6);
        assert!((yoyo.phase(1.5 + 0.375) - 0.75).abs() < 1e-6);
        // Third cycle goes forward again.
        assert!((yoyo.phase(3.0 + 0.375) - 0.25).abs() < 1e-6);

        // Finite repeat parks at the final value.
        let mut once = lin.clone();
        once.repeat = 0;
        once.delay = 0.0;
        assert_eq!(once.phase(99.0), 1.0);
        let mut twice = lin.clone();
        twice.repeat = 1;
        twice.delay = 0.0;
        assert_eq!(twice.phase(99.0), 1.0);
    }

    /// `RotationGroup3D.eval` sums every tween — the two drills'
    /// counter-rotating X spins and the wavy bob both land.
    #[test]
    fn rotation_group_eval_sums_tweens() {
        let g = RotationGroup3D {
            drone_ids: vec![1],
            base_euler: (0.0, 0.0, 0.0),
            tweens: vec![
                GroupTween {
                    duration: 5.0,
                    delay: 0.0,
                    rotate: (360.0, -360.0, 0.0),
                    offset: (0.0, 0.0, 0.0),
                    scale: None,
                    repeat: -1,
                    yoyo: false,
                    ease: Ease::Linear,
                },
                GroupTween {
                    duration: 2.0,
                    delay: 0.0,
                    rotate: (0.0, 0.0, 0.0),
                    offset: (0.0, -30.0, 0.0),
                    scale: Some(2.0),
                    repeat: -1,
                    yoyo: true,
                    ease: Ease::Linear,
                },
            ],
        };
        let (euler, offset, scale) = g.eval(1.0);
        // A fifth into the 5 s spin: 72° on both axes.
        assert!((euler[0] - 72.0).abs() < 1e-3);
        assert!((euler[1] + 72.0).abs() < 1e-3);
        // Half-way through the yoyo's first forward cycle: -15 y, 1.5×.
        assert!((offset[1] + 15.0).abs() < 1e-3);
        assert!((scale - 1.5).abs() < 1e-3);
    }

    /// `target3d` maps the authored slot to Away3D `(x, -z, -y)`, spins
    /// it through the group container and tilts by the base euler.
    #[test]
    fn target3d_maps_spins_and_tilts() {
        let f = formation(
            r#"<formation id="14" name="ring">
                 <display3D>
                   <positionsList data="150,0,20"/>
                   <usedPositions droneCount="1" positionIDlist="1"/>
                   <rotationGroup rotationY="90" droneIds="1">
                     <tween duration="5" rotationX="360" yoyo="false" repeat="-1" ease="Linear.easeNone"/>
                   </rotationGroup>
                 </display3D>
                 <positionsList data="1,1"/>
               </formation>"#,
        );
        // t=0: no container spin; authored (150,0,20) -> Away (150,-20,0),
        // then the static base euler rotationY=90 — away-Y +90 sends
        // +x to -z and -y stays down.
        let p = f.target3d(1, 0, 1.0, 0.0, 0.0);
        assert!(near3(p, [0.0, -20.0, -150.0]), "{p:?}");
        // A quarter of the 5 s X-spin (away-X +90 sends -y to -z)
        // gives (150, 0, -20) inside the container; the base euler then
        // moves +x to -z and -z to -x -> (-20, 0, -150).
        let p = f.target3d(1, 0, 1.0, 0.0, 1.25);
        assert!(near3(p, [-20.0, 0.0, -150.0]), "{p:?}");
        // userScale multiplies the local slot before the appended euler.
        let p = f.target3d(1, 0, 2.0, 0.0, 0.0);
        assert!(near3(p, [0.0, -40.0, -300.0]), "{p:?}");
    }

    /// Without `<display3D>` the 3D target falls back to the 2D table
    /// (`standardFormations` path — `standardFormations3D` entry absent).
    #[test]
    fn target3d_without_display3d_uses_2d_slots() {
        let f = formation(
            r#"<formation id="0" name="standard">
                 <positionsList data="150,0"/>
                 <usedPositions droneCount="1" positionIDlist="1"/>
               </formation>"#,
        );
        assert!(f.display3d.is_none());
        // yaw 0 -> away (x, -z=0, -y=0); yaw 90 -> away (0, 0, -150)?
        // formation yaw rotates in the away plane: +x slot rotated +90°
        // about Y moves to +z (map -y — consistent with `rotate` in
        // game/drones.rs where the same 90° sends +x to +map-y... the
        // away z axis is mirrored).
        let p = f.target3d(1, 0, 1.0, 0.0, 0.0);
        assert!(near3(p, [150.0, 0.0, 0.0]), "{p:?}");
        let p = f.target3d(1, 0, 1.0, 90.0, 0.0);
        assert!(near3(p, [0.0, 0.0, -150.0]), "{p:?}");
    }
}
