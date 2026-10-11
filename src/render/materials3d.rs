//! Shared shader set for authored `display3D` meshes — the
//! `EntityBasicMaterial` port.
//!
//! One lit + one unlit (`light="false"`) program serve every model:
//! per-material state rides in the material's map slots and
//! `maps[DIFFUSE].color` (uploaded by `DrawMesh` as `colDiffuse`, decoded
//! by the shader — see [`apply`]), while scene lighting is pushed once per
//! frame through [`Materials3D::set_lighting`].
//!
//! Map-slot assignment (`DrawMesh` binds `maps[i]` to texture unit i and
//! writes the unit to `locs[SHADER_LOC_MAP_ALBEDO + i]`; slots 0..2 are
//! auto-bound by name, 3..6 are pinned manually in [`Materials3D::load`]):
//!
//! ```text
//! ALBEDO(0)/texture0    diffuse
//! METALNESS(1)/texture1 specular map (aka MATERIAL_MAP_SPECULAR)
//! NORMAL(2)/texture2    tangent-space normal map
//! ROUGHNESS(3)/texture3 GAL pack  (r=glow mask, g=alpha mask, b=lightmap)
//! OCCLUSION(4)/texture4 AO lightmap
//! EMISSION(5)/texture5  glow (additive)
//! HEIGHT(6)/texture6    alpha mask (r channel)
//! ```
//!
//! Known deviations from stock Away3D:
//! - `secondaryUVForLightMap` is approximated by sampling the `ao`/`gal`
//!   lightmap channels on UV1 — the OBJ cache carries a single UV set.
//! - The GAL glow `shaderParams.glow` multiplier is fixed at 1
//!   (`fragmentConstants` fc.y — no per-mesh glow override is parsed).
//! - `blendMode="add"`/`materialBothSides` cannot switch raylib's per-draw
//!   blend/cull state; `blend_add` materials render alpha-blended and
//!   `both_sides` renders front faces only.
//! - Point-light attenuation uses a linear `1 - d/fallOff` ramp instead of
//!   Away3D's `(fallOff - d)/(fallOff - radius)` curve (`radius` is
//!   authored 0, so the difference is only the curve shape).

use crate::assets::{MaterialDesc, TextureQuality, TextureSlot};
use raylib::ffi;
use raylib::prelude::*;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// `Settings3D._defaultSunLight` (`LightSettings(point?, visible, color,
/// diffuse, specular, ambientColor, ambient, radius, fallOff, tilt, pan)`).
const SUN_COLOR: Vector3 = Vector3::new(163.0 / 255.0, 1.0, 1.0); // 0xA3FFFF
const SUN_DIFFUSE: f32 = 0.8;
const SUN_SPECULAR: f32 = 1.1;
/// `ambientColor` 0xFF855C * `ambient` 0.5, premultiplied.
const AMBIENT: Vector3 = Vector3::new(0.5, 133.0 / 255.0 * 0.5, 92.0 / 255.0 * 0.5);

/// `Settings3D.heroPositionLight` (0x2E7BFF, radius 0, fallOff 450).
const HERO_COLOR: Vector3 = Vector3::new(46.0 / 255.0, 123.0 / 255.0, 1.0);
const HERO_DIFFUSE: f32 = 0.6;
const HERO_SPECULAR: f32 = 1.5;
const HERO_FALLOFF: f32 = 450.0;

/// `ExtMath.tiltPan2Vector(directionTilt = 100, directionPan = 35, -1)` —
/// the direction the sun's light travels, expressed in raylib space
/// (Away3D z mirrored like `scene3d::rl_point`).
fn sun_direction() -> Vector3 {
    let tilt = 100.0_f32.to_radians();
    let pan = 35.0_f32.to_radians();
    Vector3::new(-tilt.sin() * pan.sin(), -tilt.sin() * pan.cos(), tilt.cos())
}

/// Every `{base}_{slot}_{size}` manifest id the mesh's material wants at
/// `quality` (unfiltered — callers apply `geometry_file_exists` before
/// requesting; `sizeMultiplier` stays 1, the zoom>1.3 bump from
/// `HeroDecorator3D` is not modelled).
pub fn layer_keys(
    geometry: &str,
    material: &MaterialDesc,
    quality: TextureQuality,
) -> Vec<(TextureSlot, String)> {
    MaterialDesc::supported_slots(quality)
        .iter()
        .filter_map(|&slot| {
            material
                .texture_key(slot, geometry, quality, 1)
                .map(|key| (slot, key))
        })
        .collect()
}

/// Deterministic identity of a mesh's produced `Model`:
/// `(geometry, fingerprint)` where the fingerprint covers every input the
/// produced model depends on — resolved layer ids, `specular_strength`,
/// `gloss`, `lit`, `blend_add`, `alpha_blending`, `both_sides` and the
/// texture-quality tier.
pub fn model_key(
    geometry: &str,
    material: &MaterialDesc,
    quality: TextureQuality,
) -> (String, String) {
    let mut hasher = DefaultHasher::new();
    quality.hash(&mut hasher);
    material
        .specular_strength(quality)
        .to_bits()
        .hash(&mut hasher);
    material.gloss_value(quality).to_bits().hash(&mut hasher);
    (
        material.lit,
        material.blend_add,
        material.alpha_blending,
        material.both_sides,
        material.secondary_uv_lightmap,
    )
        .hash(&mut hasher);
    for (slot, key) in layer_keys(geometry, material, quality) {
        (slot as u8).hash(&mut hasher);
        key.hash(&mut hasher);
    }
    (geometry.to_owned(), format!("{:016x}", hasher.finish()))
}

/// Uniform locations on the shared lit program (fetched once at load).
struct LightingLocs {
    sun_dir: i32,
    sun_color: i32,
    sun_diffuse: i32,
    sun_specular: i32,
    ambient: i32,
    hero_pos: i32,
    hero_color: i32,
    hero_falloff: i32,
    hero_diffuse: i32,
    hero_specular: i32,
    view_pos: i32,
}

/// The two shared programs plus the 1x1 fallback textures bound for
/// absent/failed layers (white = multiply/add-identity for mask/ao/specular
/// and diffuse-less meshes, black = additive-identity for glow, flat
/// normal, and a GAL identity with alpha/lightmap channels open and the
/// glow floor at 0).
///
/// Owned here (not per model): `UnloadModel` frees each material's `maps`
/// array but never unloads shared shaders/textures, and this struct is
/// declared *after* `models3d`/`textures3d` in `TextureCache` so models
/// drop before the resources their materials weakly reference.
pub(crate) struct Materials3D {
    lit: Shader,
    unlit: Shader,
    locs: LightingLocs,
    white: Texture2D,
    black: Texture2D,
    flat_normal: Texture2D,
    gal_identity: Texture2D,
}

impl Materials3D {
    pub(crate) fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Option<Self> {
        let mut lit =
            rl.load_shader_from_memory(thread, Some(VERTEX_SHADER), Some(&fragment_shader(false)));
        let mut unlit =
            rl.load_shader_from_memory(thread, Some(VERTEX_SHADER), Some(&fragment_shader(true)));
        if !lit.is_shader_valid() || !unlit.is_shader_valid() {
            log::warn!("materials3d shader compile failed; authored 3D models disabled");
            return None;
        }
        // Slots 3..6 have no auto-bound sampler name — pin them so
        // `DrawMesh`'s `locs[SHADER_LOC_MAP_DIFFUSE + i]` write reaches them.
        for shader in [&mut lit, &mut unlit] {
            for (i, name) in ["texture3", "texture4", "texture5", "texture6"]
                .iter()
                .enumerate()
            {
                let loc = shader.get_shader_location(name);
                shader.locs_mut()[ShaderLocationIndex::SHADER_LOC_MAP_ROUGHNESS as usize + i] = loc;
            }
        }
        let locs = LightingLocs {
            sun_dir: lit.get_shader_location("sunDirection"),
            sun_color: lit.get_shader_location("sunColor"),
            sun_diffuse: lit.get_shader_location("sunDiffuse"),
            sun_specular: lit.get_shader_location("sunSpecular"),
            ambient: lit.get_shader_location("ambientColor"),
            hero_pos: lit.get_shader_location("heroPosition"),
            hero_color: lit.get_shader_location("heroColor"),
            hero_falloff: lit.get_shader_location("heroFallOff"),
            hero_diffuse: lit.get_shader_location("heroDiffuse"),
            hero_specular: lit.get_shader_location("heroSpecular"),
            view_pos: lit.get_shader_location("viewPos"),
        };
        Some(Self {
            lit,
            unlit,
            locs,
            white: solid(rl, thread, Color::WHITE)?,
            black: solid(rl, thread, Color::BLACK)?,
            flat_normal: solid(rl, thread, Color::new(128, 128, 255, 255))?,
            gal_identity: solid(rl, thread, Color::new(0, 255, 255, 255))?,
        })
    }

    fn shader(&self, lit: bool) -> &Shader {
        if lit {
            &self.lit
        } else {
            &self.unlit
        }
    }

    /// Fallback bound for an absent or failed layer: identity under the
    /// shader's ops for that slot (white multiplies, black adds 0, flat
    /// normal keeps `N`, gal keeps alpha/lightmap at 1 and glow floor 0).
    fn fallback(&self, slot: TextureSlot) -> &Texture2D {
        match slot {
            TextureSlot::Normal => &self.flat_normal,
            TextureSlot::Glow => &self.black,
            TextureSlot::Gal => &self.gal_identity,
            _ => &self.white,
        }
    }

    /// Push the scene lighting once per frame — sun + ambient constants and
    /// the hero point light (disabled when `hero_pos` is `None`).
    pub(crate) fn set_lighting(&mut self, camera_pos: Vector3, hero_pos: Option<Vector3>) {
        let l = &self.locs;
        let s = &mut self.lit;
        s.set_shader_value(l.sun_dir, sun_direction());
        s.set_shader_value(l.sun_color, SUN_COLOR);
        s.set_shader_value(l.sun_diffuse, SUN_DIFFUSE);
        s.set_shader_value(l.sun_specular, SUN_SPECULAR);
        s.set_shader_value(l.ambient, AMBIENT);
        s.set_shader_value(l.hero_pos, hero_pos.unwrap_or(Vector3::zero()));
        s.set_shader_value(l.hero_color, HERO_COLOR);
        s.set_shader_value(l.hero_falloff, HERO_FALLOFF);
        s.set_shader_value(
            l.hero_diffuse,
            if hero_pos.is_some() {
                HERO_DIFFUSE
            } else {
                0.0
            },
        );
        s.set_shader_value(
            l.hero_specular,
            if hero_pos.is_some() {
                HERO_SPECULAR
            } else {
                0.0
            },
        );
        s.set_shader_value(l.view_pos, camera_pos);
    }
}

/// `PrefabMaterialManager.assembleMeshMaterial`: assign the shared lit or
/// unlit program to every material of a freshly loaded model, bind each
/// resolved layer (fallbacks for absent/failed layers) and pack the
/// material scalars.
///
/// `maps[SPECULAR]` (= `MATERIAL_MAP_METALNESS`) `.color` gets
/// `specular_strength` in rgb and `gloss | alphaBlending<<7` in alpha —
/// informational only, since this raylib build uploads `colSpecular` from
/// `maps[SHADER_LOC_COLOR_SPECULAR]` (index 13, past the 12-entry maps
/// array). The channel the shader actually decodes is
/// `maps[DIFFUSE].color` → `colDiffuse`: rgb = `strength * 127.5` (÷2 →
/// range 0..2), a low 7 bits = gloss, top bit = `alphaBlending`. The
/// shader therefore never uses `colDiffuse` as a tint.
pub(crate) fn assign(
    model: &mut Model,
    material: &MaterialDesc,
    quality: TextureQuality,
    layers: &[(TextureSlot, String)],
    textures: &HashMap<String, Texture2D>,
    shared: &Materials3D,
) {
    let layer = |slot: TextureSlot| -> &Texture2D {
        layers
            .iter()
            .find(|(s, _)| *s == slot)
            .and_then(|(_, id)| textures.get(id))
            .unwrap_or_else(|| shared.fallback(slot))
    };
    let shader = *shared.shader(material.lit).as_ref();
    let strength = (material.specular_strength(quality).clamp(0.0, 2.0) * 127.5).round() as u8;
    let gloss = material.gloss_value(quality).round().clamp(0.0, 127.0) as u8;
    let params = ffi::Color {
        r: strength,
        g: strength,
        b: strength,
        a: gloss | if material.alpha_blending { 128 } else { 0 },
    };
    for m in model.materials_mut() {
        // SAFETY: `Materials3D` outlives every model it is applied to
        // (field order in `TextureCache`); `WeakShader` does not drop.
        *m.shader_mut() = unsafe { WeakShader::from_raw(shader) };
        m.set_material_texture(
            MaterialMapIndex::MATERIAL_MAP_ALBEDO,
            weak(layer(TextureSlot::Diffuse)),
        );
        m.set_material_texture(
            MaterialMapIndex::MATERIAL_MAP_METALNESS,
            weak(layer(TextureSlot::Specular)),
        );
        m.set_material_texture(
            MaterialMapIndex::MATERIAL_MAP_NORMAL,
            weak(layer(TextureSlot::Normal)),
        );
        m.set_material_texture(
            MaterialMapIndex::MATERIAL_MAP_ROUGHNESS,
            weak(layer(TextureSlot::Gal)),
        );
        m.set_material_texture(
            MaterialMapIndex::MATERIAL_MAP_OCCLUSION,
            weak(layer(TextureSlot::Ao)),
        );
        m.set_material_texture(
            MaterialMapIndex::MATERIAL_MAP_EMISSION,
            weak(layer(TextureSlot::Glow)),
        );
        m.set_material_texture(
            MaterialMapIndex::MATERIAL_MAP_HEIGHT,
            weak(layer(TextureSlot::Alpha)),
        );
        // METALNESS is the SPECULAR slot alias; the ALBEDO slot's color is
        // what actually reaches the shader as `colDiffuse` (see above).
        m.maps_mut()[MaterialMapIndex::MATERIAL_MAP_METALNESS as usize].color = params;
        m.maps_mut()[MaterialMapIndex::MATERIAL_MAP_ALBEDO as usize].color = params;
    }
}

/// Material maps retain non-owning texture handles; the cache owns the
/// strong textures (`textures3d`) and the shared fallbacks.
fn weak(texture: &Texture2D) -> WeakTexture2D {
    // SAFETY: models referencing this texture live in `TextureCache`
    // alongside the owning map.
    unsafe { WeakTexture2D::from_raw(*texture.as_ref()) }
}

fn solid(rl: &mut RaylibHandle, thread: &RaylibThread, color: Color) -> Option<Texture2D> {
    rl.load_texture_from_image(thread, &Image::gen_image_color(1, 1, color))
        .ok()
}

/// GLSL 330 vertex stage. Attribute/uniform names match rlgl's default
/// bindings (`vertexPosition`, `vertexTexCoord`, `vertexNormal`,
/// `vertexColor`, `mvp`, `matModel`, `matNormal`) — `vertexTexCoord2` and
/// `vertexTangent` stay undeclared because the OBJ cache provides neither.
const VERTEX_SHADER: &str = r"#version 330
in vec3 vertexPosition;
in vec2 vertexTexCoord;
in vec3 vertexNormal;
in vec4 vertexColor;

uniform mat4 mvp;
uniform mat4 matModel;
uniform mat4 matNormal;

out vec3 fragPosition;
out vec2 fragTexCoord;
out vec4 fragColor;
out vec3 fragNormal;

void main()
{
    fragPosition = vec3(matModel*vec4(vertexPosition, 1.0));
    fragTexCoord = vertexTexCoord;
    fragColor = vertexColor;
    fragNormal = normalize(mat3(matNormal)*vertexNormal);
    gl_Position = mvp*vec4(vertexPosition, 1.0);
}
";

/// Fragment stage shared by both variants; `UNLIT` strips the lighting
/// block for `light="false"` materials.
fn fragment_shader(unlit: bool) -> String {
    format!(
        "#version 330\n{}{}",
        if unlit { "#define UNLIT\n" } else { "" },
        FRAGMENT_BODY
    )
}

const FRAGMENT_BODY: &str = r"
in vec3 fragPosition;
in vec2 fragTexCoord;
in vec4 fragColor;
in vec3 fragNormal;

uniform vec4 colDiffuse;
uniform sampler2D texture0;
uniform sampler2D texture1;
uniform sampler2D texture2;
uniform sampler2D texture3;
uniform sampler2D texture4;
uniform sampler2D texture5;
uniform sampler2D texture6;

#ifndef UNLIT
uniform vec3 sunDirection;
uniform vec3 sunColor;
uniform float sunDiffuse;
uniform float sunSpecular;
uniform vec3 ambientColor;
uniform vec3 heroPosition;
uniform vec3 heroColor;
uniform float heroFallOff;
uniform float heroDiffuse;
uniform float heroSpecular;
uniform vec3 viewPos;
#endif

out vec4 finalColor;

#ifndef UNLIT
// Tangent frame from screen-space derivatives (thetenthplanet.de technique,
// as in raylib's lighting.fs) — the OBJ meshes carry no tangents.
mat3 cotangentFrame(vec3 N, vec3 p, vec2 uv)
{
    vec3 dp1 = dFdx(p);
    vec3 dp2 = dFdy(p);
    vec2 duv1 = dFdx(uv);
    vec2 duv2 = dFdy(uv);
    vec3 dp2perp = cross(dp2, N);
    vec3 dp1perp = cross(N, dp1);
    vec3 T = dp2perp*duv1.x + dp1perp*duv2.x;
    vec3 B = dp2perp*duv1.y + dp1perp*duv2.y;
    float det = max(dot(T, T), dot(B, B));
    float invmax = det > 1e-12 ? inversesqrt(det) : 0.0;
    return mat3(T*invmax, B*invmax, N);
}
#endif

void main()
{
    vec4 texelDiffuse = texture(texture0, fragTexCoord);
    vec3 albedo = texelDiffuse.rgb*fragColor.rgb;
    float alpha = texelDiffuse.a*fragColor.a;

    // `colDiffuse` packs material scalars (see `apply`).
    float specStrength = colDiffuse.r*2.0;
    float packed = colDiffuse.a*255.0;
    bool alphaBlend = packed > 127.5;
    float gloss = clamp(packed - (alphaBlend ? 128.0 : 0.0), 1.0, 127.0);

    vec4 gal = texture(texture3, fragTexCoord);
    float maskA = texture(texture6, fragTexCoord).r;
    // AlphaMaskMethod + CombinedGALMethod kills (`kill mask - 0.5`,
    // stock-unconditional; gated on alphaBlending here since raylib's
    // per-draw blend state can't switch).
    if (!alphaBlend && min(maskA, gal.g) < 0.5) discard;

#ifdef UNLIT
    vec3 lit = albedo;
#else
    vec3 N = normalize(fragNormal);
    vec3 V = normalize(viewPos - fragPosition);
    vec3 mapped = cotangentFrame(N, -V, fragTexCoord)*(texture(texture2, fragTexCoord).rgb*2.0 - 1.0);
    if (dot(mapped, mapped) > 0.0001) N = normalize(mapped);
    vec3 specMap = texture(texture1, fragTexCoord).rgb;

    vec3 diffuseAcc = ambientColor*albedo;
    vec3 specularAcc = vec3(0.0);
    {
        // DirectionalLight: `-sunDirection` points toward the sun.
        vec3 L = normalize(-sunDirection);
        diffuseAcc += albedo*sunColor*sunDiffuse*max(dot(N, L), 0.0);
        vec3 H = normalize(L + V);
        specularAcc += sunColor*specMap*specStrength*sunSpecular
            *pow(max(dot(N, H), 0.0), gloss);
    }
    if (heroDiffuse + heroSpecular > 0.0) {
        // PointLight with radius 0 — Away3D falloff approximated linearly.
        vec3 toLight = heroPosition - fragPosition;
        float dist = length(toLight);
        vec3 L = toLight/max(dist, 1e-4);
        float atten = clamp(1.0 - dist/heroFallOff, 0.0, 1.0);
        diffuseAcc += albedo*heroColor*heroDiffuse*max(dot(N, L), 0.0)*atten;
        vec3 H = normalize(L + V);
        specularAcc += heroColor*specMap*specStrength*heroSpecular
            *pow(max(dot(N, H), 0.0), gloss)*atten;
    }
    vec3 lit = diffuseAcc + specularAcc;
#endif

    // Post-shading method chain in stock order: AlphaMaskMethod multiply,
    // GlowMethod add, CombinedGALMethod (alpha g / lightmap b / glow-floor
    // r), LightMapMethod AO multiply (approximated on rgb — the stock
    // shader order applies it last as well).
    vec4 outColor = vec4(lit, alpha);
    outColor *= maskA;
    outColor.rgb += texture(texture5, fragTexCoord).rgb;
    outColor *= gal.g;
    outColor *= gal.b;
    outColor.rgb = max(outColor.rgb, texelDiffuse.rgb*gal.rrr);
    outColor.rgb *= texture(texture4, fragTexCoord).rgb;
    finalColor = outColor;
}
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::SlotSource;

    fn material(texture: &str, settings: &str) -> MaterialDesc {
        MaterialDesc {
            texture: texture.to_owned(),
            texture_settings: settings.to_owned(),
            texture_base: texture.to_owned(),
            ..MaterialDesc::default()
        }
    }

    #[test]
    fn model_key_is_stable_for_identical_input() {
        let m = material("ship1", "ship_big");
        assert_eq!(
            model_key("geom", &m, TextureQuality::High),
            model_key("geom", &m, TextureQuality::High)
        );
    }

    #[test]
    fn model_key_differs_across_quality_tiers() {
        let m = material("ship1", "ship_big");
        assert_ne!(
            model_key("geom", &m, TextureQuality::Low),
            model_key("geom", &m, TextureQuality::High)
        );
    }

    #[test]
    fn model_key_differs_across_materials() {
        let a = material("ship1", "ship_big");
        let mut unlit = a.clone();
        unlit.lit = false;
        let mut blend = a.clone();
        blend.alpha_blending = true;
        let mut glossy = a.clone();
        glossy.gloss = Some(25.0);
        let key = model_key("geom", &a, TextureQuality::High);
        assert_ne!(key, model_key("geom", &unlit, TextureQuality::High));
        assert_ne!(key, model_key("geom", &blend, TextureQuality::High));
        assert_ne!(key, model_key("geom", &glossy, TextureQuality::High));
    }

    #[test]
    fn layer_keys_follow_supported_slots() {
        let m = material("ship1", "ship_big");
        let high = layer_keys("geom", &m, TextureQuality::High);
        assert_eq!(high.len(), TextureSlot::ALL.len());
        assert!(high
            .iter()
            .any(|(s, k)| *s == TextureSlot::Diffuse && k == "ship1_diffuse_512"));
        assert!(high
            .iter()
            .any(|(s, k)| *s == TextureSlot::Normal && k == "ship1_normal_512"));
        let low = layer_keys("geom", &m, TextureQuality::Low);
        assert_eq!(low.len(), 3);
        assert!(low.iter().all(|(s, _)| matches!(
            s,
            TextureSlot::Diffuse | TextureSlot::Alpha | TextureSlot::Gal
        )));
        assert!(low
            .iter()
            .any(|(s, k)| *s == TextureSlot::Diffuse && k == "ship1_diffuse_256"));
    }

    #[test]
    fn layer_keys_respect_off_named_and_fallback() {
        let mut m = material("ship1", "ship_big");
        m.slots[TextureSlot::Normal as usize] = SlotSource::Off;
        m.slots[TextureSlot::Specular as usize] = SlotSource::Named("shared".to_owned());
        let layers = layer_keys("geom", &m, TextureQuality::High);
        assert!(!layers.iter().any(|(s, _)| *s == TextureSlot::Normal));
        assert!(layers
            .iter()
            .any(|(s, k)| *s == TextureSlot::Specular && k == "shared_specular_512"));

        // No texture base at all -> geometry id is the Auto base.
        let bare = material("", "ship_big");
        assert!(layer_keys("geom", &bare, TextureQuality::High)
            .iter()
            .any(|(s, k)| *s == TextureSlot::Diffuse && k == "geom_diffuse_512"));

        // `texture="none"` suppresses every layer.
        let none = material("none", "ship_big");
        assert!(layer_keys("geom", &none, TextureQuality::High).is_empty());
    }

    #[test]
    fn sun_direction_matches_away3d_tilt_pan() {
        // ExtMath.tiltPan2Vector(100°, 35°, -1) = (-sin·sin, -sin·cos, -cos)
        // in authored space; z mirrors into raylib space, flipping the sign
        // of the third component: -cos(100°) → +cos(100°) = t.cos().
        let t = 100f32.to_radians();
        let p = 35f32.to_radians();
        let d = sun_direction();
        let expected = (-t.sin() * p.sin(), -t.sin() * p.cos(), t.cos());
        assert!((d.x - expected.0).abs() < 1e-6);
        assert!((d.y - expected.1).abs() < 1e-6);
        assert!((d.z - expected.2).abs() < 1e-6);
        assert!(d.y < 0.0, "sun shines downward");
    }
}
