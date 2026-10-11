//! Software rasterizer: SWF character definitions -> RGBA frames.
//!
//! Rendering model (all vector geometry is built in twips; a tiny-skia
//! `Transform` maps twips -> output pixels at draw time):
//!
//! * Shapes are distilled to per-style paths and filled/stroked directly.
//! * Sprite timelines are simulated once: a display list (`BTreeMap` by
//!   depth) is maintained, `ShowFrame` emits one output frame, and nested
//!   sprite children tick their own timelines per parent frame — this is
//!   what makes 32-frame rotation strips work.
//! * Every placed child is rendered into its own bounds-fitted pixmap, then
//!   composited at an integer offset (vector content is rasterized in the
//!   final space, so nothing is double-sampled).
//! * `ColorTransform`s compose multiplicatively down to leaf characters and
//!   are applied per-pixel before compositing.
//! * `clip_depth` clip layers rasterize to an alpha `Mask` that is applied
//!   to the following siblings inside the clip range.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use anyhow::{anyhow, Result};
use image::RgbaImage;
use swf::{CharacterId, ColorTransform, Depth, FillStyle, Matrix, Rectangle, Tag, Twips};
use tiny_skia::{
    FillRule, FilterQuality, GradientStop, LineCap, LineJoin, Mask, MaskType, Paint, Pixmap,
    PixmapPaint, Point, Shader, SpreadMode, Stroke, Transform,
};

use crate::swf_extract::{bitmap, distill, video, Frame, ParsedLib};

/// Maximum output pixmap dimension (guards against degenerate bounds).
const MAX_DIM: f32 = 4096.0;
/// Recursion cap for nested sprites and cyclic character graphs.
const MAX_DEPTH: u8 = 24;

/// A renderable character definition borrowed from the parsed tag stream.
#[derive(Clone, Copy)]
pub(crate) enum CharDef<'a> {
    Shape(&'a swf::Shape),
    Sprite(&'a swf::Sprite<'a>),
    /// DefineBits / DefineBitsJpeg2-4 payload.
    Bits {
        data: &'a [u8],
        alpha: Option<&'a [u8]>,
        /// DefineBits v1 payloads need `JpegTables` glued on.
        needs_tables: bool,
    },
    Lossless(&'a swf::DefineBitsLossless<'a>),
    /// DefineVideoStream + its encoded VideoFrame payloads.
    Video {
        stream: &'a swf::DefineVideoStream,
        /// `(frame_num, encoded data)` pairs sorted by frame number.
        frames: &'a [(u16, &'a [u8])],
    },
}

type RectT = Rectangle<Twips>;
type DisplayList = BTreeMap<Depth, Placed>;

/// Runtime state of a placed sprite child: its own display list plus how far
/// its timeline has been consumed. Persisted across parent frames so child
/// clips animate on their own timeline.
#[derive(Clone, Default)]
struct SpriteState {
    dl: DisplayList,
    /// Index of the next unprocessed tag in `Sprite::tags`.
    cursor: usize,
    /// ShowFrame tags consumed so far.
    frames_done: u32,
}

/// A display-list entry.
#[derive(Clone)]
struct Placed {
    char_id: CharacterId,
    matrix: Matrix,
    cx: ColorTransform,
    clip_depth: Option<Depth>,
    visible: bool,
    /// PlaceObject `ratio` — for video characters this is the video frame
    /// number to display; also carries morph-shape ratios (unused here).
    ratio: Option<u16>,
    /// Present when `char_id` resolves to a DefineSprite.
    sprite: Option<SpriteState>,
}

pub(crate) struct Renderer<'a> {
    lib: &'a ParsedLib<'a>,
    bitmap_cache: RefCell<HashMap<CharacterId, Option<Rc<Pixmap>>>>,
    /// Decoded video frames per stream id, indexed by frame number.
    video_cache: RefCell<HashMap<CharacterId, Option<Rc<Vec<Option<Rc<Pixmap>>>>>>>,
    warned: RefCell<HashMap<&'static str, bool>>,
}

impl<'a> Renderer<'a> {
    pub fn new(lib: &'a ParsedLib<'a>) -> Self {
        Self {
            lib,
            bitmap_cache: RefCell::new(HashMap::new()),
            video_cache: RefCell::new(HashMap::new()),
            warned: RefCell::new(HashMap::new()),
        }
    }

    /// Top-level entry: rasterize a character id to frames.
    pub fn render_character(&self, id: CharacterId) -> Result<Vec<Frame>> {
        match self.lib.def(id) {
            Some(CharDef::Sprite(sprite)) => self.render_sprite(sprite),
            Some(_) => {
                let inst = Placed {
                    char_id: id,
                    matrix: Matrix::IDENTITY,
                    cx: ColorTransform::IDENTITY,
                    clip_depth: None,
                    visible: true,
                    ratio: None,
                    sprite: None,
                };
                let view = Transform::from_scale(0.05, 0.05);
                let (pm, x, y) = self.render_inst(&inst, view, ColorTransform::IDENTITY, 0)?;
                Ok(vec![finish_frame(pm, x as f32, y as f32)])
            }
            None => Err(anyhow!("unknown or unsupported character {id}")),
        }
    }

    // ------------------------------------------------------------------
    // Sprite timeline simulation
    // ------------------------------------------------------------------

    /// Simulate a sprite timeline once, emitting a `Frame` per ShowFrame.
    fn render_sprite(&self, sprite: &swf::Sprite<'_>) -> Result<Vec<Frame>> {
        let mut dl: DisplayList = BTreeMap::new();
        let mut frames = Vec::new();
        for tag in &sprite.tags {
            match tag {
                Tag::PlaceObject(po) => self.apply_place(&mut dl, po, 0),
                Tag::RemoveObject(r) => self.apply_remove(&mut dl, r),
                Tag::ShowFrame => {
                    frames.push(self.emit_frame(&dl)?);
                    // Per-tick advance: every placed sprite child shows its
                    // next frame on the next output frame.
                    for inst in dl.values_mut() {
                        if let Some(state) = &mut inst.sprite {
                            if let Some(CharDef::Sprite(child)) = self.lib.def(inst.char_id) {
                                self.advance(state, child, 0);
                            }
                        }
                    }
                }
                Tag::FrameLabel(_) | Tag::ScriptLimits { .. } | Tag::SetTabIndex { .. } => {}
                Tag::DoAction(_) | Tag::DoInitAction { .. } | Tag::DoAbc(_) | Tag::DoAbc2(_) => {
                    self.warn_once("actions", "skipping ActionScript tags");
                }
                Tag::SoundStreamBlock(_)
                | Tag::SoundStreamHead(_)
                | Tag::SoundStreamHead2(_)
                | Tag::StartSound(_)
                | Tag::StartSound2 { .. } => {
                    self.warn_once("sound", "skipping sound tags");
                }
                // VideoFrame payloads are decoded up-front per stream id
                // (ParsedLib::video_frames); the timeline only selects the
                // displayed frame via PlaceObject ratios.
                Tag::VideoFrame(_) => {}
                _ => {}
            }
        }
        if frames.is_empty() {
            // Sprite without ShowFrames: emit whatever the display list holds.
            frames.push(self.emit_frame(&dl)?);
        }
        Ok(frames)
    }

    /// Apply a PlaceObject(1-4) tag to a display list.
    fn apply_place(&self, dl: &mut DisplayList, po: &swf::PlaceObject<'_>, depth: u8) {
        if depth > MAX_DEPTH {
            return;
        }
        match po.action {
            swf::PlaceObjectAction::Place(id) => {
                let inst = self.new_instance(id, po, None, depth);
                dl.insert(po.depth, inst);
            }
            swf::PlaceObjectAction::Replace(id) => {
                let old = dl.remove(&po.depth);
                let inst = self.new_instance(id, po, old, depth);
                dl.insert(po.depth, inst);
            }
            swf::PlaceObjectAction::Modify => {
                if let Some(inst) = dl.get_mut(&po.depth) {
                    inst.matrix = po.matrix.unwrap_or(inst.matrix);
                    inst.cx = po.color_transform.unwrap_or(inst.cx);
                    if let Some(v) = po.is_visible {
                        inst.visible = v;
                    }
                    if let Some(r) = po.ratio {
                        inst.ratio = Some(r);
                    }
                }
            }
        }
    }

    fn apply_remove(&self, dl: &mut DisplayList, r: &swf::RemoveObject) {
        // RemoveObject v1 also carries the character id; only remove when it
        // matches the instance actually at that depth.
        match r.character_id {
            Some(id) => {
                if dl.get(&r.depth).map(|i| i.char_id) == Some(id) {
                    dl.remove(&r.depth);
                }
            }
            None => {
                dl.remove(&r.depth);
            }
        }
    }

    /// Instantiate a freshly placed character, inheriting omitted fields from
    /// `old` (PlaceObject2 "replace/move" semantics).
    fn new_instance(
        &self,
        id: CharacterId,
        po: &swf::PlaceObject<'_>,
        old: Option<Placed>,
        depth: u8,
    ) -> Placed {
        let mut inst = Placed {
            char_id: id,
            matrix: po
                .matrix
                .or_else(|| old.as_ref().map(|o| o.matrix))
                .unwrap_or(Matrix::IDENTITY),
            cx: po
                .color_transform
                .or_else(|| old.as_ref().map(|o| o.cx))
                .unwrap_or(ColorTransform::IDENTITY),
            clip_depth: po.clip_depth,
            visible: po.is_visible.unwrap_or(true),
            ratio: po.ratio.or_else(|| old.as_ref().and_then(|o| o.ratio)),
            sprite: None,
        };
        if let Some(old) = old {
            // A clip replaced by the same character keeps running.
            if old.char_id == id && old.sprite.is_some() {
                inst.sprite = old.sprite;
                return inst;
            }
        }
        if let Some(CharDef::Sprite(sprite)) = self.lib.def(id) {
            // A freshly placed MovieClip starts on its first frame.
            let mut state = SpriteState::default();
            self.advance(&mut state, sprite, depth + 1);
            inst.sprite = Some(state);
        }
        inst
    }

    /// Advance a sprite instance by one timeline frame: feed control tags up
    /// to and including the next ShowFrame. Wraps to frame 0 at the end.
    fn advance(&self, state: &mut SpriteState, sprite: &swf::Sprite<'_>, depth: u8) {
        if depth > MAX_DEPTH {
            return;
        }
        let num_frames = sprite.num_frames.max(1) as u32;
        if state.frames_done >= num_frames {
            // Loop: restart the timeline.
            state.dl.clear();
            state.cursor = 0;
            state.frames_done = 0;
        }
        while state.cursor < sprite.tags.len() {
            let tag = &sprite.tags[state.cursor];
            state.cursor += 1;
            match tag {
                Tag::PlaceObject(po) => self.apply_place(&mut state.dl, po, depth),
                Tag::RemoveObject(r) => self.apply_remove(&mut state.dl, r),
                Tag::ShowFrame => {
                    state.frames_done += 1;
                    // This sprite's own children animate in step with it.
                    for inst in state.dl.values_mut() {
                        if let Some(child_state) = &mut inst.sprite {
                            if let Some(CharDef::Sprite(child)) = self.lib.def(inst.char_id) {
                                self.advance(child_state, child, depth + 1);
                            }
                        }
                    }
                    break;
                }
                _ => {}
            }
        }
        // No more ShowFrames: treat the tail of the timeline as static.
    }

    // ------------------------------------------------------------------
    // Bounds
    // ------------------------------------------------------------------

    /// Local (twips) bounds of a character. `state` carries a sprite child's
    /// current display list.
    fn char_bounds(
        &self,
        id: CharacterId,
        state: Option<&SpriteState>,
        depth: u8,
    ) -> Option<RectT> {
        if depth > MAX_DEPTH {
            return None;
        }
        match self.lib.def(id)? {
            CharDef::Shape(shape) => {
                if shape.edge_bounds.is_valid() {
                    Some(shape.edge_bounds)
                } else if shape.shape_bounds.is_valid() {
                    Some(shape.shape_bounds)
                } else {
                    None
                }
            }
            CharDef::Sprite(_) => {
                let state = state?;
                let mut bounds = RectT::INVALID;
                for inst in state.dl.values() {
                    if !inst.visible {
                        continue;
                    }
                    if let Some(b) = self.inst_bounds(inst, depth + 1) {
                        bounds = bounds.union(&b);
                    }
                }
                bounds.is_valid().then_some(bounds)
            }
            CharDef::Bits { .. } | CharDef::Lossless(_) => {
                let pm = self.bitmap(id)?;
                Some(RectT {
                    x_min: Twips::ZERO,
                    y_min: Twips::ZERO,
                    x_max: Twips::from_pixels_i32(pm.width() as i32),
                    y_max: Twips::from_pixels_i32(pm.height() as i32),
                })
            }
            CharDef::Video { stream, .. } => Some(RectT {
                x_min: Twips::ZERO,
                y_min: Twips::ZERO,
                x_max: Twips::from_pixels_i32(stream.width as i32),
                y_max: Twips::from_pixels_i32(stream.height as i32),
            }),
        }
    }

    /// Bounds of a placed instance in its parent's twips space.
    fn inst_bounds(&self, inst: &Placed, depth: u8) -> Option<RectT> {
        let b = self.char_bounds(inst.char_id, inst.sprite.as_ref(), depth)?;
        Some(transform_rect(&b, &inst.matrix))
    }

    /// Bounds of a whole display list (the sprite's own space).
    fn dl_bounds(&self, dl: &DisplayList) -> Option<RectT> {
        let mut bounds = RectT::INVALID;
        for inst in dl.values() {
            if !inst.visible {
                continue;
            }
            if let Some(b) = self.inst_bounds(inst, 0) {
                bounds = bounds.union(&b);
            }
        }
        bounds.is_valid().then_some(bounds)
    }

    // ------------------------------------------------------------------
    // Frame emission
    // ------------------------------------------------------------------

    /// Render the current display list into one output `Frame`.
    fn emit_frame(&self, dl: &DisplayList) -> Result<Frame> {
        let bounds = self.dl_bounds(dl).unwrap_or(RectT {
            x_min: Twips::ZERO,
            y_min: Twips::ZERO,
            x_max: Twips::ONE_PX,
            y_max: Twips::ONE_PX,
        });
        let min_px = (
            bounds.x_min.to_pixels() as f32,
            bounds.y_min.to_pixels() as f32,
        );
        let w = size_px(&bounds.x_min, &bounds.x_max);
        let h = size_px(&bounds.y_min, &bounds.y_max);
        let mut pm = Pixmap::new(w, h).ok_or_else(|| anyhow!("frame pixmap alloc failed"))?;

        // twips -> px: subtract the bounds origin, scale by 1/20.
        let view = Transform::from_scale(0.05, 0.05).pre_concat(Transform::from_translate(
            -bounds.x_min.get() as f32,
            -bounds.y_min.get() as f32,
        ));
        self.render_dl(&mut pm, dl, view, ColorTransform::IDENTITY, 0);
        Ok(finish_frame(pm, min_px.0, min_px.1))
    }

    /// Draw a display list into `pm`. `view` maps the list's twips space to
    /// `pm` pixels.
    fn render_dl(
        &self,
        pm: &mut Pixmap,
        dl: &DisplayList,
        view: Transform,
        acc_cx: ColorTransform,
        depth: u8,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        // Active clip mask: (alpha mask, clip_end_depth).
        let mut clip: Option<(Mask, Depth)> = None;
        for (d, inst) in dl {
            if let Some((_, until)) = &clip {
                if *d > *until {
                    clip = None;
                }
            }
            if !inst.visible {
                continue;
            }
            if let Some(clip_end) = inst.clip_depth {
                // Clip layer: its alpha defines the mask for siblings at
                // depths (d, clip_end]. The clip content itself is not drawn.
                if let Some(mut layer) = Pixmap::new(pm.width(), pm.height()) {
                    let layer_view = view.pre_concat(ts(&inst.matrix));
                    self.draw_inst_content(
                        &mut layer,
                        inst,
                        layer_view,
                        acc_cx * inst.cx,
                        depth + 1,
                    );
                    clip = Some((Mask::from_pixmap(layer.as_ref(), MaskType::Alpha), clip_end));
                }
                continue;
            }
            let acc2 = acc_cx * inst.cx;
            match self.render_inst(inst, view, acc2, depth + 1) {
                Ok((cpm, x, y)) => {
                    pm.draw_pixmap(
                        x,
                        y,
                        cpm.as_ref(),
                        &PIXMAP_PAINT,
                        Transform::identity(),
                        clip.as_ref().map(|(m, _)| m),
                    );
                }
                Err(e) => log::debug!("child render failed (char {}): {e}", inst.char_id),
            }
        }
    }

    /// Render a placed instance into a fresh bounds-fitted pixmap; returns
    /// the pixmap and its top-left offset in `view` pixel space.
    fn render_inst(
        &self,
        inst: &Placed,
        view: Transform,
        acc_cx: ColorTransform,
        depth: u8,
    ) -> Result<(Pixmap, i32, i32)> {
        let fallback = RectT {
            x_min: Twips::ZERO,
            y_min: Twips::ZERO,
            x_max: Twips::ONE_PX,
            y_max: Twips::ONE_PX,
        };
        let b = self.char_bounds(inst.char_id, inst.sprite.as_ref(), depth);
        let b = b.filter(|b| b.is_valid()).unwrap_or(fallback);
        // inst-local twips -> view px.
        let t = view.pre_concat(ts(&inst.matrix));
        let r = transform_rect_f32(&b, t);

        let mut x0 = finite_or(r.0, 0.0).floor();
        let mut y0 = finite_or(r.1, 0.0).floor();
        let mut w = (finite_or(r.2, x0 + 1.0).ceil() - x0).max(1.0);
        let mut h = (finite_or(r.3, y0 + 1.0).ceil() - y0).max(1.0);
        if w > MAX_DIM {
            w = MAX_DIM;
        }
        if h > MAX_DIM {
            h = MAX_DIM;
        }
        if !x0.is_finite() {
            x0 = 0.0;
        }
        if !y0.is_finite() {
            y0 = 0.0;
        }

        let mut pm =
            Pixmap::new(w as u32, h as u32).ok_or_else(|| anyhow!("child pixmap alloc failed"))?;
        let child_view = Transform::from_translate(-x0, -y0).pre_concat(t);
        self.draw_inst_content(&mut pm, inst, child_view, acc_cx, depth);
        Ok((pm, x0 as i32, y0 as i32))
    }

    /// Draw `inst`'s content into `pm` with `view` (inst-local twips -> pm
    /// px). `acc_cx` already includes `inst.cx` — applied here at leaf level,
    /// or passed down for sprites.
    fn draw_inst_content(
        &self,
        pm: &mut Pixmap,
        inst: &Placed,
        view: Transform,
        acc_cx: ColorTransform,
        depth: u8,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        match self.lib.def(inst.char_id) {
            Some(CharDef::Shape(shape)) => {
                self.draw_shape(pm, shape, view);
                apply_cx(pm, acc_cx);
            }
            Some(CharDef::Bits { .. }) | Some(CharDef::Lossless(_)) => {
                if let Some(bm) = self.bitmap(inst.char_id) {
                    // Bitmap char space: 1 px = 20 twips.
                    let t = view.pre_concat(Transform::from_scale(20.0, 20.0));
                    pm.draw_pixmap(0, 0, Pixmap::as_ref(&bm), &PIXMAP_PAINT, t, None);
                }
                apply_cx(pm, acc_cx);
            }
            Some(CharDef::Video { .. }) => {
                if let Some(frame) = self.video_frame(inst.char_id, inst.ratio) {
                    // Video char space: 1 px = 20 twips, same as bitmaps.
                    let t = view.pre_concat(Transform::from_scale(20.0, 20.0));
                    pm.draw_pixmap(0, 0, Pixmap::as_ref(&frame), &PIXMAP_PAINT, t, None);
                }
                apply_cx(pm, acc_cx);
            }
            Some(CharDef::Sprite(_)) => {
                if let Some(state) = &inst.sprite {
                    self.render_dl(pm, &state.dl, view, acc_cx, depth);
                }
            }
            None => {}
        }
    }

    /// Rasterize a shape's draw ops.
    fn draw_shape(&self, pm: &mut Pixmap, shape: &swf::Shape, view: Transform) {
        let fill_rule = if shape.flags.contains(swf::ShapeFlag::NON_ZERO_WINDING_RULE) {
            FillRule::Winding
        } else {
            FillRule::EvenOdd
        };
        for op in distill::distill(shape) {
            match op {
                distill::DrawOp::Fill { style, path } => {
                    self.draw_fill(pm, &path, style, fill_rule, view);
                }
                distill::DrawOp::Stroke { style, path } => {
                    self.draw_stroke(pm, &path, style, view);
                }
            }
        }
    }

    /// Build a `Paint` for a SWF fill style and run `f` with it.
    ///
    /// Bitmap-fill shaders borrow the decoded bitmap from the cache, so the
    /// paint must be used inside this call (its lifetime is bound to the
    /// local `Rc` handle).
    fn with_paint<R>(&self, style: &FillStyle, f: impl FnOnce(&Paint) -> R) -> Option<R> {
        if let FillStyle::Bitmap {
            id,
            matrix,
            is_smoothed,
            is_repeating,
        } = style
        {
            let bm = self.bitmap(*id)?;
            // The SWF bitmap fill matrix maps bitmap pixels -> shape twips
            // (a=20 means 1 px = 20 twips); tiny-skia's pattern transform
            // uses the same shader-space -> fill-space convention.
            let t = ts(matrix);
            let paint = Paint {
                shader: tiny_skia::Pattern::new(
                    Pixmap::as_ref(&bm),
                    if *is_repeating {
                        SpreadMode::Repeat
                    } else {
                        SpreadMode::Pad
                    },
                    if *is_smoothed {
                        FilterQuality::Bilinear
                    } else {
                        FilterQuality::Nearest
                    },
                    1.0,
                    t,
                ),
                blend_mode: tiny_skia::BlendMode::default(),
                anti_alias: true,
                force_hq_pipeline: false,
            };
            return Some(f(&paint));
        }
        let paint = self.fill_paint(style)?;
        Some(f(&paint))
    }

    /// Fill `path` with a SWF fill style.
    fn draw_fill(
        &self,
        pm: &mut Pixmap,
        path: &tiny_skia::Path,
        style: &FillStyle,
        fill_rule: FillRule,
        view: Transform,
    ) {
        self.with_paint(style, |paint| {
            pm.fill_path(path, paint, fill_rule, view, None)
        });
    }

    /// Stroke `path` with a SWF line style (stroke width in twips; `view`
    /// maps twips -> px, so the width scales with the shape).
    fn draw_stroke(
        &self,
        pm: &mut Pixmap,
        path: &tiny_skia::Path,
        style: &swf::LineStyle,
        view: Transform,
    ) {
        let stroke = Stroke {
            width: style.width().get() as f32,
            miter_limit: match style.join_style() {
                swf::LineJoinStyle::Miter(limit) => limit.to_f32().max(1.0),
                _ => 4.0,
            },
            line_cap: match style.start_cap() {
                swf::LineCapStyle::Round => LineCap::Round,
                swf::LineCapStyle::Square => LineCap::Square,
                swf::LineCapStyle::None => LineCap::Butt,
            },
            line_join: match style.join_style() {
                swf::LineJoinStyle::Round => LineJoin::Round,
                swf::LineJoinStyle::Bevel => LineJoin::Bevel,
                swf::LineJoinStyle::Miter(_) => LineJoin::Miter,
            },
            dash: None,
        };
        // Line styles may carry any fill style (including gradients/bitmap).
        self.with_paint(style.fill_style(), |paint| {
            pm.stroke_path(path, paint, &stroke, view, None)
        });
    }

    // ------------------------------------------------------------------
    // Paints
    // ------------------------------------------------------------------

    /// Build a tiny-skia `Paint` for a non-bitmap SWF fill style.
    ///
    /// Gradient shaders get their SWF matrix as the shader-space -> twips
    /// transform (twips domain, matching path coordinates). Bitmap fills are
    /// handled by [`Self::with_paint`] because their shader borrows the
    /// decoded bitmap.
    fn fill_paint(&self, style: &FillStyle) -> Option<Paint<'static>> {
        let shader = match style {
            FillStyle::Color(c) => {
                Shader::SolidColor(tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a))
            }
            FillStyle::LinearGradient(g) => tiny_skia::LinearGradient::new(
                // SWF gradient space spans x = [-16384, +16384].
                Point::from_xy(-16384.0, 0.0),
                Point::from_xy(16384.0, 0.0),
                gradient_stops(&g.records),
                spread(g.spread),
                ts(&g.matrix),
            )?,
            FillStyle::RadialGradient(g) => tiny_skia::RadialGradient::new(
                Point::from_xy(0.0, 0.0),
                Point::from_xy(0.0, 0.0),
                16384.0,
                gradient_stops(&g.records),
                spread(g.spread),
                ts(&g.matrix),
            )?,
            FillStyle::FocalGradient {
                gradient: g,
                focal_point,
            } => {
                // Focal point is a fraction of the gradient radius along the
                // gradient-space x axis. tiny-skia's radial shader is a
                // two-point conical (start = focal point, end = circle
                // center), which models it directly.
                let f = focal_point.to_f32() * 16384.0;
                tiny_skia::RadialGradient::new(
                    Point::from_xy(f, 0.0),
                    Point::from_xy(0.0, 0.0),
                    16384.0,
                    gradient_stops(&g.records),
                    spread(g.spread),
                    ts(&g.matrix),
                )?
            }
            FillStyle::Bitmap { .. } => return None,
        };
        Some(Paint {
            shader,
            blend_mode: tiny_skia::BlendMode::default(),
            anti_alias: true,
            force_hq_pipeline: false,
        })
    }

    /// Decoded bitmap character cache.
    fn bitmap(&self, id: CharacterId) -> Option<Rc<Pixmap>> {
        if let Some(hit) = self.bitmap_cache.borrow().get(&id) {
            return hit.clone();
        }
        let decoded = match self.lib.def(id) {
            Some(CharDef::Bits {
                data,
                alpha,
                needs_tables,
            }) => bitmap::decode_bits(data, alpha, self.lib.jpeg_tables, needs_tables).ok(),
            Some(CharDef::Lossless(tag)) => bitmap::decode_lossless(tag).ok(),
            _ => None,
        }
        .map(Rc::new);
        self.bitmap_cache.borrow_mut().insert(id, decoded.clone());
        decoded
    }

    /// Decoded video stream cache: character id -> frames by frame number.
    /// Decoding happens once per stream and is reused by every instance.
    fn video_frames(&self, id: CharacterId) -> Option<Rc<Vec<Option<Rc<Pixmap>>>>> {
        if let Some(hit) = self.video_cache.borrow().get(&id) {
            return hit.clone();
        }
        let decoded = match self.lib.def(id) {
            Some(CharDef::Video { stream, frames }) => Some(Rc::new(
                video::decode_stream(stream.codec, stream.width, stream.height, frames)
                    .into_iter()
                    .map(|f| f.map(Rc::new))
                    .collect(),
            )),
            _ => None,
        };
        self.video_cache.borrow_mut().insert(id, decoded.clone());
        decoded
    }

    /// The decoded video frame selected by `ratio`; falls back to the
    /// nearest earlier decoded frame when the requested index is absent
    /// or out of range (VP6 gaps / stream shorter than the ratio).
    fn video_frame(&self, id: CharacterId, ratio: Option<u16>) -> Option<Rc<Pixmap>> {
        let frames = self.video_frames(id)?;
        let want = ratio.unwrap_or(0) as usize;
        (0..=want)
            .rev()
            .find_map(|i| frames.get(i).and_then(|f| f.clone()))
    }

    fn warn_once(&self, kind: &'static str, msg: &str) {
        let mut w = self.warned.borrow_mut();
        if !w.contains_key(kind) {
            w.insert(kind, true);
            log::debug!("swf_extract: {msg}");
        }
    }
}

// ----------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------

const PIXMAP_PAINT: PixmapPaint = PixmapPaint {
    opacity: 1.0,
    blend_mode: tiny_skia::BlendMode::SourceOver,
    quality: FilterQuality::Bilinear,
};

/// swf::Matrix (twips domain) -> tiny_skia::Transform.
fn ts(m: &Matrix) -> Transform {
    Transform::from_row(
        m.a.to_f32(),
        m.b.to_f32(),
        m.c.to_f32(),
        m.d.to_f32(),
        m.tx.get() as f32,
        m.ty.get() as f32,
    )
}

/// Rectangle<Twips> transformed by a swf::Matrix (corner union).
fn transform_rect(b: &RectT, m: &Matrix) -> RectT {
    let mut out = RectT::INVALID;
    for (x, y) in [
        (b.x_min, b.y_min),
        (b.x_max, b.y_min),
        (b.x_min, b.y_max),
        (b.x_max, b.y_max),
    ] {
        let p = *m * swf::Point::new(x, y);
        out = out.encompass(p);
    }
    out
}

/// Rectangle<Twips> transformed by a tiny_skia::Transform -> f32 px rect
/// (x0, y0, x1, y1).
fn transform_rect_f32(b: &RectT, t: Transform) -> (f32, f32, f32, f32) {
    let mut x0 = f32::INFINITY;
    let mut y0 = f32::INFINITY;
    let mut x1 = f32::NEG_INFINITY;
    let mut y1 = f32::NEG_INFINITY;
    for (x, y) in [
        (b.x_min.get(), b.y_min.get()),
        (b.x_max.get(), b.y_min.get()),
        (b.x_min.get(), b.y_max.get()),
        (b.x_max.get(), b.y_max.get()),
    ] {
        let mut p = Point::from_xy(x as f32, y as f32);
        t.map_point(&mut p);
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    (x0, y0, x1, y1)
}

/// Pixel extent of a twips range, clamped to [1, MAX_DIM] (i64 math, no
/// overflow on degenerate rects).
fn size_px(min: &Twips, max: &Twips) -> u32 {
    let tw = (max.get() as i64 - min.get() as i64).max(0);
    ((tw as f64 / 20.0).ceil() as u32).clamp(1, MAX_DIM as u32)
}

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        default
    }
}

fn spread(s: swf::GradientSpread) -> SpreadMode {
    match s {
        swf::GradientSpread::Pad => SpreadMode::Pad,
        swf::GradientSpread::Reflect => SpreadMode::Reflect,
        swf::GradientSpread::Repeat => SpreadMode::Repeat,
    }
}

/// SWF gradient records -> tiny-skia stops. Ratios 0..=255 map to 0..=1;
/// ordering is preserved (the shader pads the [0,1] ends itself).
fn gradient_stops(records: &[swf::GradientRecord]) -> Vec<GradientStop> {
    records
        .iter()
        .map(|r| {
            GradientStop::new(
                r.ratio as f32 / 255.0,
                tiny_skia::Color::from_rgba8(r.color.r, r.color.g, r.color.b, r.color.a),
            )
        })
        .collect()
}

/// Apply a composed color transform to a leaf pixmap, per-pixel, in straight
/// alpha space (premultiplied -> cx -> premultiplied).
fn apply_cx(pm: &mut Pixmap, cx: ColorTransform) {
    if cx == ColorTransform::IDENTITY {
        return;
    }
    for p in pm.pixels_mut() {
        if p.alpha() == 0 {
            continue;
        }
        let c = p.demultiply();
        let out = &cx
            * swf::Color {
                r: c.red(),
                g: c.green(),
                b: c.blue(),
                a: c.alpha(),
            };
        *p = tiny_skia::ColorU8::from_rgba(out.r, out.g, out.b, out.a).premultiply();
    }
}

/// Convert a premultiplied pixmap to a straight-alpha `RgbaImage`, trimming
/// fully transparent borders. `(ox, oy)` is the pixmap's top-left in symbol
/// pixel space; `Frame::offset` gets the trimmed top-left.
fn finish_frame(pm: Pixmap, ox: f32, oy: f32) -> Frame {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let mut straight = Vec::with_capacity(w * h * 4);
    for p in pm.pixels() {
        let c = p.demultiply();
        straight.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }

    // Alpha bounding box.
    let mut x0 = w;
    let mut y0 = h;
    let mut x1 = 0usize;
    let mut y1 = 0usize;
    for y in 0..h {
        for x in 0..w {
            if straight[(y * w + x) * 4 + 3] != 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }

    if x0 >= x1 || y0 >= y1 {
        // Fully transparent frame.
        let image =
            RgbaImage::from_raw(1, 1, vec![0, 0, 0, 0]).unwrap_or_else(|| RgbaImage::new(1, 1));
        return Frame {
            image,
            offset: (ox, oy),
        };
    }

    let (cw, ch) = (x1 - x0, y1 - y0);
    let mut cropped = vec![0u8; cw * ch * 4];
    for (row, dst) in cropped.chunks_exact_mut(cw * 4).enumerate() {
        let src = &straight[((y0 + row) * w + x0) * 4..((y0 + row) * w + x0 + cw) * 4];
        dst.copy_from_slice(src);
    }
    let image = RgbaImage::from_raw(cw as u32, ch as u32, cropped)
        .unwrap_or_else(|| RgbaImage::new(cw as u32, ch as u32));
    Frame {
        image,
        offset: (ox + x0 as f32, oy + y0 as f32),
    }
}
