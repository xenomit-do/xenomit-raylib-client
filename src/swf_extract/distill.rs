//! Distills `swf::Shape` edge soup into per-style vector paths.
//!
//! SWF shape records interleave edges for the left-hand fill style
//! (`fill_style_0`), right-hand fill style (`fill_style_1`) and the active
//! line style, and the edges for a single fill can arrive out of order.
//! This is a slim port of Ruffle's `ShapeConverter` (`render/src/shape_utils.rs`):
//! edges are accumulated into ordered point segments per style, segments are
//! linked into contours by matching endpoints, and `fill_style_0` segments are
//! flipped so they wind the same way as `fill_style_1` contours.

use swf::{FillStyle, LineStyle, ShapeRecord, Twips};
use tiny_skia::PathBuilder;

/// One finished drawing operation, in emission order.
///
/// Commands are expressed in twips (shape-local coordinates).
pub(crate) enum DrawOp<'a> {
    /// Fill `path` with `style`.
    Fill {
        style: &'a FillStyle,
        path: tiny_skia::Path,
    },
    /// Stroke `path` with `style`; `closed` controls end caps.
    Stroke {
        style: &'a LineStyle,
        path: tiny_skia::Path,
    },
}

/// Convert a `swf::Shape` into ordered draw ops (twips coordinates).
pub(crate) fn distill(shape: &swf::Shape) -> Vec<DrawOp<'_>> {
    Converter::new(shape).run()
}

#[derive(Clone, Copy, Debug)]
struct P {
    x: Twips,
    y: Twips,
    bezier_control: bool,
}

/// A continuous run of edges. Fill segments are directed (winding matters);
/// stroke segments are undirected.
#[derive(Clone, Debug)]
struct Segment {
    points: Vec<P>,
}

impl Segment {
    fn new(start: swf::Point<Twips>) -> Self {
        Self {
            points: vec![P {
                x: start.x,
                y: start.y,
                bezier_control: false,
            }],
        }
    }

    fn reset(&mut self, start: swf::Point<Twips>) {
        self.points.clear();
        self.points.push(P {
            x: start.x,
            y: start.y,
            bezier_control: false,
        });
    }

    /// Flash fill paths are dual-sided: `fill_style_1` marks the positive
    /// side, `fill_style_0` the negative. Flipping lets both sides share the
    /// same winding convention.
    fn flip(&mut self) {
        self.points.reverse();
    }

    fn is_empty(&self) -> bool {
        self.points.len() <= 1
    }

    fn start(&self) -> Option<swf::Point<Twips>> {
        self.points.first().map(|p| swf::Point::new(p.x, p.y))
    }

    fn end(&self) -> Option<swf::Point<Twips>> {
        self.points.last().map(|p| swf::Point::new(p.x, p.y))
    }

    fn is_closed(&self) -> bool {
        self.start() == self.end()
    }

    /// Append this segment's outline onto a tiny-skia path builder.
    /// `close` forces contour closure (used for fills).
    fn trace(&self, pb: &mut PathBuilder, close: bool) {
        debug_assert!(!self.is_empty());
        let mut i = self.points.iter();
        let first = match i.next() {
            Some(p) => p,
            None => return,
        };
        pb.move_to(tw(first.x), tw(first.y));
        while let Some(p) = i.next() {
            if p.bezier_control {
                if let Some(anchor) = i.next() {
                    pb.quad_to(tw(p.x), tw(p.y), tw(anchor.x), tw(anchor.y));
                }
            } else {
                pb.line_to(tw(p.x), tw(p.y));
            }
        }
        if close {
            pb.close();
        }
    }
}

/// Points accumulating for a single active style index.
#[derive(Clone, Debug)]
struct ActivePath {
    style_id: u32,
    segment: Segment,
}

impl ActivePath {
    fn new() -> Self {
        Self {
            style_id: 0,
            segment: Segment::new(swf::Point::ZERO),
        }
    }

    fn add_point(&mut self, p: P) {
        self.segment.points.push(p);
    }

    /// Move the pending segment into `pending` and start a fresh one.
    /// `flip` reverses winding (fill_style_0 only).
    fn flush_fill(&mut self, start: swf::Point<Twips>, pending: &mut [PendingPath], flip: bool) {
        if self.style_id > 0 && !self.segment.is_empty() {
            if flip {
                self.segment.flip();
            }
            if let Some(slot) = pending.get_mut(self.style_id as usize - 1) {
                slot.add_segment(self.segment.clone());
            }
        }
        self.segment.reset(start);
    }

    fn flush_stroke(&mut self, start: swf::Point<Twips>, pending: &mut [PendingPath]) {
        if self.style_id > 0 && !self.segment.is_empty() {
            if let Some(slot) = pending.get_mut(self.style_id as usize - 1) {
                slot.segments.push(self.segment.clone());
            }
        }
        self.segment.reset(start);
    }
}

/// Segments accumulating for one style; edges may arrive unordered, so new
/// segments are linked onto existing ones via matching endpoints.
#[derive(Clone, Debug, Default)]
struct PendingPath {
    segments: Vec<Segment>,
}

impl PendingPath {
    fn add_segment(&mut self, mut new_segment: Segment) {
        if new_segment.is_empty() {
            return;
        }
        let mut start_open = true;
        let mut end_open = true;
        let mut i = 0;
        while (start_open || end_open) && i < self.segments.len() {
            let other = &mut self.segments[i];
            if start_open && other.end() == new_segment.start() {
                other.points.extend_from_slice(&new_segment.points[1..]);
                new_segment = self.segments.swap_remove(i);
                start_open = false;
            } else if end_open && new_segment.end() == other.start() {
                std::mem::swap(&mut other.points, &mut new_segment.points);
                other.points.extend_from_slice(&new_segment.points[1..]);
                new_segment = self.segments.swap_remove(i);
                end_open = false;
            } else {
                i += 1;
            }
        }
        self.segments.push(new_segment);
    }
}

struct Converter<'a> {
    records: &'a [ShapeRecord],
    cursor: swf::Point<Twips>,

    fill_styles: &'a [FillStyle],
    line_styles: &'a [LineStyle],

    fill_style0: ActivePath,
    fill_style1: ActivePath,
    line_style: ActivePath,

    fills: Vec<PendingPath>,
    strokes: Vec<PendingPath>,

    ops: Vec<DrawOp<'a>>,
}

impl<'a> Converter<'a> {
    fn new(shape: &'a swf::Shape) -> Self {
        Self {
            records: &shape.shape,
            cursor: swf::Point::ZERO,
            fill_styles: &shape.styles.fill_styles,
            line_styles: &shape.styles.line_styles,
            fill_style0: ActivePath::new(),
            fill_style1: ActivePath::new(),
            line_style: ActivePath::new(),
            fills: vec![PendingPath::default(); shape.styles.fill_styles.len()],
            strokes: vec![PendingPath::default(); shape.styles.line_styles.len()],
            ops: Vec::new(),
        }
    }

    fn run(mut self) -> Vec<DrawOp<'a>> {
        let mut num_fill_styles = self.fill_styles.len() as u32;
        let mut num_line_styles = self.line_styles.len() as u32;
        // `records` borrows the shape data ('a), not `self`, so the active
        // style fields can be mutated freely inside the loop.
        let records = self.records;
        for record in records {
            match record {
                ShapeRecord::StyleChange(sc) => {
                    if let Some(move_to) = &sc.move_to {
                        self.cursor = *move_to;
                        self.flush_paths();
                    }

                    if let Some(styles) = &sc.new_styles {
                        // A new style list marks a new drawing layer.
                        self.flush_layer();
                        self.fill_styles = &styles.fill_styles;
                        self.line_styles = &styles.line_styles;
                        self.fills
                            .resize_with(self.fill_styles.len(), PendingPath::default);
                        self.strokes
                            .resize_with(self.line_styles.len(), PendingPath::default);
                        num_fill_styles = self.fill_styles.len() as u32;
                        num_line_styles = self.line_styles.len() as u32;
                    }

                    if let Some(id) = sc.fill_style_1 {
                        self.fill_style1
                            .flush_fill(self.cursor, &mut self.fills, false);
                        // Style ids are 1-based; 0 = no fill. Out-of-range
                        // ids are treated as no fill.
                        self.fill_style1.style_id = if id <= num_fill_styles { id } else { 0 };
                    }

                    if let Some(id) = sc.fill_style_0 {
                        self.fill_style0
                            .flush_fill(self.cursor, &mut self.fills, true);
                        self.fill_style0.style_id = if id <= num_fill_styles { id } else { 0 };
                    }

                    if let Some(id) = sc.line_style {
                        self.line_style.flush_stroke(self.cursor, &mut self.strokes);
                        self.line_style.style_id = if id <= num_line_styles { id } else { 0 };
                    }
                }
                ShapeRecord::StraightEdge { delta } => {
                    self.cursor += *delta;
                    self.visit_point(false);
                }
                ShapeRecord::CurvedEdge {
                    control_delta,
                    anchor_delta,
                } => {
                    self.cursor += *control_delta;
                    self.visit_point(true);
                    self.cursor += *anchor_delta;
                    self.visit_point(false);
                }
            }
        }

        self.flush_layer();
        self.ops
    }

    fn visit_point(&mut self, bezier_control: bool) {
        let p = P {
            x: self.cursor.x,
            y: self.cursor.y,
            bezier_control,
        };
        if self.fill_style1.style_id > 0 {
            self.fill_style1.add_point(p);
        }
        if self.fill_style0.style_id > 0 {
            self.fill_style0.add_point(p);
        }
        if self.line_style.style_id > 0 {
            self.line_style.add_point(p);
        }
    }

    /// Pen lifted: end the current segments for all active styles.
    fn flush_paths(&mut self) {
        self.fill_style1
            .flush_fill(self.cursor, &mut self.fills, false);
        self.fill_style0
            .flush_fill(self.cursor, &mut self.fills, true);
        self.line_style.flush_stroke(self.cursor, &mut self.strokes);
    }

    /// End-of-layer: emit fill paths first, then strokes on top.
    fn flush_layer(&mut self) {
        self.flush_paths();

        for (i, path) in self.fills.iter_mut().enumerate() {
            if path.segments.is_empty() {
                continue;
            }
            let Some(style) = self.fill_styles.get(i) else {
                continue;
            };
            let mut pb = PathBuilder::new();
            for segment in &path.segments {
                if !segment.is_empty() {
                    segment.trace(&mut pb, true);
                }
            }
            if let Some(p) = pb.finish() {
                self.ops.push(DrawOp::Fill { style, path: p });
            }
            path.segments.clear();
        }

        for (i, path) in self.strokes.iter_mut().enumerate() {
            let Some(style) = self.line_styles.get(i) else {
                continue;
            };
            for segment in &path.segments {
                if segment.is_empty() {
                    continue;
                }
                let mut pb = PathBuilder::new();
                // NO_CLOSE is honored by leaving the contour open.
                segment.trace(&mut pb, segment.is_closed() && style.allow_close());
                if let Some(p) = pb.finish() {
                    self.ops.push(DrawOp::Stroke { style, path: p });
                }
            }
            path.segments.clear();
        }
    }
}

/// Twips -> f32 coordinate for path construction.
#[inline]
fn tw(t: Twips) -> f32 {
    t.get() as f32
}
