//! Away3D material descriptors parsed from `<mesh>` / root-geometry
//! `<display3D>` recipe nodes — the input `PrefabMaterialManager` gets in
//! the Flash client. The useful output is `texture_key`: the
//! `resources_3d` manifest id `{base}_{slot}_{size}` that
//! `PrefabMaterialManager.assembleTextureResKey` builds per texture layer.

use super::uberlib::{clean_attr, opt_f_attr, TextureQuality, Xml};

/// A texture layer slot of `EntityBasicMaterial` (`supportedTextures`).
/// Discriminant order matches [`TextureSlot::ALL`] and indexes
/// [`MaterialDesc::slots`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureSlot {
    Diffuse,
    Alpha,
    /// CombinedGALMethod texture (glow-alpha-light overlay).
    Gal,
    Normal,
    Specular,
    /// Ambient-occlusion light map.
    Ao,
    Glow,
}

impl TextureSlot {
    pub const ALL: [Self; 7] = [
        Self::Diffuse,
        Self::Alpha,
        Self::Gal,
        Self::Normal,
        Self::Specular,
        Self::Ao,
        Self::Glow,
    ];

    /// Manifest key infix (`{base}_{slot}_{size}`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Diffuse => "diffuse",
            Self::Alpha => "alpha",
            Self::Gal => "gal",
            Self::Normal => "normal",
            Self::Specular => "specular",
            Self::Ao => "ao",
            Self::Glow => "glow",
        }
    }
}

/// Per-slot authored texture-base override (`@{slot}` attribute).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SlotSource {
    /// No attribute — the `@texture`/`@geometry` fallback chain applies.
    #[default]
    Auto,
    /// Authored `="none"` — the layer is suppressed.
    Off,
    /// Authored base id (verbatim, even numeric strings like `"0.2"`).
    Named(String),
}

/// One mesh node's material attributes (`ITextureMaterial.setShaderParams`
/// input). `texture`/`texture_settings` mirror the legacy flat fields on
/// `Mesh3D`/`SceneMesh3D`; the per-slot table drives the other layers.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialDesc {
    /// `@diffuse || @texture` — the legacy diffuse base.
    pub texture: String,
    /// `@tex_settings` — names a `Settings3D` size column.
    pub texture_settings: String,
    /// `@shader` (e.g. `"ship"`, `"uber_ship"`, `"xRay"`) — preserved for
    /// the renderer; irrelevant to key resolution.
    pub shader: String,
    /// Raw `@texture` attribute — `assembleTextureResKey`'s Auto fallback
    /// base for every slot (`@diffuse` is modelled as `slots[Diffuse]`).
    pub(crate) texture_base: String,
    /// Per-slot source, indexed by `slot as usize` ([`TextureSlot::ALL`] order).
    pub slots: [SlotSource; 7],
    pub specularity: Option<f32>,
    pub specularity_low: Option<f32>,
    pub gloss: Option<f32>,
    pub gloss_low: Option<f32>,
    /// `blendMode="add"`.
    pub blend_add: bool,
    /// `alphaBlending="true"`.
    pub alpha_blending: bool,
    /// `materialBothSides="true"`.
    pub both_sides: bool,
    /// `light="false"` marks the material unlit; default lit.
    pub lit: bool,
    /// `secondaryUVForLightMap="true"` — the `ao`/`gal` layers sample UV2.
    pub secondary_uv_lightmap: bool,
}

impl Default for MaterialDesc {
    fn default() -> Self {
        Self {
            texture: String::new(),
            texture_settings: String::new(),
            shader: String::new(),
            texture_base: String::new(),
            slots: Default::default(),
            specularity: None,
            specularity_low: None,
            gloss: None,
            gloss_low: None,
            blend_add: false,
            alpha_blending: false,
            both_sides: false,
            lit: true,
            secondary_uv_lightmap: false,
        }
    }
}

impl MaterialDesc {
    /// Read every material attribute off one `<mesh>` or root-geometry
    /// `<display3D>` node (infallible string work, like stock's `XML.@`
    /// reads — absent attributes are empty strings).
    pub fn parse(node: &Xml) -> Self {
        let texture_base = clean_attr(node.attr("texture"));
        let mut slots: [SlotSource; 7] = Default::default();
        for slot in TextureSlot::ALL {
            // `getTextureName`: "none" suppresses, empty takes the fallback.
            // Slot `Diffuse` reads the `diffuse` attribute here — numeric
            // strings like `alpha="0.2"` stay verbatim `Named` values (they
            // resolve to absent manifest ids, never to opacity).
            slots[slot as usize] = match clean_attr(node.attr(slot.as_str())).as_str() {
                "" => SlotSource::Auto,
                "none" => SlotSource::Off,
                value => SlotSource::Named(value.to_owned()),
            };
        }
        let diffuse = clean_attr(node.attr("diffuse"));
        let texture = if diffuse.is_empty() {
            texture_base.clone()
        } else {
            diffuse
        };
        Self {
            texture,
            texture_settings: clean_attr(node.attr("tex_settings")),
            shader: clean_attr(node.attr("shader")),
            texture_base,
            slots,
            specularity: opt_f_attr(node, "specularity"),
            specularity_low: opt_f_attr(node, "specularityLow"),
            gloss: opt_f_attr(node, "gloss"),
            gloss_low: opt_f_attr(node, "glossLow"),
            blend_add: node.attr("blendMode") == "add",
            alpha_blending: node.attr("alphaBlending") == "true",
            both_sides: node.attr("materialBothSides") == "true",
            lit: node.attr("light") != "false",
            secondary_uv_lightmap: node.attr("secondaryUVForLightMap") == "true",
        }
    }

    /// `assembleTextureResKey` — `{base}_{slot}_{size}`, pure string math:
    /// the caller checks the manifest for presence. `geometry` is the
    /// mesh's `geometry` id (deepest fallback); `size_mult` is
    /// `PrefabMaterialManager.sizeMultiplier` (applied only below 512).
    pub fn texture_key(
        &self,
        slot: TextureSlot,
        geometry: &str,
        quality: TextureQuality,
        size_mult: i32,
    ) -> Option<String> {
        let base = match self.slot_source(slot) {
            SlotSource::Off => None,
            SlotSource::Named(name) if name == "none" => None,
            SlotSource::Named(name) if !name.is_empty() => Some(name.clone()),
            // Auto: getTextureName(@texture, @geometry) — "none" breaks the
            // chain (a `texture="none"` recipe has no layers at all).
            _ if self.texture_base == "none" => None,
            _ if !self.texture_base.is_empty() => Some(self.texture_base.clone()),
            _ if !geometry.is_empty() => Some(geometry.to_owned()),
            _ => None,
        }?;
        let size = texture_size(&self.texture_settings, quality);
        if size <= 0 {
            return None;
        }
        let mult = if size < 512 { size_mult } else { 1 };
        Some(format!(
            "{base}_{}_{}",
            slot.as_str(),
            (size * mult).clamp(128, 1024)
        ))
    }

    /// The authored source of one slot's base id.
    pub fn slot_source(&self, slot: TextureSlot) -> &SlotSource {
        &self.slots[slot as usize]
    }

    /// `EntityBasicMaterial.supportedTextures` at this quality: glow maps
    /// need Medium, the advanced layers (normal/specular/ao) need High.
    pub fn supported_slots(quality: TextureQuality) -> &'static [TextureSlot] {
        match quality {
            TextureQuality::Low => &[TextureSlot::Diffuse, TextureSlot::Alpha, TextureSlot::Gal],
            TextureQuality::Medium => &[
                TextureSlot::Diffuse,
                TextureSlot::Alpha,
                TextureSlot::Gal,
                TextureSlot::Glow,
            ],
            TextureQuality::High => &[
                TextureSlot::Diffuse,
                TextureSlot::Alpha,
                TextureSlot::Gal,
                TextureSlot::Normal,
                TextureSlot::Specular,
                TextureSlot::Ao,
                TextureSlot::Glow,
            ],
        }
    }

    /// `setShaderParams`: `specularity`, falling back to `specularityLow`
    /// below High (which itself defaults to the authored `specularity`).
    pub fn specular_strength(&self, quality: TextureQuality) -> f32 {
        match quality {
            TextureQuality::High => self.specularity,
            _ => self.specularity_low.or(self.specularity),
        }
        .unwrap_or(1.0)
    }

    /// `setShaderParams`: `gloss` / `glossLow` pair, default 50.
    pub fn gloss_value(&self, quality: TextureQuality) -> f32 {
        match quality {
            TextureQuality::High => self.gloss,
            _ => self.gloss_low.or(self.gloss),
        }
        .unwrap_or(50.0)
    }
}

/// `Settings3D.getTextureSize(@tex_settings)` + `updateTexturesSize`: the
/// per-name size column for a quality tier. Unknown/absent names fall back
/// to the `ship` column (`getTextureSize` returns `tex_ships` last).
fn texture_size(settings: &str, quality: TextureQuality) -> i32 {
    match quality {
        TextureQuality::Low => match settings {
            "ship_very_small" | "ship_small" | "drone" => 64,
            "ship_big" | "ship_very_big" => 256,
            "building_small" | "planet_small" => 128,
            "building" | "building_big" | "planet" => 256,
            _ => 128,
        },
        TextureQuality::Medium => match settings {
            "ship_very_small" | "ship_small" | "drone" => 64,
            "ship_big" => 256,
            "ship_very_big" => 512,
            "building_small" | "planet_small" => 256,
            "building" | "building_big" | "planet" => 512,
            _ => 128,
        },
        TextureQuality::High => match settings {
            "ship_very_small" | "ship_small" | "drone" => 128,
            "ship_big" => 512,
            "ship_very_big" => 1024,
            "building_small" | "planet_small" | "building" => 512,
            "building_big" | "planet" => 1024,
            _ => 256,
        },
    }
}
