//! Small, defensive reader for the static triangle-geometry subset of the
//! Away3D AWD 2.1 files used by DarkOrbit's `display3D` mesh recipes.
//!
//! This follows `away3d.loaders.parsers.AWD2Parser.parseHeader`,
//! `parseNextBlock` and `parseTriangleGeometrieBlock` in the decompiled
//! client. It deliberately rejects unsupported compression and malformed
//! buffers instead of inventing replacement geometry.

use anyhow::{bail, ensure, Context, Result};
use flate2::read::ZlibDecoder;
use std::io::Read;

const MAX_AWD_DECOMPRESSED: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct AwdMesh {
    pub name: String,
    /// Flattened triangle vertices in the coordinate system authored by DO.
    pub positions: Vec<[f32; 3]>,
    /// One UV per flattened position when the geometry includes channel 0.
    pub uvs: Vec<[f32; 2]>,
    pub triangles: Vec<[u32; 3]>,
}

impl AwdMesh {
    /// Convert to a tiny Wavefront OBJ so raylib's native model loader can
    /// upload the real authored mesh without adding a second GPU backend.
    ///
    /// Away3D is left-handed with clockwise front faces; raylib is
    /// right-handed with counter-clockwise ones. The OBJ therefore stores
    /// `(x, y, -z)` with reversed triangle winding, which keeps the picture
    /// identical to Away3D's when the scene is drawn with the same z flip
    /// (see `render::scene3d`).
    ///
    /// raylib's OBJ loader indexes `vt`/`vn` unconditionally (a missing
    /// attribute reads through a NULL pointer), so both are always written:
    /// smooth area-weighted normals, and zero UVs when the mesh has none.
    pub fn to_obj(&self) -> String {
        let positions: Vec<[f32; 3]> = self.positions.iter().map(|p| [p[0], p[1], -p[2]]).collect();
        let mut out = String::with_capacity(positions.len() * 96);
        out.push_str("# DarkOrbit AWD 2.1 static mesh (z flipped to right-handed)\n");
        for p in &positions {
            out.push_str(&format!("v {:.7} {:.7} {:.7}\n", p[0], p[1], p[2]));
        }
        if self.uvs.len() == positions.len() {
            for uv in &self.uvs {
                out.push_str(&format!("vt {:.7} {:.7}\n", uv[0], 1.0 - uv[1]));
            }
        } else {
            for _ in &positions {
                out.push_str("vt 0.0000000 0.0000000\n");
            }
        }
        // Reversed winding: (a, b, c) -> (a, c, b).
        let triangles: Vec<[usize; 3]> = self
            .triangles
            .iter()
            .map(|t| [t[0] as usize, t[2] as usize, t[1] as usize])
            .filter(|t| t.iter().all(|&i| i < positions.len()))
            .collect();
        let mut normals = vec![[0.0f32; 3]; positions.len()];
        for &[a, b, c] in &triangles {
            let (pa, pb, pc) = (positions[a], positions[b], positions[c]);
            let u = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
            let v = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            for i in [a, b, c] {
                for k in 0..3 {
                    normals[i][k] += n[k];
                }
            }
        }
        for n in &normals {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let n = if len > 1e-12 {
                [n[0] / len, n[1] / len, n[2] / len]
            } else {
                [0.0, 1.0, 0.0]
            };
            out.push_str(&format!("vn {:.7} {:.7} {:.7}\n", n[0], n[1], n[2]));
        }
        for [a, b, c] in triangles {
            let (a, b, c) = (a + 1, b + 1, c + 1);
            out.push_str(&format!("f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}\n"));
        }
        out
    }
}

/// Mesh-instance names `ShipView3D.processMesh` consumes as attachment
/// points instead of rendering them.
const HELPER_MESH_NAMES: [&str; 6] = [
    "engine_",
    "laserpoint_",
    "light_laser",
    "light_position",
    "rocketlauncher",
    "dockingPoint_",
];

struct MeshInstance {
    name: String,
    geometry: u32,
    /// Row-vector 3x4 matrix as stored by AWD (`Matrix3D.rawData`): three
    /// basis rows followed by the translation.
    matrix: [f32; 12],
}

/// Shared AWD 2.1 block walk: header check, decompression and the
/// geometry (`1`) / mesh-instance (`23`) block collection that
/// [`parse_aw2`] and [`parse_aw2_slots`] share.
fn awd_geometry_and_instances(
    bytes: &[u8],
) -> Result<(std::collections::HashMap<u32, AwdMesh>, Vec<MeshInstance>)> {
    ensure!(
        bytes.len() >= 12 && &bytes[..3] == b"AWD",
        "not an AWD file"
    );
    // Some DarkOrbit models (e.g. `cbs`) carry major byte 0x63 instead of 2
    // but use the identical 2.1 block layout.
    ensure!(
        matches!(bytes[3], 2 | 0x63) && bytes[4] == 1,
        "only AWD 2.1 is supported"
    );
    // AWD2Parser only honors the precision flags for exactly version 2.1.
    let flags = u16::from_le_bytes([bytes[5], bytes[6]]);
    ensure!(
        bytes[3] != 2 || flags & 0b110 == 0,
        "AWD float64 matrices/geometry are not supported"
    );
    let compression = bytes[7];
    let body = match compression {
        0 => bytes[12..].to_vec(),
        1 => inflate_bounded(&bytes[12..]).context("inflate AWD body")?,
        v => bail!("AWD compression mode {v} is not supported yet"),
    };
    let mut cursor = Cursor::new(&body);
    let mut geometries = std::collections::HashMap::<u32, AwdMesh>::new();
    let mut instances = Vec::<MeshInstance>::new();
    while cursor.remaining() >= 11 {
        let id = cursor.u32()?;
        let _namespace = cursor.u8()?;
        let block_type = cursor.u8()?;
        let block_flags = cursor.u8()?;
        let length = cursor.u32()? as usize;
        ensure!(
            length <= cursor.remaining(),
            "AWD block length exceeds body"
        );
        let block = Cursor::new(cursor.take(length)?);
        let block = if block_flags & 8 != 0 {
            inflate_bounded(block.bytes).context("inflate AWD block")?
        } else {
            block.bytes.to_vec()
        };
        match block_type {
            1 => {
                if let Some(mesh) = parse_geometry_block(&block)
                    .with_context(|| format!("parse AWD geometry block {id}"))?
                {
                    geometries.insert(id, mesh);
                }
            }
            23 => {
                if let Some(instance) = parse_mesh_instance(&block)
                    .with_context(|| format!("parse AWD mesh instance {id}"))?
                {
                    instances.push(instance);
                }
            }
            _ => {}
        }
    }
    Ok((geometries, instances))
}

/// Instance-matrix transform of a model-local point (`MeshInstance.matrix`
/// is a row-vector 3x4 `Matrix3D.rawData` — the same convention
/// `parse_aw2` bakes vertices with).
fn instance_transform(m: &[f32; 12], p: [f32; 3]) -> [f32; 3] {
    [
        p[0] * m[0] + p[1] * m[3] + p[2] * m[6] + m[9],
        p[0] * m[1] + p[1] * m[4] + p[2] * m[7] + m[10],
        p[0] * m[2] + p[1] * m[5] + p[2] * m[8] + m[11],
    ]
}

/// `ShipView3D.processMesh` + `Slot3D.fromMesh` attachment extraction.
///
/// For every top-level mesh instance named `{prefix}*` (stock matches
/// `indexOf` but authored names all *start* with it) this returns
/// `(name-without-prefix, position)` where the position is the referenced
/// geometry's bounds center pushed through the instance matrix — the slot
/// in the model's local (AWD-root) coordinate space, to be placed in the
/// world by the mesh node's transform (`Lasers3D`/`scene3d` chain).
pub fn parse_aw2_slots(bytes: &[u8], prefix: &str) -> Result<Vec<(String, [f32; 3])>> {
    let (geometries, instances) = awd_geometry_and_instances(bytes)?;
    let mut out = Vec::new();
    for instance in &instances {
        let Some(short) = instance.name.strip_prefix(prefix) else {
            continue;
        };
        let Some(geometry) = geometries.get(&instance.geometry) else {
            continue;
        };
        // `bounds.fromGeometry` center: (min + max) / 2 per axis.
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for p in &geometry.positions {
            for k in 0..3 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
        if min[0] > max[0] {
            continue;
        }
        let center = std::array::from_fn(|k| (min[k] + max[k]) * 0.5);
        out.push((
            short.to_owned(),
            instance_transform(&instance.matrix, center),
        ));
    }
    Ok(out)
}

/// Parse one AWD 2.1 static geometry collection into a single mesh.
///
/// Follows `PrefabView3D.reinitializeMeshes` / `ShipView3D.processMesh`:
/// every top-level mesh instance is rendered with its own authored matrix,
/// except the helper/attachment meshes named in [`HELPER_MESH_NAMES`]. The
/// instances are baked into one mesh (Away3D left-handed coordinates; see
/// [`AwdMesh::to_obj`] for the conversion to raylib).
pub fn parse_aw2(bytes: &[u8]) -> Result<AwdMesh> {
    let (geometries, instances) = awd_geometry_and_instances(bytes)?;
    let mut out = AwdMesh::default();
    for instance in &instances {
        if HELPER_MESH_NAMES
            .iter()
            .any(|helper| instance.name.contains(helper))
        {
            continue;
        }
        let Some(geometry) = geometries.get(&instance.geometry) else {
            continue;
        };
        let offset = out.positions.len() as u32;
        out.positions.extend(
            geometry
                .positions
                .iter()
                .map(|p| instance_transform(&instance.matrix, *p)),
        );
        out.triangles.extend(
            geometry
                .triangles
                .iter()
                .map(|t| [t[0] + offset, t[1] + offset, t[2] + offset]),
        );
        if geometry.uvs.len() == geometry.positions.len() {
            out.uvs.extend(geometry.uvs.iter().copied());
        } else {
            out.uvs
                .extend(std::iter::repeat([0.0, 0.0]).take(geometry.positions.len()));
        }
        if out.name.is_empty() {
            out.name = instance.name.clone();
        }
    }
    if out.triangles.is_empty() {
        // Geometry-only files (particle `ExternalShapeSubParser` shapes) have
        // no mesh instance: use every geometry block as authored.
        let mut ids: Vec<_> = geometries.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let geometry = &geometries[&id];
            let offset = out.positions.len() as u32;
            out.positions.extend(geometry.positions.iter().copied());
            out.triangles.extend(
                geometry
                    .triangles
                    .iter()
                    .map(|t| [t[0] + offset, t[1] + offset, t[2] + offset]),
            );
            if geometry.uvs.len() == geometry.positions.len() {
                out.uvs.extend(geometry.uvs.iter().copied());
            } else {
                out.uvs
                    .extend(std::iter::repeat([0.0, 0.0]).take(geometry.positions.len()));
            }
        }
    }
    ensure!(
        !out.triangles.is_empty(),
        "AWD has no supported triangle geometry"
    );
    Ok(out)
}

/// `AWD2Parser.parseMeshInstance`: only instances parented to the root are
/// direct children of the geometry source, which is what `PrefabView3D`
/// instantiates; nested ones are skipped (`None`).
fn parse_mesh_instance(bytes: &[u8]) -> Result<Option<MeshInstance>> {
    let mut c = Cursor::new(bytes);
    let parent = c.u32()?;
    let mut matrix = [0.0f32; 12];
    for value in &mut matrix {
        *value = c.f32()?;
    }
    let name = c.string_u16()?;
    let geometry = c.u32()?;
    Ok((parent == 0).then_some(MeshInstance {
        name,
        geometry,
        matrix,
    }))
}

fn inflate_bounded(input: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = ZlibDecoder::new(input);
    let mut data = Vec::with_capacity(input.len().min(MAX_AWD_DECOMPRESSED));
    decoder
        .by_ref()
        .take((MAX_AWD_DECOMPRESSED + 1) as u64)
        .read_to_end(&mut data)
        .context("inflate zlib stream")?;
    ensure!(
        data.len() <= MAX_AWD_DECOMPRESSED,
        "AWD decompressed size exceeds 128 MiB"
    );
    Ok(data)
}

fn parse_geometry_block(bytes: &[u8]) -> Result<Option<AwdMesh>> {
    let mut c = Cursor::new(bytes);
    let name = c.string_u16()?;
    let sub_count = c.u16()? as usize;
    // Root scaleUV properties: the game uses the default 1x1 on its ship
    // geometry, so unknown property values can be skipped safely here.
    skip_props(&mut c)?;
    let mut mesh = AwdMesh {
        name,
        ..Default::default()
    };
    for _ in 0..sub_count {
        let sub_len = c.u32()? as usize;
        ensure!(
            sub_len <= c.remaining(),
            "AWD subgeometry length exceeds block"
        );
        let sub_bytes = c.take(sub_len)?;
        let mut sub = Cursor::new(sub_bytes);
        skip_props(&mut sub)?;
        let mut vertices = Vec::<[f32; 3]>::new();
        let mut indices = Vec::<u32>::new();
        let mut uvs = Vec::<[f32; 2]>::new();
        while sub.remaining() >= 6 {
            let kind = sub.u8()?;
            let _flags = sub.u8()?;
            let attr_len = sub.u32()? as usize;
            ensure!(
                attr_len <= sub.remaining(),
                "AWD geometry attribute exceeds subgeometry"
            );
            let mut attr = Cursor::new(sub.take(attr_len)?);
            match kind {
                1 => {
                    while attr.remaining() >= 12 {
                        vertices.push([attr.f32()?, attr.f32()?, attr.f32()?]);
                    }
                    ensure!(attr.remaining() == 0, "partial AWD position tuple");
                }
                2 => {
                    while attr.remaining() >= 2 {
                        indices.push(attr.u16()? as u32);
                    }
                }
                3 => {
                    while attr.remaining() >= 8 {
                        uvs.push([attr.f32()?, attr.f32()?]);
                    }
                }
                _ => {} // normals, tangents, joints, secondary UVs, etc.
            }
        }
        // `GeomUtil.fromVectors` creates triangle indices when omitted.
        if vertices.len() >= 3 {
            let offset = mesh.positions.len() as u32;
            let base = mesh.triangles.len() as u32;
            let valid_uvs = uvs.len() == vertices.len();
            mesh.positions.extend(vertices.iter().copied());
            if valid_uvs {
                mesh.uvs.extend(uvs);
            }
            if indices.is_empty() {
                indices.extend(0..vertices.len() as u32);
            }
            for tri in indices.chunks_exact(3) {
                if tri.iter().all(|i| (*i as usize) < vertices.len()) {
                    mesh.triangles
                        .push([offset + tri[0], offset + tri[1], offset + tri[2]]);
                }
            }
            if !valid_uvs {
                mesh.uvs.clear();
            }
            debug_assert!(mesh.triangles.len() as u32 >= base);
        }
        // AWD2Parser consumes one user-attribute list immediately after each
        // length-delimited subgeometry, in the parent geometry stream.
        skip_user_attributes(&mut c)?;
    }
    skip_user_attributes(&mut c)?;
    Ok((!mesh.triangles.is_empty()).then_some(mesh))
}

fn skip_props(c: &mut Cursor<'_>) -> Result<()> {
    let len = c.u32()? as usize;
    ensure!(len <= c.remaining(), "AWD property block exceeds geometry");
    c.skip(len)
}

fn skip_user_attributes(c: &mut Cursor<'_>) -> Result<()> {
    let len = c.u32()? as usize;
    ensure!(len <= c.remaining(), "AWD user attributes exceed geometry");
    c.skip(len)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        ensure!(n <= self.remaining(), "truncated AWD data");
        let start = self.pos;
        self.pos += n;
        Ok(&self.bytes[start..self.pos])
    }
    fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn string_u16(&mut self) -> Result<String> {
        let len = self.u16()? as usize;
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_genuine_darkorbit_static_meshes() {
        for (bytes, min_triangles, name) in [
            (
                include_bytes!("../../tests/fixtures/3d/aegis.awd").as_slice(),
                100,
                "aegis",
            ),
            (
                include_bytes!("../../tests/fixtures/3d/ore-palladium.awd").as_slice(),
                20,
                "ore-palladium",
            ),
        ] {
            let mesh = parse_aw2(bytes).unwrap_or_else(|e| panic!("parse {name} AWD2.1: {e:#}"));
            assert!(
                mesh.triangles.len() >= min_triangles,
                "{} triangles in {}",
                mesh.triangles.len(),
                name
            );
            assert_eq!(mesh.positions.len(), mesh.uvs.len());
            assert!(mesh.to_obj().contains("\nf "));
        }
    }

    #[test]
    fn reads_genuine_darkorbit_laser_slots() {
        // aegis.awd authors `laserpoint_centerFront`/`leftRearInner`/
        // `rightFrontInner`/`rightRearInner` helper instances.
        let bytes = include_bytes!("../../tests/fixtures/3d/aegis.awd").as_slice();
        let slots = parse_aw2_slots(bytes, "laserpoint_").expect("slots");
        let names: Vec<&str> = slots.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names.len(), 4, "laserpoint instances in aegis: {names:?}");
        for want in [
            "centerFront",
            "leftRearInner",
            "rightFrontInner",
            "rightRearInner",
        ] {
            assert!(names.contains(&want), "missing slot {want}");
        }
        // Slots are model-local points near the hull, not the origin.
        for (name, p) in &slots {
            assert!(
                p.iter().all(|v| v.is_finite() && v.abs() < 500.0),
                "slot {name} implausible: {p:?}"
            );
        }
        // The mesh bake still skips the helper instances.
        let mesh = parse_aw2(bytes).expect("mesh");
        assert!(mesh.triangles.len() >= 100);
    }
}

#[cfg(test)]
mod cached_tests {
    #[test]
    fn parses_cached_3d_models_when_present() {
        let dir = std::path::Path::new(".xenomit-assets/resources_3d/files");
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            let bytes = std::fs::read(entry.path()).unwrap();
            if bytes.starts_with(b"AWD") {
                let mesh = super::parse_aw2(&bytes).unwrap_or_else(|e| panic!("{name}: {e:#}"));
                assert!(!mesh.positions.is_empty(), "{name} has no geometry");
            }
        }
    }
}
