//! Away3D particle groups (`.awp`, the particle editor's JSON) from the
//! separate `resources_3d_particles` collection.
//!
//! Sources: `away3d/loaders/parsers/ParticleGroupParser`, the
//! `particleSubParsers` value/node parsers, their `*Setter` random value
//! generators and the `animators/nodes/Particle*Node` vertex programs. The
//! GPU animation (Stage3D vertex shader) is evaluated on the CPU here: every
//! particle samples its random properties once, then [`ParticleAnim::eval`]
//! turns a time into one drawable quad/mesh transform.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::io::Read;

/// xorshift64* — particle properties only need cheap, reproducible noise.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn f(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32) / 16_777_216.0
    }
}

/// `OneD*ValueSubParser` setters.
#[derive(Clone, Debug)]
pub enum V1 {
    Const(f32),
    Random(f32, f32),
    /// `(x, y, type)` anchors; sampled at `index / total` (`OneDCurveSetter`).
    Curve(Vec<(f32, f32, i32)>),
}

impl V1 {
    fn parse(v: &Value) -> Option<V1> {
        let d = v.get("data")?;
        match v.get("id")?.as_str()? {
            "OneDConstValueSubParser" => Some(V1::Const(num(d, "value"))),
            "OneDRandomVauleSubParser" => Some(V1::Random(num(d, "min"), num(d, "max"))),
            "OneDCurveValueSubParser" => {
                let anchors = d
                    .get("anchorDatas")?
                    .as_array()?
                    .iter()
                    .map(|a| {
                        (
                            num(a, "x"),
                            num(a, "y"),
                            a.get("type").and_then(Value::as_i64).unwrap_or(0) as i32,
                        )
                    })
                    .collect();
                Some(V1::Curve(anchors))
            }
            _ => None,
        }
    }

    fn sample(&self, rng: &mut Rng, index: usize, total: usize) -> f32 {
        match self {
            V1::Const(v) => *v,
            V1::Random(min, max) => rng.f() * (max - min) + min,
            V1::Curve(anchors) => {
                let t = index as f32 / total.max(1) as f32;
                for i in 0..anchors.len().saturating_sub(1) {
                    if anchors[i + 1].0 <= t {
                        continue;
                    }
                    return match anchors[i].2 {
                        1 => anchors[i].1,
                        _ => {
                            anchors[i].1
                                + (t - anchors[i].0) / (anchors[i + 1].0 - anchors[i].0).max(1e-6)
                                    * (anchors[i + 1].1 - anchors[i].1)
                        }
                    };
                }
                anchors.last().map_or(0.0, |a| a.1)
            }
        }
    }

    /// Largest value the spec can produce (`Random` max, `Curve`'s
    /// highest anchor x/y… for the group-lifetime helpers the caller
    /// wants the upper bound, not a sample).
    pub fn upper(&self) -> f32 {
        match self {
            V1::Const(v) => *v,
            V1::Random(_, max) => *max,
            V1::Curve(anchors) => anchors.last().map_or(0.0, |a| a.1),
        }
    }
}

/// `ThreeD*ValueSubParser` setters.
#[derive(Clone, Debug)]
pub enum V3 {
    Const([f32; 3]),
    Composite {
        x: V1,
        y: V1,
        z: V1,
        iso: bool,
    },
    Sphere {
        inner: f32,
        outer: f32,
        center: [f32; 3],
    },
    Cylinder {
        inner: f32,
        outer: f32,
        height: f32,
        center: [f32; 3],
        dir: [f32; 3],
    },
}

impl V3 {
    fn parse(v: &Value) -> Option<V3> {
        let d = v.get("data")?;
        match v.get("id")?.as_str()? {
            "ThreeDConstValueSubParser" => Some(V3::Const([num(d, "x"), num(d, "y"), num(d, "z")])),
            "ThreeDCompositeValueSubParser" => Some(V3::Composite {
                x: V1::parse(d.get("x")?)?,
                y: V1::parse(d.get("y")?)?,
                z: V1::parse(d.get("z")?)?,
                iso: d.get("isometric").and_then(Value::as_bool).unwrap_or(false),
            }),
            "ThreeDSphereValueSubParser" => Some(V3::Sphere {
                inner: num(d, "innerRadius"),
                outer: num(d, "outerRadius"),
                center: [num(d, "centerX"), num(d, "centerY"), num(d, "centerZ")],
            }),
            "ThreeDCylinderValueSubParser" => Some(V3::Cylinder {
                inner: num(d, "innerRadius"),
                outer: num(d, "outerRadius"),
                height: num(d, "height"),
                center: [num(d, "centerX"), num(d, "centerY"), num(d, "centerZ")],
                dir: [num(d, "dX"), num(d, "dY"), num(d, "dZ")],
            }),
            _ => None,
        }
    }

    fn sample(&self, rng: &mut Rng, index: usize, total: usize) -> [f32; 3] {
        match self {
            V3::Const(v) => *v,
            V3::Composite { x, y, z, iso } => {
                let vx = x.sample(rng, index, total);
                if *iso {
                    [vx, vx, vx]
                } else {
                    [vx, y.sample(rng, index, total), z.sample(rng, index, total)]
                }
            }
            V3::Sphere {
                inner,
                outer,
                center,
            } => {
                let r = (rng.f() * (outer.powi(3) - inner.powi(3)) + inner.powi(3)).cbrt();
                let mut d = [rng.f() - 0.5, rng.f() - 0.5, rng.f() - 0.5];
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if len == 0.0 {
                    d = [1.0, 0.0, 0.0];
                } else {
                    d = [d[0] / len, d[1] / len, d[2] / len];
                }
                [
                    center[0] + d[0] * r,
                    center[1] + d[1] * r,
                    center[2] + d[2] * r,
                ]
            }
            V3::Cylinder {
                inner,
                outer,
                height,
                center,
                dir,
            } => {
                let h = rng.f() * height;
                let ratio = if *outer > 0.0 { inner / outer } else { 0.0 };
                let r = outer * (rng.f() * (1.0 - ratio) + ratio).sqrt();
                let a = rng.f() * std::f32::consts::TAU;
                let mut p = [r * a.cos(), h, r * a.sin()];
                let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
                if len > 0.0 {
                    let q = quat_between_y(&[dir[0] / len, dir[1] / len, dir[2] / len]);
                    p = quat_rotate(&q, &p);
                }
                [p[0] + center[0], p[1] + center[1], p[2] + center[2]]
            }
        }
    }
}

/// `FourDComposite*ValueSubParser`: `Vector3D(x, y, z, w)`.
#[derive(Clone, Debug)]
pub enum V4 {
    OneD([Option<V1>; 4]),
    ThreeD(V3, V1),
}

impl V4 {
    fn parse(v: &Value) -> Option<V4> {
        let d = v.get("data")?;
        match v.get("id")?.as_str()? {
            "FourDCompositeWithOneDValueSubParser" => {
                let c = |k: &str| d.get(k).and_then(V1::parse);
                Some(V4::OneD([c("x"), c("y"), c("z"), c("w")]))
            }
            "FourDCompositeWithThreeDValueSubParser" => {
                Some(V4::ThreeD(V3::parse(d.get("x")?)?, V1::parse(d.get("w")?)?))
            }
            _ => None,
        }
    }

    fn sample(&self, rng: &mut Rng, index: usize, total: usize) -> [f32; 4] {
        match self {
            V4::OneD(c) => {
                let mut out = [0.0; 4];
                for (o, v) in out.iter_mut().zip(c) {
                    *o = v.as_ref().map_or(0.0, |v| v.sample(rng, index, total));
                }
                out
            }
            V4::ThreeD(v3, w) => {
                let p = v3.sample(rng, index, total);
                [p[0], p[1], p[2], w.sample(rng, index, total)]
            }
        }
    }
}

/// `CompositeColorValueSubParser`: multiplier (0..1) and offset (0..255) per channel.
#[derive(Clone, Debug)]
pub struct ColorSpec {
    mul: [V1; 4],
    off: [V1; 4],
}

type Rgba = [f32; 4];

impl ColorSpec {
    fn parse(v: &Value) -> Option<ColorSpec> {
        let d = v.get("data")?;
        if d.get("mr").is_some() {
            let c = |k: &str| V1::Const(num(d, k));
            return Some(ColorSpec {
                mul: [c("mr"), c("mg"), c("mb"), c("ma")],
                off: [c("or"), c("og"), c("ob"), c("oa")],
            });
        }
        let pick =
            |k: &str, default: f32| d.get(k).and_then(V1::parse).unwrap_or(V1::Const(default));
        Some(ColorSpec {
            mul: [
                pick("redMultiplierValue", 1.0),
                pick("greenMultiplierValue", 1.0),
                pick("blueMultiplierValue", 1.0),
                pick("alphaMultiplierValue", 1.0),
            ],
            off: [
                pick("redOffsetValue", 0.0),
                pick("greenOffsetValue", 0.0),
                pick("blueOffsetValue", 0.0),
                pick("alphaOffsetValue", 0.0),
            ],
        })
    }

    fn sample(&self, rng: &mut Rng, index: usize, total: usize) -> (Rgba, Rgba) {
        let mut mul = [0.0; 4];
        let mut off = [0.0; 4];
        for i in 0..4 {
            mul[i] = self.mul[i].sample(rng, index, total);
            off[i] = self.off[i].sample(rng, index, total) / 255.0;
        }
        (mul, off)
    }
}

#[derive(Clone, Debug)]
pub enum Node {
    Time {
        start: V1,
        duration: Option<V1>,
        delay: Option<V1>,
        looping: bool,
        uses_delay: bool,
    },
    Scale {
        scale: V4,
        cycle: bool,
        phase: bool,
    },
    SegScale {
        start: V3,
        end: V3,
        points: Vec<(f32, V3)>,
    },
    Billboard,
    Velocity(V3),
    Accel(V3),
    Position(V3),
    RotVel(V4),
    Orbit {
        orbit: V4,
        eulers: Option<V3>,
        cycle: bool,
        phase: bool,
    },
    RotateToHeading,
    Uv {
        cycle: V1,
        scale: Option<V1>,
        axis: char,
        formula: i32,
    },
    Sheet {
        cols: u32,
        rows: u32,
        scale: V4,
        cycle: bool,
        phase: bool,
    },
    SegColor {
        start: ColorSpec,
        end: ColorSpec,
        points: Vec<(f32, ColorSpec)>,
    },
    InitialColor(ColorSpec),
    Color {
        start: ColorSpec,
        end: ColorSpec,
        cycle: bool,
        phase: bool,
    },
    Oscillator(V4),
    /// Follow / Bezier / unknown: no CPU effect.
    Ignored,
}

#[derive(Clone, Debug)]
pub enum Shape {
    Plane {
        w: f32,
        h: f32,
    },
    /// AWD geometry id in the particle collection (`fx_*.obj` -> `fx_*`).
    External(String),
    /// Sphere / cube / cylinder primitives are not drawn.
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Add,
    Normal,
}

#[derive(Clone, Debug)]
pub struct ParticleAnim {
    pub name: String,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
    /// Seconds added to the group clock (`ParticleGroupAnimator.animatorTimeOffset`).
    pub time_offset: f32,
    pub play_speed: f32,
    pub count: usize,
    pub shape: Shape,
    /// Texture id in the particle collection (`loot_box_glow.png` -> `loot_box_glow`).
    pub texture: Option<String>,
    pub blend: Blend,
    pub nodes: Vec<Node>,
}

#[derive(Clone, Debug, Default)]
pub struct ParticleGroup {
    pub anims: Vec<ParticleAnim>,
    pub scalable: bool,
    pub billboard: bool,
    /// `customParameters.maxScale` — `Particle3D.updateObj` clamps the
    /// driven `scale` to this cap when present.
    pub max_scale: Option<f32>,
    /// `customParameters.dispose == "scaleDown"` — `Particle3D.disposeView`
    /// shrinks the group out instead of cutting it.
    pub dispose_scale_down: bool,
}

impl ParticleGroup {
    /// One-shot horizon in seconds: the largest `delay + duration` upper
    /// bound across the group's `Time` nodes (`Particle3D` waits for the
    /// group's end event). `None` when any animator loops — the group
    /// never ends on its own (thrusters, starfields, auras). Anims
    /// without a `Time` node contribute nothing.
    pub fn one_shot_duration(&self) -> Option<f32> {
        let mut horizon = 0.0f32;
        for anim in &self.anims {
            for node in &anim.nodes {
                if let Node::Time {
                    duration,
                    delay,
                    looping,
                    ..
                } = node
                {
                    if *looping {
                        return None;
                    }
                    let d = duration.as_ref().map_or(0.0, V1::upper)
                        + delay.as_ref().map_or(0.0, V1::upper);
                    horizon = horizon.max(d);
                }
            }
        }
        (horizon > 0.0).then_some(horizon)
    }
}

/// Collection id for a recipe/material/shape url (`box_2.zip`, `a.png`,
/// `x.obj`). The stem is the group/anim identity — `particle_groups` keys,
/// render `p:` cache keys — while manifest fetches re-attach the remapped
/// extension (`{stem}.zip` / `.atf` / `.awd`), since `resources_3d_particles`
/// keys files `id.type` (`_useFilesExtensions`;
/// `ParticlesAssetLoaderContext.getRemappedUrl` maps `.png -> .atf`,
/// `.obj -> .awd`).
pub fn resource_id(url: &str) -> String {
    let name = url.rsplit('/').next().unwrap_or(url);
    match name.rsplit_once('.') {
        Some((stem, ext))
            if matches!(ext, "zip" | "awp" | "png" | "jpg" | "atf" | "obj" | "awd") =>
        {
            stem.to_owned()
        }
        _ => name.to_owned(),
    }
}

fn num(v: &Value, key: &str) -> f32 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn vec3_const(v: Option<&Value>) -> [f32; 3] {
    match v.and_then(V3::parse) {
        Some(V3::Const(c)) => c,
        _ => [0.0; 3],
    }
}

fn one_const(v: Option<&Value>, default: f32) -> f32 {
    match v.and_then(V1::parse) {
        Some(V1::Const(c)) => c,
        _ => default,
    }
}

impl ParticleGroup {
    /// Parse a downloaded particle resource: a zip holding one `.awp`, or the
    /// bare JSON.
    pub fn from_bytes(bytes: &[u8]) -> Result<ParticleGroup> {
        let json: Vec<u8> = if bytes.starts_with(b"PK") {
            zip_first_entry(bytes, ".awp")?
        } else {
            bytes.to_vec()
        };
        let root: Value = serde_json::from_slice(&json).context("particle JSON")?;
        let custom = root.get("customParameters");
        let mut group = ParticleGroup {
            scalable: custom
                .and_then(|c| c.get("scalable"))
                .and_then(Value::as_f64)
                .map_or(true, |v| v != 0.0),
            billboard: custom
                .and_then(|c| c.get("billboard"))
                .and_then(Value::as_f64)
                .is_some_and(|v| v == 1.0),
            max_scale: custom
                .and_then(|c| c.get("maxScale"))
                .and_then(Value::as_f64)
                .map(|v| v as f32),
            dispose_scale_down: custom
                .and_then(|c| c.get("dispose"))
                .and_then(Value::as_str)
                .is_some_and(|v| v == "scaleDown"),
            anims: Vec::new(),
        };
        for ad in root
            .get("animationDatas")
            .and_then(Value::as_array)
            .context("no animationDatas")?
        {
            if let Some(anim) = parse_anim(ad) {
                group.anims.push(anim);
            }
        }
        Ok(group)
    }
}

fn parse_anim(ad: &Value) -> Option<ParticleAnim> {
    let prop = ad.get("property")?.get("data")?;
    let d = ad.get("data")?;
    let assembler = d
        .get("geometry")?
        .get("data")?
        .get("assembler")?
        .get("data")?;
    let shape_v = assembler.get("shape")?;
    let sd = shape_v.get("data")?;
    let shape = match shape_v.get("id")?.as_str()? {
        "PlaneShapeSubParser" => Shape::Plane {
            w: num(sd, "width"),
            h: num(sd, "height"),
        },
        "ExternalShapeSubParser" => Shape::External(resource_id(sd.get("url")?.as_str()?)),
        _ => Shape::Unsupported,
    };
    let material = d.get("material")?;
    let md = material.get("data")?;
    let texture = md
        .get("url")
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty())
        .map(resource_id);
    let blend = if md.get("blendMode").and_then(Value::as_str) == Some("add") {
        Blend::Add
    } else {
        Blend::Normal
    };
    let nodes = d
        .get("nodes")
        .and_then(Value::as_array)
        .map(|nodes| nodes.iter().map(parse_node).collect())
        .unwrap_or_default();
    Some(ParticleAnim {
        name: d
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        position: vec3_const(prop.get("position")),
        rotation: vec3_const(prop.get("rotation")),
        scale: {
            let s = vec3_const(prop.get("scale"));
            if s == [0.0; 3] {
                [1.0; 3]
            } else {
                s
            }
        },
        time_offset: one_const(prop.get("timeOffset"), 0.0),
        play_speed: one_const(prop.get("playSpeed"), 1.0),
        count: assembler
            .get("num")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1) as usize,
        shape,
        texture,
        blend,
        nodes,
    })
}

fn parse_node(n: &Value) -> Node {
    parse_node_inner(n).unwrap_or(Node::Ignored)
}

fn parse_node_inner(n: &Value) -> Option<Node> {
    let d = n.get("data")?;
    let id = n.get("id")?.as_str()?;
    let v3 = |k: &str| d.get(k).and_then(V3::parse);
    let v4 = |k: &str| d.get(k).and_then(V4::parse);
    let col = |k: &str| d.get(k).and_then(ColorSpec::parse);
    Some(match id {
        "ParticleTimeNodeSubParser" => Node::Time {
            start: d
                .get("startTime")
                .and_then(V1::parse)
                .unwrap_or(V1::Const(0.0)),
            duration: d.get("duration").and_then(V1::parse),
            delay: d.get("delay").and_then(V1::parse),
            looping: flag(d, "usesLooping"),
            uses_delay: flag(d, "usesDelay"),
        },
        "ParticleScaleNodeSubParser" => Node::Scale {
            scale: v4("scale")?,
            cycle: flag(d, "usesCycle"),
            phase: flag(d, "usesPhase"),
        },
        "ParticleSegmentedScaleNodeSubParser" => Node::SegScale {
            start: v3("startScale")?,
            end: v3("endScale")?,
            points: d
                .get("segmentPoints")?
                .as_array()?
                .iter()
                .filter_map(|p| Some((num(p, "life"), V3::parse(p.get("scale")?)?)))
                .collect(),
        },
        "ParticleBillboardNodeSubParser" => Node::Billboard,
        "ParticleVelocityNodeSubParser" => Node::Velocity(v3("velocity")?),
        "ParticleAccelerationNodeSubParser" => Node::Accel(v3("acceleration")?),
        "ParticlePositionNodeSubParser" => Node::Position(v3("position")?),
        "ParticleRotationalVelocityNodeSubParser" => Node::RotVel(v4("rotation")?),
        "ParticleOrbitNodeSubParser" => Node::Orbit {
            orbit: v4("orbit")?,
            eulers: v3("eulers"),
            cycle: flag(d, "usesCycle"),
            phase: flag(d, "usesPhase"),
        },
        "ParticleRotateToHeadingNodeSubParser" => Node::RotateToHeading,
        "ParticleUVNodeSubParser" => Node::Uv {
            cycle: d.get("cycle").and_then(V1::parse)?,
            scale: d.get("scale").and_then(V1::parse),
            axis: d
                .get("axis")
                .and_then(Value::as_str)
                .and_then(|s| s.chars().next())
                .unwrap_or('x'),
            formula: d.get("formula").and_then(Value::as_i64).unwrap_or(1) as i32,
        },
        "ParticleSpriteSheetNodeSubParser" => Node::Sheet {
            cols: d
                .get("numColumns")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .max(1) as u32,
            rows: d.get("numRows").and_then(Value::as_u64).unwrap_or(1).max(1) as u32,
            scale: v4("scale")?,
            cycle: flag(d, "usesCycle"),
            phase: flag(d, "usesPhase"),
        },
        "ParticleSegmentedColorNodeSubParser" => Node::SegColor {
            start: col("startColor")?,
            end: col("endColor")?,
            points: d
                .get("segmentPoints")?
                .as_array()?
                .iter()
                .filter_map(|p| Some((num(p, "life"), ColorSpec::parse(p.get("color")?)?)))
                .collect(),
        },
        "ParticleInitialColorNodeSubParser" => Node::InitialColor(col("color")?),
        "ParticleColorNodeSubParser" => Node::Color {
            start: col("startColor")?,
            end: col("endColor")?,
            cycle: flag(d, "usesCycle"),
            phase: flag(d, "usesPhase"),
        },
        "ParticleOscillatorNodeSubParser" => Node::Oscillator(v4("oscillator")?),
        _ => Node::Ignored,
    })
}

// --- per-particle sampled state -------------------------------------------

#[derive(Clone, Debug)]
enum State {
    Time {
        start: f32,
        duration: f32,
        delay: f32,
    },
    Scale([f32; 4]),
    SegScale {
        start: [f32; 3],
        end: [f32; 3],
        points: Vec<(f32, [f32; 3])>,
    },
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Orbit {
        orbit: [f32; 4],
        eulers: [f32; 3],
    },
    Uv {
        cycle: f32,
        scale: f32,
    },
    Color {
        mul: Rgba,
        off: Rgba,
    },
    SegColor {
        start: (Rgba, Rgba),
        end: (Rgba, Rgba),
        points: Vec<(f32, (Rgba, Rgba))>,
    },
    ColorRange {
        start: (Rgba, Rgba),
        end: (Rgba, Rgba),
    },
    None,
}

/// One particle's sampled properties.
#[derive(Clone, Debug)]
pub struct Particle {
    states: Vec<State>,
}

impl ParticleAnim {
    /// Sample every particle's random properties (`generatePropertyOfOneParticle`).
    pub fn instantiate(&self, seed: u64) -> Vec<Particle> {
        let mut rng = Rng::new(seed ^ 0xA5A5_5A5A);
        (0..self.count)
            .map(|i| Particle {
                states: self
                    .nodes
                    .iter()
                    .map(|node| sample_node(node, &mut rng, i, self.count))
                    .collect(),
            })
            .collect()
    }
}

fn sample_node(node: &Node, rng: &mut Rng, i: usize, n: usize) -> State {
    match node {
        Node::Time {
            start,
            duration,
            delay,
            ..
        } => State::Time {
            start: start.sample(rng, i, n),
            duration: duration
                .as_ref()
                .map_or(1.0, |d| d.sample(rng, i, n))
                .max(1e-3),
            delay: delay.as_ref().map_or(0.0, |d| d.sample(rng, i, n)),
        },
        Node::Scale { scale, .. } => State::Scale(scale.sample(rng, i, n)),
        Node::SegScale { start, end, points } => State::SegScale {
            start: start.sample(rng, i, n),
            end: end.sample(rng, i, n),
            points: points
                .iter()
                .map(|(life, v)| (*life, v.sample(rng, i, n)))
                .collect(),
        },
        Node::Velocity(v) | Node::Accel(v) | Node::Position(v) => State::Vec3(v.sample(rng, i, n)),
        Node::RotVel(v) | Node::Oscillator(v) => State::Vec4(v.sample(rng, i, n)),
        Node::Orbit { orbit, eulers, .. } => State::Orbit {
            orbit: orbit.sample(rng, i, n),
            eulers: eulers.as_ref().map_or([0.0; 3], |e| e.sample(rng, i, n)),
        },
        Node::Uv { cycle, scale, .. } => State::Uv {
            cycle: cycle.sample(rng, i, n),
            scale: scale.as_ref().map_or(1.0, |s| s.sample(rng, i, n)),
        },
        Node::Sheet { scale, .. } => State::Vec4(scale.sample(rng, i, n)),
        Node::InitialColor(c) => {
            let (mul, off) = c.sample(rng, i, n);
            State::Color { mul, off }
        }
        Node::SegColor { start, end, points } => State::SegColor {
            start: start.sample(rng, i, n),
            end: end.sample(rng, i, n),
            points: points
                .iter()
                .map(|(life, c)| (*life, c.sample(rng, i, n)))
                .collect(),
        },
        Node::Color { start, end, .. } => State::ColorRange {
            start: start.sample(rng, i, n),
            end: end.sample(rng, i, n),
        },
        Node::Billboard | Node::RotateToHeading | Node::Ignored => State::None,
    }
}

// --- evaluation ------------------------------------------------------------

/// One particle at one instant, in the instance's local (Away3D) space.
#[derive(Clone, Debug)]
pub struct Quad {
    pub pos: [f32; 3],
    /// Per-axis scale factor of the particle shape.
    pub scale: [f32; 3],
    /// Rotation quaternion `[w, x, y, z]` of the particle shape.
    pub rot: [f32; 4],
    pub velocity: [f32; 3],
    pub billboard: bool,
    pub rotate_to_heading: bool,
    /// `(u0, v0, u1, v1)` of the texture rectangle to show.
    pub uv: [f32; 4],
    /// Final tint (multiplier + offset), straight alpha 0..1.
    pub color: Rgba,
}

impl ParticleAnim {
    /// Evaluate one particle at group time `time` (seconds, before this
    /// anim's play speed / offset). `None` while it is not alive.
    pub fn eval(&self, particle: &Particle, time: f32) -> Option<Quad> {
        let t_group = time * self.play_speed + self.time_offset;
        let mut q = Quad {
            pos: [0.0; 3],
            scale: [1.0; 3],
            rot: [1.0, 0.0, 0.0, 0.0],
            velocity: [0.0; 3],
            billboard: false,
            rotate_to_heading: false,
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [1.0; 4],
        };
        let mut mul = [1.0f32; 4];
        let mut off = [0.0f32; 4];
        let mut uv_off = [0.0f32; 2];
        let mut vtime = 0.0f32;
        let mut life = 0.0f32;
        for (node, state) in self.nodes.iter().zip(&particle.states) {
            match (node, state) {
                (
                    Node::Time {
                        looping,
                        uses_delay,
                        ..
                    },
                    State::Time {
                        start,
                        duration,
                        delay,
                    },
                ) => {
                    let mut t = t_group - start;
                    if t < 0.0 {
                        return None;
                    }
                    if *looping {
                        if *uses_delay {
                            let period = delay + duration;
                            t = (t / period).fract() * period;
                            if t >= *duration {
                                return None;
                            }
                        } else {
                            t = (t / duration).fract() * duration;
                        }
                    } else if t > *duration {
                        return None;
                    }
                    vtime = t;
                    life = t / duration;
                }
                (Node::Scale { cycle, phase, .. }, State::Scale(s)) => {
                    let value = if *cycle {
                        let c = if s[2] > 0.0 {
                            std::f32::consts::TAU / s[2]
                        } else {
                            0.0
                        };
                        let p = if *phase { s[3].to_radians() } else { 0.0 };
                        (s[0] + s[1]) * 0.5 + (s[0] - s[1]).abs() * 0.5 * (vtime * c + p).sin()
                    } else {
                        s[0] + (s[1] - s[0]) * life
                    };
                    for k in 0..3 {
                        q.scale[k] *= value;
                    }
                }
                (Node::SegScale { .. }, State::SegScale { start, end, points }) => {
                    let mut keys: Vec<(f32, [f32; 3])> = vec![(0.0, *start)];
                    keys.extend(points.iter().copied());
                    keys.push((1.0, *end));
                    let v = piecewise3(&keys, life);
                    for k in 0..3 {
                        q.scale[k] *= v[k];
                    }
                }
                (Node::Billboard, _) => q.billboard = true,
                (Node::RotateToHeading, _) => q.rotate_to_heading = true,
                (Node::Velocity(_), State::Vec3(v)) => {
                    for k in 0..3 {
                        q.pos[k] += v[k] * vtime;
                        q.velocity[k] += v[k];
                    }
                }
                (Node::Accel(_), State::Vec3(a)) => {
                    for k in 0..3 {
                        q.pos[k] += 0.5 * a[k] * vtime * vtime;
                        q.velocity[k] += a[k] * vtime;
                    }
                }
                (Node::Position(_), State::Vec3(p)) => {
                    for k in 0..3 {
                        q.pos[k] += p[k];
                    }
                }
                (Node::RotVel(_), State::Vec4(r)) => {
                    let len = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
                    let axis = if len > 0.0 {
                        [r[0] / len, r[1] / len, r[2] / len]
                    } else {
                        [0.0, 0.0, 1.0]
                    };
                    if r[3] > 0.0 {
                        let angle = std::f32::consts::TAU * vtime / r[3];
                        q.rot = quat_mul(&quat_axis(&axis, angle), &q.rot);
                    }
                }
                (Node::Orbit { cycle, phase, .. }, State::Orbit { orbit, eulers }) => {
                    let angle = if *cycle {
                        let c = if orbit[1] > 0.0 {
                            std::f32::consts::TAU / orbit[1]
                        } else {
                            0.0
                        };
                        vtime * c + if *phase { orbit[2].to_radians() } else { 0.0 }
                    } else {
                        life * std::f32::consts::TAU
                    };
                    let mut p = [angle.cos() * orbit[0], angle.sin() * orbit[0], 0.0];
                    let mut v = [-angle.sin(), angle.cos(), 0.0];
                    let e = euler_quat(eulers);
                    p = quat_rotate(&e, &p);
                    v = quat_rotate(&e, &v);
                    let speed = if *cycle && orbit[1] > 0.0 {
                        orbit[0] * std::f32::consts::TAU / orbit[1]
                    } else {
                        0.0
                    };
                    for k in 0..3 {
                        q.pos[k] += p[k];
                        q.velocity[k] += v[k] * speed;
                    }
                }
                (Node::Oscillator(_), State::Vec4(o)) => {
                    if o[3] > 0.0 {
                        let s = (vtime * std::f32::consts::TAU / o[3]).sin();
                        for k in 0..3 {
                            q.pos[k] += o[k] * s;
                        }
                    }
                }
                (Node::Uv { axis, formula, .. }, State::Uv { cycle, scale }) => {
                    let amount = if *formula == 2 {
                        (vtime * std::f32::consts::TAU / cycle.max(1e-3)).sin() * scale
                    } else {
                        vtime / cycle.max(1e-3) * scale
                    };
                    if *axis == 'y' {
                        uv_off[1] += amount;
                    } else {
                        uv_off[0] += amount;
                    }
                }
                (
                    Node::Sheet {
                        cols,
                        rows,
                        cycle,
                        phase,
                        ..
                    },
                    State::Vec4(s),
                ) => {
                    let total = (cols * rows) as f32;
                    let frame = if *cycle {
                        let dur = s[0].max(1e-3);
                        let p = if *phase { s[1] } else { 0.0 };
                        (((vtime + p) / dur).fract() * total).floor()
                    } else {
                        (life * total).floor()
                    }
                    .clamp(0.0, total - 1.0) as u32;
                    let (cw, ch) = (1.0 / *cols as f32, 1.0 / *rows as f32);
                    let (c, r) = ((frame % cols) as f32, (frame / cols) as f32);
                    q.uv = [c * cw, r * ch, (c + 1.0) * cw, (r + 1.0) * ch];
                }
                (Node::InitialColor(_), State::Color { mul: m, off: o }) => {
                    for k in 0..4 {
                        mul[k] *= m[k];
                        off[k] += o[k];
                    }
                }
                (Node::SegColor { .. }, State::SegColor { start, end, points }) => {
                    let mut keys = vec![(0.0, *start)];
                    keys.extend(points.iter().copied());
                    keys.push((1.0, *end));
                    let (m, o) = piecewise_color(&keys, life);
                    for k in 0..4 {
                        mul[k] *= m[k];
                        off[k] += o[k];
                    }
                }
                (Node::Color { cycle, phase, .. }, State::ColorRange { start, end }) => {
                    let f = if *cycle {
                        let p = if *phase { 0.0 } else { 0.0 };
                        (0.5 + 0.5 * (vtime * std::f32::consts::TAU + p).sin()).clamp(0.0, 1.0)
                    } else {
                        life
                    };
                    for k in 0..4 {
                        mul[k] *= start.0[k] + (end.0[k] - start.0[k]) * f;
                        off[k] += start.1[k] + (end.1[k] - start.1[k]) * f;
                    }
                }
                _ => {}
            }
        }
        if q.uv[2] - q.uv[0] > 0.0 {
            q.uv[0] += uv_off[0];
            q.uv[2] += uv_off[0];
            q.uv[1] += uv_off[1];
            q.uv[3] += uv_off[1];
        }
        for k in 0..4 {
            q.color[k] = (mul[k] + off[k]).clamp(0.0, 1.0);
        }
        Some(q)
    }
}

fn piecewise3(keys: &[(f32, [f32; 3])], t: f32) -> [f32; 3] {
    for w in keys.windows(2) {
        if t <= w[1].0 {
            let span = (w[1].0 - w[0].0).max(1e-6);
            let f = ((t - w[0].0) / span).clamp(0.0, 1.0);
            return std::array::from_fn(|k| w[0].1[k] + (w[1].1[k] - w[0].1[k]) * f);
        }
    }
    keys.last().map_or([1.0; 3], |k| k.1)
}

fn piecewise_color(keys: &[(f32, (Rgba, Rgba))], t: f32) -> (Rgba, Rgba) {
    for w in keys.windows(2) {
        if t <= w[1].0 {
            let span = (w[1].0 - w[0].0).max(1e-6);
            let f = ((t - w[0].0) / span).clamp(0.0, 1.0);
            let m = std::array::from_fn(|k| w[0].1 .0[k] + (w[1].1 .0[k] - w[0].1 .0[k]) * f);
            let o = std::array::from_fn(|k| w[0].1 .1[k] + (w[1].1 .1[k] - w[0].1 .1[k]) * f);
            return (m, o);
        }
    }
    keys.last().map_or(([1.0; 4], [0.0; 4]), |k| k.1)
}

// --- quaternion helpers ([w, x, y, z]) ------------------------------------

pub fn quat_axis(axis: &[f32; 3], angle: f32) -> [f32; 4] {
    let (s, c) = (angle * 0.5).sin_cos();
    [c, axis[0] * s, axis[1] * s, axis[2] * s]
}

pub fn quat_mul(a: &[f32; 4], b: &[f32; 4]) -> [f32; 4] {
    [
        a[0] * b[0] - a[1] * b[1] - a[2] * b[2] - a[3] * b[3],
        a[0] * b[1] + a[1] * b[0] + a[2] * b[3] - a[3] * b[2],
        a[0] * b[2] - a[1] * b[3] + a[2] * b[0] + a[3] * b[1],
        a[0] * b[3] + a[1] * b[2] - a[2] * b[1] + a[3] * b[0],
    ]
}

pub fn quat_rotate(q: &[f32; 4], v: &[f32; 3]) -> [f32; 3] {
    let u = [q[1], q[2], q[3]];
    let dot = u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
    let cross = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let uu = u[0] * u[0] + u[1] * u[1] + u[2] * u[2];
    std::array::from_fn(|k| v[k] * (q[0] * q[0] - uu) + 2.0 * u[k] * dot + 2.0 * q[0] * cross[k])
}

/// Z * Y * X Euler degrees (same order as `mapscene`).
pub fn euler_quat(deg: &[f32; 3]) -> [f32; 4] {
    let qx = quat_axis(&[1.0, 0.0, 0.0], deg[0].to_radians());
    let qy = quat_axis(&[0.0, 1.0, 0.0], deg[1].to_radians());
    let qz = quat_axis(&[0.0, 0.0, 1.0], deg[2].to_radians());
    quat_mul(&qz, &quat_mul(&qy, &qx))
}

fn quat_between_y(dir: &[f32; 3]) -> [f32; 4] {
    // Rotation taking +Y to `dir`.
    let axis = [dir[2], 0.0, -dir[0]];
    let len = (axis[0] * axis[0] + axis[2] * axis[2]).sqrt();
    let angle = dir[1].clamp(-1.0, 1.0).acos();
    if len < 1e-6 {
        return if dir[1] >= 0.0 {
            [1.0, 0.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0, 0.0]
        };
    }
    quat_axis(&[axis[0] / len, 0.0, axis[2] / len], angle)
}

// --- minimal zip reader (stored / deflate) --------------------------------

/// Contents of the first entry whose name ends with `suffix`. Particle zips
/// hold exactly one `.awp`, so a central-directory walk is all that is needed.
fn zip_first_entry(zip: &[u8], suffix: &str) -> Result<Vec<u8>> {
    let eocd = zip
        .windows(4)
        .rposition(|w| w == b"PK\x05\x06")
        .context("zip: no end-of-central-directory record")?;
    let u16at = |o: usize| u16::from_le_bytes([zip[o], zip[o + 1]]) as usize;
    let u32at =
        |o: usize| u32::from_le_bytes([zip[o], zip[o + 1], zip[o + 2], zip[o + 3]]) as usize;
    let entries = u16at(eocd + 10);
    let mut cursor = u32at(eocd + 16);
    for _ in 0..entries {
        if zip.get(cursor..cursor + 4) != Some(b"PK\x01\x02") {
            bail!("zip: bad central directory");
        }
        let method = u16at(cursor + 10);
        let comp = u32at(cursor + 20);
        let name_len = u16at(cursor + 28);
        let extra_len = u16at(cursor + 30);
        let comment_len = u16at(cursor + 32);
        let local = u32at(cursor + 42);
        let name = std::str::from_utf8(&zip[cursor + 46..cursor + 46 + name_len]).unwrap_or("");
        if name.ends_with(suffix) {
            let lname = u16at(local + 26);
            let lextra = u16at(local + 28);
            let start = local + 30 + lname + lextra;
            let data = zip
                .get(start..start + comp)
                .context("zip: truncated entry")?;
            return match method {
                0 => Ok(data.to_vec()),
                8 => {
                    let mut out = Vec::new();
                    flate2::read::DeflateDecoder::new(data)
                        .take(64 * 1024 * 1024)
                        .read_to_end(&mut out)
                        .context("zip: inflate")?;
                    Ok(out)
                }
                m => bail!("zip: unsupported compression method {m}"),
            };
        }
        cursor += 46 + name_len + extra_len + comment_len;
    }
    bail!("zip: no {suffix} entry")
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOX2: &[u8] = include_bytes!("../../tests/fixtures/particles/box_2.zip");
    const MORDON: &[u8] = include_bytes!("../../tests/fixtures/particles/mordon_boss.zip");

    #[test]
    fn parses_loot_box_particle_group() {
        let g = ParticleGroup::from_bytes(BOX2).unwrap();
        assert!(g.anims.len() >= 3, "{} anims", g.anims.len());
        let glowy = &g.anims[0];
        assert_eq!(glowy.name, "glowy_box");
        assert!(matches!(&glowy.shape, Shape::External(id) if id == "fx_loot_box_glow"));
        assert_eq!(glowy.texture.as_deref(), Some("loot_box_glow"));
        assert_eq!(glowy.blend, Blend::Normal);
        let orange = &g.anims[1];
        assert!(matches!(orange.shape, Shape::Plane { w, .. } if (w - 73.0).abs() < 1e-3));
        assert_eq!(orange.blend, Blend::Add);
    }

    #[test]
    fn evaluates_looping_particles_deterministically() {
        let g = ParticleGroup::from_bytes(BOX2).unwrap();
        let glowy = &g.anims[0];
        let p = glowy.instantiate(7);
        let a = glowy.eval(&p[0], 0.5).expect("alive at 0.5s");
        let b = glowy
            .eval(&p[0], 0.5 + 2.0)
            .expect("loops with the 2s duration");
        // Scale cycle 17..18 with a phase-free sine: same value one period later.
        assert!((a.scale[0] - b.scale[0]).abs() < 1e-3);
        assert!(
            a.scale[0] > 16.0 && a.scale[0] < 19.0,
            "scale {}",
            a.scale[0]
        );
        // The glow planes use random start times in [0, 2): hidden before they start.
        let orange = &g.anims[1];
        let ps = orange.instantiate(7);
        assert!(ps.iter().any(|p| orange.eval(p, 3.0).is_some()));
    }

    #[test]
    fn parses_every_node_of_a_ship_effect() {
        let g = ParticleGroup::from_bytes(MORDON).unwrap();
        assert!(!g.anims.is_empty());
        for anim in &g.anims {
            let parts = anim.instantiate(1);
            assert_eq!(parts.len(), anim.count);
            for p in &parts {
                let _ = anim.eval(p, 1.0);
            }
        }
    }

    /// `XENOMIT_PARTICLE_DIR=<dir of *.zip>`: every real particle file must
    /// parse and evaluate without panicking.
    #[test]
    fn real_particle_corpus_parses_when_requested() {
        let Some(dir) = std::env::var_os("XENOMIT_PARTICLE_DIR") else {
            return;
        };
        let mut files = 0;
        for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
            if entry.path().extension().and_then(|e| e.to_str()) != Some("zip") {
                continue;
            }
            let bytes = std::fs::read(entry.path()).unwrap();
            let group = ParticleGroup::from_bytes(&bytes)
                .unwrap_or_else(|e| panic!("{}: {e:#}", entry.path().display()));
            for anim in &group.anims {
                for p in anim.instantiate(3) {
                    for t in [0.0, 0.3, 1.7, 9.0] {
                        let _ = anim.eval(&p, t);
                    }
                }
            }
            files += 1;
        }
        assert!(files > 0);
    }
}
