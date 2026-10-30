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

fn load_image(p: &Profile) -> Option<Pixmap> {
    if !p.use_image || p.image_path.is_empty() {
        return None;
    }
    Pixmap::load_png(&p.image_path).ok()
}

fn outline_px(p: &Profile) -> u32 {
    if p.outline { p.outline_thick } else { 0 }
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
        e = e.max((p.cross_gap + p.cross_len) as f32 + o + p.cross_thick as f32);
    }
    if p.dot {
        e = e.max(p.dot_size as f32 / 2.0 + o + 1.0);
    }
    if p.circle {
        e = e.max(p.circle_radius as f32 + p.circle_thick as f32 / 2.0 + o + 1.0);
    }
    if p.square {
        e = e.max(p.square_size as f32 / 2.0 + o + 1.0);
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

type R = (f32, f32, f32, f32);

fn arms(c: f32, gap: f32, len: f32, t: f32, t_style: bool) -> Vec<R> {
    let lo = (t as u32 / 2) as f32;
    // An odd thickness occupies the pixel [c, c+1), so the far arms start
    // one pixel later to keep both gaps equal.
    let e = (t as u32 % 2) as f32;
    let mut v = vec![(c + e + gap, c - lo, len, t), (c - gap - len, c - lo, len, t), (c - lo, c + e + gap, t, len)];
    if !t_style {
        v.push((c - lo, c - gap - len, t, len));
    }
    v
}

fn fill_rects(pm: &mut Pixmap, rects: &[R], grow: f32, color: [u8; 4], tf: Transform, aa: bool) {
    let paint = paint(color, aa);
    for &(x, y, w, h) in rects {
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        if let Some(r) = Rect::from_xywh(x - grow, y - grow, w + 2.0 * grow, h + 2.0 * grow) {
            pm.fill_path(&PathBuilder::from_rect(r), &paint, FillRule::Winding, tf, None);
        }
    }
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
        fill_rects(&mut base, &[(0.0, 0.0, s, s)], 0.0, [0, 0, 0, 110], Transform::identity(), false);
        let white = [255, 255, 255, 200];
        fill_rects(
            &mut base,
            &[(0.0, 0.0, s, 1.0), (0.0, s - 1.0, s, 1.0), (0.0, 0.0, 1.0, s), (s - 1.0, 0.0, 1.0, s)],
            0.0,
            white,
            Transform::identity(),
            false,
        );
    }

    // Everything is drawn opaque into a layer, then composited once with
    // the profile opacity so overlapping shapes never double-blend.
    let mut layer = Pixmap::new(size, size).expect("pixmap");
    let id = Transform::identity();
    let o = outline_px(p) as f32;

    if let Some(img) = &img {
        let s = p.image_scale;
        let tx = c - img.width() as f32 * s / 2.0;
        let ty = c - img.height() as f32 * s / 2.0;
        let pp = PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() };
        layer.draw_pixmap(0, 0, img.as_ref(), &pp, Transform::from_row(s, 0.0, 0.0, s, tx, ty), None);
    } else if !p.use_image {
        let cross_tf = if p.rotation != 0.0 { Transform::from_rotate_at(p.rotation, c, c) } else { id };
        let cross_aa = p.rotation != 0.0;
        let cross = if p.cross && p.cross_len > 0 {
            arms(c, p.cross_gap as f32, p.cross_len as f32, p.cross_thick as f32, p.t_style)
        } else {
            vec![]
        };
        let dot_sq = if p.dot && !p.dot_round {
            let lo = (p.dot_size / 2) as f32;
            vec![(c - lo, c - lo, p.dot_size as f32, p.dot_size as f32)]
        } else {
            vec![]
        };
        let square = if p.square { square_rects(c, p.square_size, p.square_thick, p.square_filled) } else { vec![] };

        let circle =|pm: &mut Pixmap, r: f32, color: [u8; 4], width: Option<f32>| {
            let mut pb = PathBuilder::new();
            pb.push_circle(c, c, r);
            if let Some(path) = pb.finish() {
                match width {
                    Some(w) => {
                        let stroke = Stroke { width: w, ..Default::default() };
                        pm.stroke_path(&path, &paint(color, true), &stroke, id, None);
                    }
                    None => pm.fill_path(&path, &paint(color, true), FillRule::Winding, id, None),
                }
            }
        };

        // Outlines first so they sit underneath every shape.
        if p.outline && p.outline_thick > 0 {
            let oc = p.outline_color;
            fill_rects(&mut layer, &cross, o, oc, cross_tf, cross_aa);
            fill_rects(&mut layer, &dot_sq, o, oc, id, false);
            fill_rects(&mut layer, &square, o, oc, id, false);
            if p.dot && p.dot_round {
                circle(&mut layer, p.dot_size as f32 / 2.0 + o, oc, None);
            }
            if p.circle {
                circle(&mut layer, p.circle_radius as f32, oc, Some(p.circle_thick as f32 + 2.0 * o));
            }
        }
        fill_rects(&mut layer, &cross, 0.0, p.color, cross_tf, cross_aa);
        fill_rects(&mut layer, &dot_sq, 0.0, p.color, id, false);
        fill_rects(&mut layer, &square, 0.0, p.color, id, false);
        if p.dot && p.dot_round {
            circle(&mut layer, p.dot_size as f32 / 2.0, p.color, None);
        }
        if p.circle {
            circle(&mut layer, p.circle_radius as f32, p.color, Some(p.circle_thick as f32));
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
    let rects = arms(c, gap, len, t, false);
    fill_rects(&mut pm, &rects, 0.0, [99, 102, 241, 255], Transform::identity(), false);
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for px in pm.pixels() {
        let d = px.demultiply();
        out.extend_from_slice(&[d.blue(), d.green(), d.red(), d.alpha()]);
    }
    out
}
