//! Software renderer. The crosshair is drawn once per change into a small
//! bitmap; there is no render loop.

use crate::config::Profile;
use tiny_skia::*;

/// Premultiplied RGBA frame.
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

impl Frame {
    /// Premultiplied BGRA, as expected by `UpdateLayeredWindow`.
    pub fn to_bgra(&self) -> Vec<u8> {
        let mut v = self.rgba.clone();
        for px in v.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        v
    }
}

type R = (f32, f32, f32, f32);

/// One drawable part of a crosshair. Every part can be drawn plain or grown
/// by the outline width.
enum Shape {
    Rects { rects: Vec<R>, tf: Transform, aa: bool },
    Stroked { path: Path, width: f32, tf: Transform },
    Disc { r: f32 },
    Ring { r: f32, width: f32 },
}

fn load_image(p: &Profile) -> Option<Pixmap> {
    if !p.use_image || p.image_path.is_empty() {
        return None;
    }
    Pixmap::load_png(&p.image_path).ok()
}

fn outline_px(p: &Profile) -> u32 {
    if p.outline { p.outline_thick } else { 0 }
}

/// Distance from the center to the far end of a rotated square-ish shape.
fn reach(half: f32, rotation: f32) -> f32 {
    if rotation % 90.0 == 0.0 { half } else { half * 1.42 }
}

fn extent(p: &Profile, img: &Option<Pixmap>) -> f32 {
    if p.use_image {
        return img
            .as_ref()
            .map(|i| i.width().max(i.height()) as f32 * p.image_scale / 2.0)
            .unwrap_or(0.0);
    }
    let o = outline_px(p) as f32;
    let mut e: f32 = 0.0;
    if p.cross {
        let len = if p.cross_split { p.cross_len.max(p.cross_len_v) } else { p.cross_len };
        e = e.max((p.cross_gap + len) as f32 + o + p.cross_thick as f32);
    }
    if p.dot {
        e = e.max(p.dot_size as f32 / 2.0 + o + 1.0);
    }
    if p.circle {
        e = e.max(p.circle_radius as f32 + p.circle_thick as f32 / 2.0 + o + 1.0);
    }
    if p.square {
        e = e.max(reach(p.square_size as f32 / 2.0, p.square_rotation) + o + 1.0);
    }
    if p.brackets {
        e = e.max(reach(p.brackets_size as f32 / 2.0, p.brackets_rotation) + p.brackets_thick as f32 + o + 1.0);
    }
    if p.chevron {
        e = e.max((p.chevron_offset + p.chevron_size) as f32 + p.chevron_thick as f32 + o + 1.0);
    }
    e
}

/// A square frame (or a filled square) centered on `c`.
fn square_rects(c: f32, side: u32, thick: u32, filled: bool) -> Vec<R> {
    let s = side as f32;
    let x0 = c - (side / 2) as f32;
    let t = (thick.min(side / 2)) as f32;
    if filled || t * 2.0 >= s {
        return vec![(x0, x0, s, s)];
    }
    vec![(x0, x0, s, t), (x0, x0 + s - t, s, t), (x0, x0 + t, t, s - 2.0 * t), (x0 + s - t, x0 + t, t, s - 2.0 * t)]
}

fn paint(c: [u8; 4], aa: bool) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = aa;
    p
}

fn arms(c: f32, gap: f32, len: f32, len_v: f32, t: f32, t_style: bool) -> Vec<R> {
    let lo = (t as u32 / 2) as f32;
    // An odd thickness occupies the pixel [c, c+1), so the far arms start
    // one pixel later to keep both gaps equal.
    let e = (t as u32 % 2) as f32;
    let mut v = vec![(c + e + gap, c - lo, len, t), (c - gap - len, c - lo, len, t), (c - lo, c + e + gap, t, len_v)];
    if !t_style {
        v.push((c - lo, c - gap - len_v, t, len_v));
    }
    v
}

/// Four corner brackets with rounded corners, like [ ] around the center.
fn brackets_path(c: f32, side: f32, arm: f32, radius: f32) -> Option<Path> {
    let h = side / 2.0;
    let r = radius.min(arm).min(h);
    let mut pb = PathBuilder::new();
    for (sx, sy) in [(-1.0_f32, -1.0_f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let (x, y) = (c + sx * h, c + sy * h);
        pb.move_to(x, y - sy * arm);
        pb.line_to(x, y - sy * r);
        pb.quad_to(x, y, x - sx * r, y);
        pb.line_to(x - sx * arm, y);
    }
    pb.finish()
}

/// A "^" below the center (or a "v" above it when flipped).
fn chevron_path(c: f32, size: f32, offset: f32, flip: bool) -> Option<Path> {
    let y = |v: f32| if flip { 2.0 * c - v } else { v };
    let mut pb = PathBuilder::new();
    pb.move_to(c - size, y(c + offset + size));
    pb.line_to(c, y(c + offset));
    pb.line_to(c + size, y(c + offset + size));
    pb.finish()
}

fn rotation_tf(deg: f32, c: f32) -> Transform {
    if deg != 0.0 { Transform::from_rotate_at(deg, c, c) } else { Transform::identity() }
}

fn draw(pm: &mut Pixmap, shape: &Shape, c: f32, color: [u8; 4], grow: f32) {
    let id = Transform::identity();
    match shape {
        Shape::Rects { rects, tf, aa } => {
            let paint = paint(color, *aa);
            for &(x, y, w, h) in rects {
                if w <= 0.0 || h <= 0.0 {
                    continue;
                }
                if let Some(r) = Rect::from_xywh(x - grow, y - grow, w + 2.0 * grow, h + 2.0 * grow) {
                    pm.fill_path(&PathBuilder::from_rect(r), &paint, FillRule::Winding, *tf, None);
                }
            }
        }
        Shape::Stroked { path, width, tf } => {
            let stroke = Stroke { width: width + 2.0 * grow, line_join: LineJoin::Round, ..Default::default() };
            pm.stroke_path(path, &paint(color, true), &stroke, *tf, None);
        }
        Shape::Disc { r } => {
            let mut pb = PathBuilder::new();
            pb.push_circle(c, c, r + grow);
            if let Some(path) = pb.finish() {
                pm.fill_path(&path, &paint(color, true), FillRule::Winding, id, None);
            }
        }
        Shape::Ring { r, width } => {
            let mut pb = PathBuilder::new();
            pb.push_circle(c, c, *r);
            if let Some(path) = pb.finish() {
                let stroke = Stroke { width: width + 2.0 * grow, ..Default::default() };
                pm.stroke_path(&path, &paint(color, true), &stroke, id, None);
            }
        }
    }
}

fn shapes(p: &Profile, c: f32) -> Vec<Shape> {
    let id = Transform::identity();
    let mut v = Vec::new();
    if p.cross && p.cross_len > 0 {
        let len_v = if p.cross_split { p.cross_len_v } else { p.cross_len };
        v.push(Shape::Rects {
            rects: arms(c, p.cross_gap as f32, p.cross_len as f32, len_v as f32, p.cross_thick as f32, p.t_style),
            tf: rotation_tf(p.rotation, c),
            aa: p.rotation != 0.0,
        });
    }
    if p.square {
        v.push(Shape::Rects {
            rects: square_rects(c, p.square_size, p.square_thick, p.square_filled),
            tf: rotation_tf(p.square_rotation, c),
            aa: p.square_rotation != 0.0,
        });
    }
    if p.dot && !p.dot_round {
        let lo = (p.dot_size / 2) as f32;
        v.push(Shape::Rects { rects: vec![(c - lo, c - lo, p.dot_size as f32, p.dot_size as f32)], tf: id, aa: false });
    }
    if p.dot && p.dot_round {
        v.push(Shape::Disc { r: p.dot_size as f32 / 2.0 });
    }
    if p.circle {
        v.push(Shape::Ring { r: p.circle_radius as f32, width: p.circle_thick as f32 });
    }
    if p.brackets {
        if let Some(path) = brackets_path(c, p.brackets_size as f32, p.brackets_arm as f32, p.brackets_radius as f32) {
            v.push(Shape::Stroked { path, width: p.brackets_thick as f32, tf: rotation_tf(p.brackets_rotation, c) });
        }
    }
    if p.chevron {
        if let Some(path) = chevron_path(c, p.chevron_size as f32, p.chevron_offset as f32, p.chevron_flip) {
            v.push(Shape::Stroked { path, width: p.chevron_thick as f32, tf: id });
        }
    }
    v
}

fn fill_rect(pm: &mut Pixmap, r: R, color: [u8; 4]) {
    draw(pm, &Shape::Rects { rects: vec![r], tf: Transform::identity(), aa: false }, 0.0, color, 0.0);
}

pub fn render(p: &Profile, hint: bool) -> Frame {
    let img = load_image(p);
    let mut size = ((extent(p, &img) * 2.0).ceil() as u32 + 4).clamp(8, 1024);
    if hint {
        size = size.max(160);
    }
    if size % 2 == 1 {
        size += 1;
    }
    let c = (size / 2) as f32;

    let mut base = Pixmap::new(size, size).expect("pixmap");
    if hint {
        // Visible drag area while repositioning.
        let s = size as f32;
        fill_rect(&mut base, (0.0, 0.0, s, s), [0, 0, 0, 110]);
        let white = [255, 255, 255, 200];
        for r in [(0.0, 0.0, s, 1.0), (0.0, s - 1.0, s, 1.0), (0.0, 0.0, 1.0, s), (s - 1.0, 0.0, 1.0, s)] {
            fill_rect(&mut base, r, white);
        }
    }

    // Everything is drawn opaque into a layer, then composited once with
    // the profile opacity so overlapping shapes never double-blend.
    let mut layer = Pixmap::new(size, size).expect("pixmap");
    let id = Transform::identity();

    if let Some(img) = &img {
        let s = p.image_scale;
        let tx = c - img.width() as f32 * s / 2.0;
        let ty = c - img.height() as f32 * s / 2.0;
        let pp = PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() };
        layer.draw_pixmap(0, 0, img.as_ref(), &pp, Transform::from_row(s, 0.0, 0.0, s, tx, ty), None);
    } else if !p.use_image {
        let parts = shapes(p, c);
        // Outlines first so they sit underneath every shape.
        if p.outline && p.outline_thick > 0 {
            for s in &parts {
                draw(&mut layer, s, c, p.outline_color, p.outline_thick as f32);
            }
        }
        for s in &parts {
            draw(&mut layer, s, c, p.color, 0.0);
        }
    }

    let pp = PixmapPaint { opacity: p.opacity.clamp(0.0, 1.0), ..Default::default() };
    base.draw_pixmap(0, 0, layer.as_ref(), &pp, id, None);
    Frame { w: size, h: size, rgba: base.take() }
}

/// 32x32-style tray icon as straight-alpha BGRA.
pub fn tray_icon(size: u32) -> Vec<u8> {
    let mut pm = Pixmap::new(size, size).expect("pixmap");
    let s = size as f32;
    let mut pb = PathBuilder::new();
    pb.push_circle(s / 2.0, s / 2.0, s / 2.0 - 0.5);
    if let Some(path) = pb.finish() {
        pm.fill_path(&path, &paint([24, 26, 31, 255], true), FillRule::Winding, Transform::identity(), None);
    }
    let t = (s / 10.0).round().max(1.0);
    let gap = s * 0.12;
    let len = s * 0.26;
    let c = (size / 2) as f32;
    let rects = arms(c, gap, len, len, t, false);
    draw(&mut pm, &Shape::Rects { rects, tf: Transform::identity(), aa: false }, c, [99, 102, 241, 255], 0.0);
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for px in pm.pixels() {
        let d = px.demultiply();
        out.extend_from_slice(&[d.blue(), d.green(), d.red(), d.alpha()]);
    }
    out
}
