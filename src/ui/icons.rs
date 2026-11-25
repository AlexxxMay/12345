//! Векторные иконки интерфейса, нарисованные кодом на сетке 24×24.
//! Масштабируются без потери качества (HiDPI) и не требуют файлов ресурсов.

use crate::renderer::canvas::{Framebuffer, Rect};
use std::f32::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Menu,
    New,
    Open,
    Save,
    Undo,
    Redo,
    Import,
    Export,
    Settings,
    Help,
    Close,
    ChevronDown,
    ChevronRight,
    ChevronLeft,
    Eye,
    EyeOff,
    Home,
    Camera,
    Sketch,
    Extrude,
    Box,
    Cylinder,
    Sphere,
    Torus,
    Cone,
    Tube,
    Wedge,
    Pyramid,
    Capsule,
    Line,
    Rect,
    Circle,
    Finish,
    Move,
    Rotate,
    Scale,
    Mirror,
    Duplicate,
    Delete,
    Union,
    Subtract,
    Intersect,
    Weld,
    Normals,
    FlipNormals,
    Reduce,
    DropToFloor,
    Center,
    Repair,
    Measure,
    Info,
    Fit,
    Orbit,
    Pan,
    Zoom,
    Perspective,
    Grid,
    Shaded,
    ShadedEdges,
    Wireframe,
    Body,
    SketchItem,
    Folder,
    Select,
    First,
    Last,
    Play,
    Back,
    Language,
    Plane,
    /// Ручная правка граней сетки.
    EditMesh,
    /// Плюс (новая вкладка).
    Plus,
    /// Плоскость по текущему виду.
    ViewPlane,
    Arc,
    Polygon,
    Ellipse,
    Slot,
    Spline,
    SketchFillet,
    SketchOffset,
    Revolve,
    Sweep,
    Loft,
    Shell,
    Pattern,
    Twist,
    Taper,
    Bend,
    Coil,
    Subdivide,
    Smooth,
    Solidify,
}

const TOP: u32 = 0xFFCF_E3F7;
const LEFT: u32 = 0xFF8F_BDEA;
const RIGHT: u32 = 0xFF54_92D1;
const OUTLINE: u32 = 0xFF2B_5E92;
const ORANGE: u32 = 0xFFF0_8A24;
const RED: u32 = 0xFFE0_4848;
const GREEN: u32 = 0xFF3B_A55C;
const BLUE: u32 = 0xFF2F_7FD8;

struct P<'a> {
    fb: &'a mut Framebuffer,
    x: f32,
    y: f32,
    s: f32,
}

impl P<'_> {
    fn pt(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.s, self.y + y * self.s)
    }
    fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, w: f32, c: u32) {
        let a = self.pt(x0, y0);
        let b = self.pt(x1, y1);
        self.fb.line(a.0, a.1, b.0, b.1, w * self.s, c);
    }
    fn poly(&mut self, pts: &[(f32, f32)], c: u32) {
        let v: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.fb.fill_polygon(&v, c);
    }
    fn stroke(&mut self, pts: &[(f32, f32)], w: f32, c: u32, closed: bool) {
        let v: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.fb.polyline(&v, w * self.s, c, closed);
    }
    fn circle(&mut self, x: f32, y: f32, r: f32, c: u32) {
        let p = self.pt(x, y);
        self.fb.fill_circle(p.0, p.1, r * self.s, c);
    }
    fn ring(&mut self, x: f32, y: f32, r: f32, w: f32, c: u32) {
        let p = self.pt(x, y);
        self.fb.stroke_circle(p.0, p.1, r * self.s, w * self.s, c);
    }
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: u32) {
        self.poly(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h)], c);
    }
    fn arc(&mut self, cx: f32, cy: f32, r: f32, a0: f32, a1: f32, w: f32, c: u32) {
        let n = 16;
        let pts: Vec<(f32, f32)> = (0..=n)
            .map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / n as f32;
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        self.stroke(&pts, w, c, false);
    }
    fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, fill: Option<u32>, stroke: Option<(f32, u32)>) {
        let n = 28;
        let pts: Vec<(f32, f32)> = (0..n)
            .map(|i| {
                let a = 2.0 * PI * i as f32 / n as f32;
                (cx + rx * a.cos(), cy + ry * a.sin())
            })
            .collect();
        if let Some(c) = fill {
            self.poly(&pts, c);
        }
        if let Some((w, c)) = stroke {
            self.stroke(&pts, w, c, true);
        }
    }
    fn arrow_head(&mut self, x: f32, y: f32, dx: f32, dy: f32, size: f32, c: u32) {
        let l = (dx * dx + dy * dy).sqrt().max(1e-6);
        let (ux, uy) = (dx / l, dy / l);
        let (px, py) = (-uy, ux);
        self.poly(
            &[
                (x + ux * size, y + uy * size),
                (x + px * size * 0.7, y + py * size * 0.7),
                (x - px * size * 0.7, y - py * size * 0.7),
            ],
            c,
        );
    }
    /// Изометрический параллелепипед.
    fn iso_box(&mut self, cx: f32, top: f32, w: f32, h: f32) {
        let (l, r, mid) = (cx - w, cx + w, cx);
        let d = w * 0.5;
        let t = [(mid, top), (r, top + d), (mid, top + 2.0 * d), (l, top + d)];
        self.poly(&t, TOP);
        self.poly(&[(l, top + d), (mid, top + 2.0 * d), (mid, top + 2.0 * d + h), (l, top + d + h)], LEFT);
        self.poly(&[(mid, top + 2.0 * d), (r, top + d), (r, top + d + h), (mid, top + 2.0 * d + h)], RIGHT);
        self.stroke(&[(mid, top), (r, top + d), (r, top + d + h), (mid, top + 2.0 * d + h), (l, top + d + h), (l, top + d)], 1.1, OUTLINE, true);
        self.stroke(&[(l, top + d), (mid, top + 2.0 * d), (r, top + d)], 1.0, OUTLINE, false);
        self.line(mid, top + 2.0 * d, mid, top + 2.0 * d + h, 1.0, OUTLINE);
    }
    fn cylinder(&mut self, cx: f32, top: f32, rx: f32, h: f32, inner: Option<f32>) {
        let ry = rx * 0.38;
        self.poly(&[(cx - rx, top), (cx + rx, top), (cx + rx, top + h), (cx - rx, top + h)], LEFT);
        self.ellipse(cx, top + h, rx, ry, Some(LEFT), None);
        // Тень справа.
        self.poly(&[(cx + rx * 0.35, top), (cx + rx, top), (cx + rx, top + h), (cx + rx * 0.35, top + h + ry * 0.9)], RIGHT);
        self.ellipse(cx, top, rx, ry, Some(TOP), Some((1.0, OUTLINE)));
        if let Some(ir) = inner {
            self.ellipse(cx, top, ir, ir * 0.38, Some(RIGHT), Some((1.0, OUTLINE)));
        }
        self.line(cx - rx, top, cx - rx, top + h, 1.1, OUTLINE);
        self.line(cx + rx, top, cx + rx, top + h, 1.1, OUTLINE);
        self.arc(cx, top + h, rx, 0.0, PI, 1.1, OUTLINE);
    }
}

/// Рисование иконки в прямоугольнике цветом `c` (для монохромных иконок).
pub fn draw(fb: &mut Framebuffer, icon: Icon, r: Rect, c: u32) {
    let size = r.w.min(r.h);
    let mut p = P { fb, x: r.x + (r.w - size) * 0.5, y: r.y + (r.h - size) * 0.5, s: size / 24.0 };
    let w = 1.7;
    match icon {
        Icon::Menu => {
            for y in [7.0, 12.0, 17.0] {
                p.line(4.0, y, 20.0, y, 2.0, c);
            }
        }
        Icon::New => {
            p.stroke(&[(6.0, 3.0), (14.0, 3.0), (19.0, 8.0), (19.0, 21.0), (6.0, 21.0)], w, c, true);
            p.stroke(&[(14.0, 3.0), (14.0, 8.0), (19.0, 8.0)], w, c, false);
            p.line(12.5, 11.0, 12.5, 18.0, 1.8, BLUE);
            p.line(9.0, 14.5, 16.0, 14.5, 1.8, BLUE);
        }
        Icon::Open | Icon::Folder => {
            p.poly(&[(3.0, 6.0), (9.0, 6.0), (11.0, 8.0), (20.0, 8.0), (20.0, 19.0), (3.0, 19.0)], 0xFFF2_C14E);
            p.poly(&[(3.0, 19.0), (6.0, 11.0), (22.0, 11.0), (20.0, 19.0)], 0xFFF7_D57A);
            p.stroke(&[(3.0, 6.0), (9.0, 6.0), (11.0, 8.0), (20.0, 8.0), (20.0, 11.0)], 1.0, 0xFFB0_7F1A, false);
            p.stroke(&[(3.0, 19.0), (6.0, 11.0), (22.0, 11.0), (20.0, 19.0), (3.0, 19.0), (3.0, 6.0)], 1.0, 0xFFB0_7F1A, false);
        }
        Icon::Save => {
            p.poly(&[(4.0, 4.0), (17.0, 4.0), (20.0, 7.0), (20.0, 20.0), (4.0, 20.0)], BLUE);
            p.rect(7.0, 4.0, 9.0, 5.0, 0xFFE8_F1FB);
            p.rect(7.0, 13.0, 10.0, 7.0, 0xFFFF_FFFF);
            p.line(9.0, 15.5, 15.0, 15.5, 1.0, 0xFF9A_B4D0);
            p.line(9.0, 17.5, 15.0, 17.5, 1.0, 0xFF9A_B4D0);
        }
        Icon::Undo | Icon::Redo => {
            let flip = icon == Icon::Redo;
            let fx = |x: f32| if flip { 24.0 - x } else { x };
            let pts: Vec<(f32, f32)> = (0..=14)
                .map(|i| {
                    let a = -PI * 0.5 + PI * i as f32 / 14.0;
                    (fx(13.0 + 6.0 * a.cos()), 13.0 + 5.0 * a.sin())
                })
                .collect();
            p.stroke(&pts, 2.0, c, false);
            p.line(fx(13.0), 18.0, fx(7.0), 18.0, 2.0, c);
            p.line(fx(13.0), 8.0, fx(9.0), 8.0, 2.0, c);
            p.arrow_head(fx(6.0), 8.0, if flip { 1.0 } else { -1.0 }, 0.0, 4.0, c);
        }
        Icon::Import | Icon::Export => {
            p.stroke(&[(4.0, 14.0), (4.0, 20.0), (20.0, 20.0), (20.0, 14.0)], w, c, false);
            if icon == Icon::Import {
                p.line(12.0, 3.0, 12.0, 13.0, 2.0, BLUE);
                p.arrow_head(12.0, 16.0, 0.0, 1.0, 4.5, BLUE);
            } else {
                p.line(12.0, 16.0, 12.0, 7.0, 2.0, GREEN);
                p.arrow_head(12.0, 3.5, 0.0, -1.0, 4.5, GREEN);
            }
        }
        Icon::Settings => {
            for i in 0..8 {
                let a = i as f32 * PI / 4.0;
                p.line(12.0 + 5.0 * a.cos(), 12.0 + 5.0 * a.sin(), 12.0 + 9.0 * a.cos(), 12.0 + 9.0 * a.sin(), 3.2, c);
            }
            p.circle(12.0, 12.0, 6.5, c);
            p.circle(12.0, 12.0, 2.8, 0xFFFF_FFFF);
        }
        Icon::Help => {
            p.ring(12.0, 12.0, 9.0, w, c);
            p.arc(12.0, 9.5, 3.0, PI, PI * 2.3, 2.0, c);
            p.line(12.0, 12.8, 12.0, 14.5, 2.0, c);
            p.circle(12.0, 17.5, 1.3, c);
        }
        Icon::Close => {
            p.line(6.0, 6.0, 18.0, 18.0, 2.0, c);
            p.line(18.0, 6.0, 6.0, 18.0, 2.0, c);
        }
        Icon::ChevronDown => p.stroke(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)], 2.2, c, false),
        Icon::ChevronRight => p.stroke(&[(9.0, 6.0), (15.0, 12.0), (9.0, 18.0)], 2.2, c, false),
        Icon::ChevronLeft => p.stroke(&[(15.0, 6.0), (9.0, 12.0), (15.0, 18.0)], 2.2, c, false),
        Icon::Eye | Icon::EyeOff => {
            let pts_top: Vec<(f32, f32)> = (0..=12).map(|i| {
                let t = i as f32 / 12.0;
                (3.0 + 18.0 * t, 12.0 - 7.0 * (PI * t).sin())
            }).collect();
            let pts_bot: Vec<(f32, f32)> = (0..=12).map(|i| {
                let t = i as f32 / 12.0;
                (3.0 + 18.0 * t, 12.0 + 7.0 * (PI * t).sin())
            }).collect();
            p.stroke(&pts_top, w, c, false);
            p.stroke(&pts_bot, w, c, false);
            p.circle(12.0, 12.0, 3.2, c);
            if icon == Icon::EyeOff {
                p.line(4.0, 20.0, 20.0, 4.0, 2.0, c);
            }
        }
        Icon::Home => {
            p.poly(&[(12.0, 3.5), (21.0, 11.5), (18.5, 11.5), (18.5, 20.0), (5.5, 20.0), (5.5, 11.5), (3.0, 11.5)], c);
            p.rect(10.0, 14.0, 4.0, 6.0, 0xFFFF_FFFF);
        }
        Icon::Camera => {
            p.poly(&[(3.0, 8.0), (8.0, 8.0), (10.0, 5.0), (14.0, 5.0), (16.0, 8.0), (21.0, 8.0), (21.0, 19.0), (3.0, 19.0)], c);
            p.circle(12.0, 13.0, 4.2, 0xFFFF_FFFF);
            p.circle(12.0, 13.0, 2.6, c);
        }
        Icon::Sketch | Icon::SketchItem => {
            p.poly(&[(2.0, 16.0), (14.0, 12.0), (22.0, 16.0), (10.0, 20.0)], 0xFFFD_E3C6);
            p.stroke(&[(2.0, 16.0), (14.0, 12.0), (22.0, 16.0), (10.0, 20.0)], 1.0, ORANGE, true);
            p.line(17.0, 3.0, 9.0, 13.0, 3.6, if super::theme::is_dark() { 0xFF9A_A4B2 } else { 0xFF4A_5059 });
            p.poly(&[(9.0, 13.0), (7.5, 16.0), (10.8, 14.6)], ORANGE);
            p.line(17.0, 3.0, 19.0, 4.6, 3.6, RED);
        }
        Icon::Extrude => {
            p.poly(&[(3.0, 17.0), (12.0, 13.0), (21.0, 17.0), (12.0, 21.0)], 0xFFFD_E3C6);
            p.stroke(&[(3.0, 17.0), (12.0, 13.0), (21.0, 17.0), (12.0, 21.0)], 1.0, ORANGE, true);
            p.iso_box(12.0, 4.0, 5.0, 6.0);
            p.line(19.5, 14.0, 19.5, 5.0, 1.8, BLUE);
            p.arrow_head(19.5, 3.0, 0.0, -1.0, 3.0, BLUE);
        }
        Icon::Box | Icon::Body => p.iso_box(12.0, 3.0, 8.0, 9.0),
        Icon::Cylinder => p.cylinder(12.0, 6.0, 7.5, 11.0, None),
        Icon::Tube => p.cylinder(12.0, 6.0, 7.5, 11.0, Some(4.0)),
        Icon::Sphere => {
            p.circle(12.0, 12.0, 8.5, LEFT);
            p.circle(10.0, 10.0, 5.5, TOP);
            p.circle(9.0, 9.0, 2.5, 0xFFF4_F9FE);
            p.ring(12.0, 12.0, 8.5, 1.1, OUTLINE);
        }
        Icon::Torus => {
            p.ellipse(12.0, 12.0, 9.5, 6.0, Some(LEFT), Some((1.1, OUTLINE)));
            p.ellipse(12.0, 11.0, 7.0, 3.6, Some(TOP), None);
            p.ellipse(12.0, 12.0, 3.8, 1.8, Some(0xFFFF_FFFF), Some((1.0, OUTLINE)));
        }
        Icon::Cone => {
            p.poly(&[(12.0, 3.0), (20.0, 17.0), (4.0, 17.0)], LEFT);
            p.ellipse(12.0, 17.0, 8.0, 3.0, Some(LEFT), None);
            p.poly(&[(12.0, 3.0), (20.0, 17.0), (15.0, 19.6)], RIGHT);
            p.stroke(&[(4.0, 17.0), (12.0, 3.0), (20.0, 17.0)], 1.1, OUTLINE, false);
            p.arc(12.0, 17.0, 8.0, 0.0, PI, 1.1, OUTLINE);
            p.ellipse(12.0, 17.0, 8.0, 3.0, None, Some((0.6, 0x802B_5E92)));
        }
        Icon::Wedge => {
            p.poly(&[(4.0, 18.0), (4.0, 6.0), (14.0, 3.0), (14.0, 15.0)], LEFT);
            p.poly(&[(14.0, 3.0), (21.0, 13.0), (21.0, 20.0), (14.0, 15.0)], TOP);
            p.poly(&[(4.0, 18.0), (14.0, 15.0), (21.0, 20.0), (11.0, 22.0)], RIGHT);
            p.stroke(&[(4.0, 6.0), (14.0, 3.0), (21.0, 13.0), (21.0, 20.0), (11.0, 22.0), (4.0, 18.0)], 1.1, OUTLINE, true);
            p.stroke(&[(4.0, 18.0), (14.0, 15.0), (21.0, 20.0)], 0.9, OUTLINE, false);
            p.line(14.0, 3.0, 14.0, 15.0, 0.9, OUTLINE);
        }
        Icon::Pyramid => {
            p.poly(&[(12.0, 3.0), (3.0, 16.0), (11.0, 21.0)], LEFT);
            p.poly(&[(12.0, 3.0), (11.0, 21.0), (21.0, 16.0)], RIGHT);
            p.stroke(&[(12.0, 3.0), (3.0, 16.0), (11.0, 21.0), (21.0, 16.0)], 1.1, OUTLINE, true);
            p.line(12.0, 3.0, 11.0, 21.0, 0.9, OUTLINE);
        }
        Icon::Capsule => {
            p.poly(&[(7.0, 8.0), (17.0, 8.0), (17.0, 16.0), (7.0, 16.0)], LEFT);
            p.circle(12.0, 8.0, 5.0, TOP);
            p.circle(12.0, 16.0, 5.0, LEFT);
            p.poly(&[(14.0, 8.0), (17.0, 8.0), (17.0, 16.0), (14.0, 16.0)], RIGHT);
            p.arc(12.0, 8.0, 5.0, PI, 2.0 * PI, 1.1, OUTLINE);
            p.arc(12.0, 16.0, 5.0, 0.0, PI, 1.1, OUTLINE);
            p.line(7.0, 8.0, 7.0, 16.0, 1.1, OUTLINE);
            p.line(17.0, 8.0, 17.0, 16.0, 1.1, OUTLINE);
        }
        Icon::Line => {
            p.line(5.0, 19.0, 19.0, 5.0, 2.0, c);
            p.circle(5.0, 19.0, 2.4, ORANGE);
            p.circle(19.0, 5.0, 2.4, ORANGE);
        }
        Icon::Rect => {
            p.stroke(&[(4.0, 6.0), (20.0, 6.0), (20.0, 18.0), (4.0, 18.0)], 2.0, c, true);
            p.circle(4.0, 6.0, 2.2, ORANGE);
            p.circle(20.0, 18.0, 2.2, ORANGE);
        }
        Icon::Circle => {
            p.ring(12.0, 12.0, 8.0, 2.0, c);
            p.circle(12.0, 12.0, 2.0, ORANGE);
            p.line(12.0, 12.0, 17.6, 6.4, 1.0, ORANGE);
        }
        Icon::Finish => {
            p.circle(12.0, 12.0, 9.5, GREEN);
            p.stroke(&[(7.0, 12.5), (10.5, 16.0), (17.0, 8.5)], 2.4, 0xFFFF_FFFF, false);
        }
        Icon::Move => {
            p.line(12.0, 4.0, 12.0, 20.0, 1.8, c);
            p.line(4.0, 12.0, 20.0, 12.0, 1.8, c);
            p.arrow_head(12.0, 2.5, 0.0, -1.0, 3.5, GREEN);
            p.arrow_head(12.0, 21.5, 0.0, 1.0, 3.5, GREEN);
            p.arrow_head(2.5, 12.0, -1.0, 0.0, 3.5, RED);
            p.arrow_head(21.5, 12.0, 1.0, 0.0, 3.5, RED);
        }
        Icon::Rotate | Icon::Orbit => {
            p.arc(12.0, 12.0, 7.5, -PI * 0.15, PI * 1.45, 2.0, if icon == Icon::Rotate { BLUE } else { c });
            p.arrow_head(12.0 + 7.5 * (-PI * 0.15f32).cos(), 12.0 + 7.5 * (-PI * 0.15f32).sin(), 0.3, 1.0, 3.6, if icon == Icon::Rotate { BLUE } else { c });
            if icon == Icon::Orbit {
                p.circle(12.0, 12.0, 2.2, c);
            } else {
                p.iso_box(12.0, 8.0, 3.5, 3.5);
            }
        }
        Icon::Scale => {
            p.stroke(&[(3.0, 10.0), (3.0, 21.0), (14.0, 21.0), (14.0, 10.0)], 1.5, c, true);
            p.line(10.0, 14.0, 20.0, 4.0, 1.8, BLUE);
            p.stroke(&[(14.0, 4.0), (20.0, 4.0), (20.0, 10.0)], 1.8, BLUE, false);
        }
        Icon::Mirror => {
            p.poly(&[(3.0, 18.0), (10.0, 6.0), (10.0, 18.0)], LEFT);
            p.stroke(&[(14.0, 18.0), (14.0, 6.0), (21.0, 18.0)], 1.4, OUTLINE, true);
            for i in 0..6 {
                let y = 3.0 + i as f32 * 3.4;
                p.line(12.0, y, 12.0, y + 1.8, 1.4, c);
            }
        }
        Icon::Duplicate => {
            p.rect(8.0, 8.0, 12.0, 12.0, LEFT);
            p.stroke(&[(8.0, 8.0), (20.0, 8.0), (20.0, 20.0), (8.0, 20.0)], 1.2, OUTLINE, true);
            p.stroke(&[(4.0, 16.0), (4.0, 4.0), (16.0, 4.0)], 1.6, c, false);
        }
        Icon::Delete => {
            p.line(4.0, 6.5, 20.0, 6.5, 2.0, c);
            p.stroke(&[(9.0, 6.0), (10.0, 3.5), (14.0, 3.5), (15.0, 6.0)], 1.6, c, false);
            p.poly(&[(6.0, 8.5), (18.0, 8.5), (17.0, 21.0), (7.0, 21.0)], RED);
            p.line(10.0, 11.0, 10.3, 18.5, 1.2, 0xFFFF_FFFF);
            p.line(14.0, 11.0, 13.7, 18.5, 1.2, 0xFFFF_FFFF);
        }
        Icon::Union | Icon::Subtract | Icon::Intersect => {
            let (fa, fb, fi) = match icon {
                Icon::Union => (LEFT, LEFT, LEFT),
                Icon::Subtract => (LEFT, 0x00FF_FFFF, 0x00FF_FFFF),
                _ => (0x00FF_FFFF, 0x00FF_FFFF, LEFT),
            };
            p.circle(9.0, 12.0, 6.5, fa);
            p.circle(15.0, 12.0, 6.5, fb);
            if icon == Icon::Intersect || icon == Icon::Subtract {
                // Пересечение двух кругов (линза).
                let n = 20;
                let mut lens = Vec::new();
                let h = (6.5f32 * 6.5 - 9.0).sqrt();
                let a = h.atan2(3.0);
                for i in 0..=n {
                    let t = -a + 2.0 * a * i as f32 / n as f32;
                    lens.push((9.0 + 6.5 * t.cos(), 12.0 + 6.5 * t.sin()));
                }
                for i in 0..=n {
                    let t = PI - a + 2.0 * a * i as f32 / n as f32;
                    lens.push((15.0 + 6.5 * t.cos(), 12.0 + 6.5 * t.sin()));
                }
                p.poly(&lens, if icon == Icon::Subtract { 0xFFF5_F6F8 } else { fi });
            }
            p.ring(9.0, 12.0, 6.5, 1.2, OUTLINE);
            p.ring(15.0, 12.0, 6.5, 1.2, if icon == Icon::Subtract { RED } else { OUTLINE });
        }
        Icon::Weld => {
            p.stroke(&[(3.0, 18.0), (10.0, 12.0), (3.0, 6.0)], 1.5, c, false);
            p.stroke(&[(21.0, 18.0), (14.0, 12.0), (21.0, 6.0)], 1.5, c, false);
            p.circle(10.0, 12.0, 2.2, ORANGE);
            p.circle(14.0, 12.0, 2.2, ORANGE);
            p.line(10.0, 12.0, 14.0, 12.0, 1.4, ORANGE);
        }
        Icon::Normals | Icon::FlipNormals => {
            p.poly(&[(3.0, 18.0), (12.0, 14.0), (21.0, 18.0), (12.0, 22.0)], LEFT);
            p.stroke(&[(3.0, 18.0), (12.0, 14.0), (21.0, 18.0), (12.0, 22.0)], 1.0, OUTLINE, true);
            if icon == Icon::Normals {
                p.line(12.0, 18.0, 12.0, 6.0, 1.8, GREEN);
                p.arrow_head(12.0, 3.5, 0.0, -1.0, 3.5, GREEN);
            } else {
                p.line(12.0, 3.0, 12.0, 14.0, 1.8, RED);
                p.arrow_head(12.0, 16.0, 0.0, 1.0, 3.5, RED);
                p.arc(12.0, 7.0, 4.0, PI * 1.1, PI * 1.9, 1.4, c);
            }
        }
        Icon::Reduce => {
            p.stroke(&[(3.0, 20.0), (8.0, 6.0), (13.0, 20.0), (18.0, 8.0), (21.0, 20.0)], 1.5, c, false);
            p.line(3.0, 20.0, 21.0, 20.0, 1.5, c);
            p.line(8.0, 6.0, 18.0, 8.0, 1.0, ORANGE);
            p.circle(8.0, 6.0, 1.8, ORANGE);
            p.circle(18.0, 8.0, 1.8, ORANGE);
        }
        Icon::DropToFloor => {
            p.iso_box(12.0, 2.0, 5.0, 5.0);
            p.line(12.0, 15.0, 12.0, 17.5, 1.8, BLUE);
            p.arrow_head(12.0, 19.0, 0.0, 1.0, 2.6, BLUE);
            p.line(3.0, 21.5, 21.0, 21.5, 2.0, c);
        }
        Icon::Center => {
            p.ring(12.0, 12.0, 7.0, 1.6, c);
            p.line(12.0, 2.0, 12.0, 8.0, 1.6, c);
            p.line(12.0, 16.0, 12.0, 22.0, 1.6, c);
            p.line(2.0, 12.0, 8.0, 12.0, 1.6, c);
            p.line(16.0, 12.0, 22.0, 12.0, 1.6, c);
            p.circle(12.0, 12.0, 2.2, RED);
        }
        Icon::Repair => {
            p.line(5.0, 19.0, 14.0, 10.0, 3.2, c);
            p.ring(16.0, 8.0, 4.0, 2.4, c);
            p.poly(&[(16.0, 3.0), (21.0, 8.0), (19.0, 10.0), (14.0, 5.0)], 0xFFF5_F6F8);
        }
        Icon::Measure => {
            p.poly(&[(2.0, 15.0), (15.0, 2.0), (22.0, 9.0), (9.0, 22.0)], 0xFFF2_C14E);
            p.stroke(&[(2.0, 15.0), (15.0, 2.0), (22.0, 9.0), (9.0, 22.0)], 1.1, 0xFFA0_7414, true);
            for i in 0..5 {
                let t = 3.5 + i as f32 * 2.4;
                let (x, y) = (2.0 + t, 15.0 - t);
                let l = if i % 2 == 0 { 4.0 } else { 2.5 };
                p.line(x, y, x + l * 0.7, y + l * 0.7, 1.0, 0xFF6B_4E0E);
            }
        }
        Icon::Info => {
            p.circle(12.0, 12.0, 9.5, BLUE);
            p.circle(12.0, 7.5, 1.5, 0xFFFF_FFFF);
            p.line(12.0, 11.0, 12.0, 17.5, 2.4, 0xFFFF_FFFF);
        }
        Icon::Fit => {
            for (x, y, dx, dy) in [(4.0, 4.0, 1.0, 1.0), (20.0, 4.0, -1.0, 1.0), (4.0, 20.0, 1.0, -1.0), (20.0, 20.0, -1.0, -1.0)] {
                p.stroke(&[(x, y + 5.0 * dy), (x, y), (x + 5.0 * dx, y)], 2.0, c, false);
            }
            p.iso_box(12.0, 7.5, 4.0, 3.5);
        }
        Icon::Pan => {
            p.line(12.0, 3.0, 12.0, 21.0, 1.6, c);
            p.line(3.0, 12.0, 21.0, 12.0, 1.6, c);
            p.arrow_head(12.0, 2.0, 0.0, -1.0, 3.0, c);
            p.arrow_head(12.0, 22.0, 0.0, 1.0, 3.0, c);
            p.arrow_head(2.0, 12.0, -1.0, 0.0, 3.0, c);
            p.arrow_head(22.0, 12.0, 1.0, 0.0, 3.0, c);
            p.circle(12.0, 12.0, 3.0, c);
        }
        Icon::Zoom => {
            p.ring(10.0, 10.0, 6.0, 2.0, c);
            p.line(14.5, 14.5, 20.5, 20.5, 3.0, c);
            p.line(7.0, 10.0, 13.0, 10.0, 1.6, c);
            p.line(10.0, 7.0, 10.0, 13.0, 1.6, c);
        }
        Icon::Perspective => {
            p.stroke(&[(4.0, 7.0), (20.0, 4.0), (20.0, 20.0), (4.0, 17.0)], 1.6, c, true);
            p.stroke(&[(9.0, 9.0), (16.0, 8.0), (16.0, 16.0), (9.0, 15.0)], 1.2, c, true);
        }
        Icon::Grid => {
            for i in 0..4 {
                let t = 4.0 + i as f32 * 5.33;
                p.line(t, 4.0, t, 20.0, 1.2, c);
                p.line(4.0, t, 20.0, t, 1.2, c);
            }
        }
        Icon::Shaded => p.iso_box(12.0, 3.0, 8.0, 9.0),
        Icon::ShadedEdges => {
            p.iso_box(12.0, 3.0, 8.0, 9.0);
            p.stroke(&[(12.0, 3.0), (20.0, 7.0), (20.0, 16.0), (12.0, 20.0), (4.0, 16.0), (4.0, 7.0)], 1.8, if super::theme::is_dark() { 0xFFD0_D6DE } else { 0xFF20_2428 }, true);
        }
        Icon::Wireframe => {
            p.stroke(&[(12.0, 3.0), (20.0, 7.0), (20.0, 16.0), (12.0, 20.0), (4.0, 16.0), (4.0, 7.0)], 1.4, c, true);
            p.stroke(&[(4.0, 7.0), (12.0, 11.0), (20.0, 7.0)], 1.2, c, false);
            p.line(12.0, 11.0, 12.0, 20.0, 1.2, c);
            p.stroke(&[(4.0, 16.0), (12.0, 12.0), (20.0, 16.0)], 0.8, 0x80000000 | (c & 0xFFFFFF), false);
            p.line(12.0, 3.0, 12.0, 12.0, 0.8, 0x80000000 | (c & 0xFFFFFF));
        }
        Icon::Select => {
            p.poly(&[(6.0, 3.0), (6.0, 19.0), (10.0, 15.0), (13.0, 21.5), (15.5, 20.5), (12.5, 14.0), (18.0, 14.0)], c);
        }
        Icon::First | Icon::Last => {
            let f = icon == Icon::Last;
            let fx = |x: f32| if f { 24.0 - x } else { x };
            p.rect(fx(6.0).min(fx(8.0)), 6.0, 2.0, 12.0, c);
            p.poly(&[(fx(9.0), 12.0), (fx(18.0), 6.0), (fx(18.0), 18.0)], c);
        }
        Icon::Play | Icon::Back => {
            let f = icon == Icon::Back;
            let fx = |x: f32| if f { 24.0 - x } else { x };
            p.poly(&[(fx(8.0), 6.0), (fx(17.0), 12.0), (fx(8.0), 18.0)], c);
        }
        Icon::Language => {
            p.ring(12.0, 12.0, 9.0, 1.5, c);
            p.ellipse(12.0, 12.0, 4.0, 9.0, None, Some((1.3, c)));
            p.line(3.0, 12.0, 21.0, 12.0, 1.3, c);
            p.line(5.0, 7.5, 19.0, 7.5, 1.0, c);
            p.line(5.0, 16.5, 19.0, 16.5, 1.0, c);
        }
        Icon::Arc => {
            p.arc(12.0, 16.0, 8.0, PI, 2.0 * PI, 2.0, c);
            p.circle(4.0, 16.0, 2.2, ORANGE);
            p.circle(20.0, 16.0, 2.2, ORANGE);
            p.circle(12.0, 8.0, 1.8, ORANGE);
        }
        Icon::Polygon => {
            let pts: Vec<(f32, f32)> = (0..6).map(|i| {
                let a = PI / 3.0 * i as f32;
                (12.0 + 8.5 * a.cos(), 12.0 + 8.5 * a.sin())
            }).collect();
            p.stroke(&pts, 2.0, c, true);
            p.circle(12.0, 12.0, 1.8, ORANGE);
            p.circle(20.5, 12.0, 2.2, ORANGE);
        }
        Icon::Ellipse => {
            p.ellipse(12.0, 12.0, 9.0, 5.5, None, Some((2.0, c)));
            p.circle(12.0, 12.0, 1.8, ORANGE);
            p.line(12.0, 12.0, 21.0, 12.0, 1.0, ORANGE);
        }
        Icon::Slot => {
            let mut pts = Vec::new();
            for i in 0..=12 {
                let a = -PI / 2.0 + PI * i as f32 / 12.0;
                pts.push((16.0 + 5.0 * a.cos(), 12.0 + 5.0 * a.sin()));
            }
            for i in 0..=12 {
                let a = PI / 2.0 + PI * i as f32 / 12.0;
                pts.push((8.0 + 5.0 * a.cos(), 12.0 + 5.0 * a.sin()));
            }
            p.stroke(&pts, 2.0, c, true);
            p.circle(8.0, 12.0, 1.8, ORANGE);
            p.circle(16.0, 12.0, 1.8, ORANGE);
        }
        Icon::Spline => {
            let pts: Vec<(f32, f32)> = (0..=24).map(|i| {
                let t = i as f32 / 24.0;
                (3.0 + 18.0 * t, 12.0 - 7.0 * (t * 2.0 * PI).sin())
            }).collect();
            p.stroke(&pts, 2.0, c, false);
            for (x, y) in [(3.0, 12.0), (7.5, 5.0), (16.5, 19.0), (21.0, 12.0)] {
                p.circle(x, y, 2.0, ORANGE);
            }
        }
        Icon::SketchFillet => {
            p.line(5.0, 3.0, 5.0, 11.0, 2.0, c);
            p.line(13.0, 19.0, 21.0, 19.0, 2.0, c);
            p.arc(13.0, 11.0, 8.0, PI * 0.5, PI, 2.2, ORANGE);
            p.stroke(&[(5.0, 11.0), (5.0, 19.0), (13.0, 19.0)], 1.0, 0x80F0_8A24, false);
        }
        Icon::SketchOffset => {
            p.stroke(&[(8.0, 8.0), (16.0, 8.0), (16.0, 16.0), (8.0, 16.0)], 2.0, c, true);
            p.stroke(&[(4.0, 4.0), (20.0, 4.0), (20.0, 20.0), (4.0, 20.0)], 1.6, ORANGE, true);
            p.line(16.0, 12.0, 20.0, 12.0, 1.0, BLUE);
        }
        Icon::Revolve => {
            p.ellipse(12.0, 17.0, 8.5, 3.2, Some(LEFT), Some((1.0, OUTLINE)));
            p.poly(&[(14.0, 6.0), (19.0, 6.0), (19.0, 16.0), (14.0, 16.0)], 0xFFFD_E3C6);
            p.stroke(&[(14.0, 6.0), (19.0, 6.0), (19.0, 16.0), (14.0, 16.0)], 1.1, ORANGE, true);
            p.line(12.0, 2.0, 12.0, 22.0, 1.4, BLUE);
            p.arc(12.0, 17.0, 8.5, PI * 0.15, PI * 0.95, 1.8, BLUE);
            p.arrow_head(12.0 + 8.5 * (PI * 0.95f32).cos(), 17.0 + 3.2 * (PI * 0.95f32).sin(), -1.0, -0.4, 3.0, BLUE);
        }
        Icon::Sweep => {
            let path: Vec<(f32, f32)> = (0..=16).map(|i| {
                let t = i as f32 / 16.0;
                (4.0 + 16.0 * t, 19.0 - 14.0 * t * t)
            }).collect();
            p.stroke(&path, 6.0, LEFT, false);
            p.stroke(&path, 1.2, ORANGE, false);
            p.ellipse(4.0, 19.0, 2.2, 3.2, Some(TOP), Some((1.0, OUTLINE)));
            p.ellipse(20.0, 5.0, 3.2, 2.2, Some(TOP), Some((1.0, OUTLINE)));
        }
        Icon::Loft => {
            p.poly(&[(3.0, 18.0), (21.0, 18.0), (16.0, 6.0), (8.0, 6.0)], LEFT);
            p.poly(&[(14.0, 18.0), (21.0, 18.0), (16.0, 6.0), (13.0, 6.0)], RIGHT);
            p.ellipse(12.0, 6.0, 4.0, 1.6, Some(TOP), Some((1.0, OUTLINE)));
            p.stroke(&[(3.0, 18.0), (21.0, 18.0)], 1.6, ORANGE, false);
            p.line(3.0, 18.0, 8.0, 6.0, 1.0, OUTLINE);
            p.line(21.0, 18.0, 16.0, 6.0, 1.0, OUTLINE);
        }
        Icon::Shell => {
            p.iso_box(12.0, 3.0, 8.0, 9.0);
            p.poly(&[(12.0, 5.0), (17.0, 7.5), (12.0, 10.0), (7.0, 7.5)], RIGHT);
            p.poly(&[(7.0, 7.5), (12.0, 10.0), (12.0, 11.5), (7.0, 9.0)], OUTLINE);
            p.stroke(&[(12.0, 5.0), (17.0, 7.5), (12.0, 10.0), (7.0, 7.5)], 0.9, OUTLINE, true);
        }
        Icon::Pattern => {
            p.iso_box(5.5, 9.0, 3.5, 5.0);
            p.iso_box(12.0, 6.0, 3.5, 5.0);
            p.iso_box(18.5, 3.0, 3.5, 5.0);
        }
        Icon::Twist => {
            p.poly(&[(8.0, 4.0), (16.0, 4.0), (16.0, 21.0), (8.0, 21.0)], LEFT);
            for k in 0..3 {
                let y = 6.0 + k as f32 * 5.5;
                p.stroke(&[(8.0, y + 4.0), (12.0, y + 1.0), (16.0, y)], 1.2, OUTLINE, false);
            }
            p.stroke(&[(8.0, 4.0), (16.0, 4.0), (16.0, 21.0), (8.0, 21.0)], 1.1, OUTLINE, true);
            p.arc(12.0, 4.0, 7.0, PI * 1.1, PI * 1.9, 1.6, BLUE);
            p.arrow_head(12.0 + 7.0 * (PI * 1.9f32).cos(), 4.0 + 7.0 * (PI * 1.9f32).sin(), 1.0, 0.6, 2.6, BLUE);
        }
        Icon::Taper => {
            p.poly(&[(4.0, 20.0), (20.0, 20.0), (15.0, 4.0), (9.0, 4.0)], LEFT);
            p.poly(&[(13.0, 20.0), (20.0, 20.0), (15.0, 4.0), (12.5, 4.0)], RIGHT);
            p.stroke(&[(4.0, 20.0), (20.0, 20.0), (15.0, 4.0), (9.0, 4.0)], 1.1, OUTLINE, true);
        }
        Icon::Bend => {
            let outer: Vec<(f32, f32)> = (0..=12).map(|i| {
                let a = PI + PI * 0.5 * i as f32 / 12.0;
                (20.0 + 16.0 * a.cos(), 21.0 + 16.0 * a.sin())
            }).collect();
            let inner: Vec<(f32, f32)> = (0..=12).rev().map(|i| {
                let a = PI + PI * 0.5 * i as f32 / 12.0;
                (20.0 + 10.0 * a.cos(), 21.0 + 10.0 * a.sin())
            }).collect();
            let mut band = outer.clone();
            band.extend(inner.iter().copied());
            p.poly(&band, LEFT);
            p.stroke(&band, 1.1, OUTLINE, true);
        }
        Icon::Coil => {
            for k in 0..4 {
                let y = 5.0 + k as f32 * 4.5;
                p.arc(12.0, y, 8.0, 0.0, PI, 2.0, OUTLINE);
                p.arc(12.0, y + 2.2, 8.0, PI, 2.0 * PI, 2.0, LEFT);
            }
        }
        Icon::Subdivide => {
            p.poly(&[(12.0, 3.0), (21.0, 20.0), (3.0, 20.0)], TOP);
            p.stroke(&[(12.0, 3.0), (21.0, 20.0), (3.0, 20.0)], 1.6, OUTLINE, true);
            p.stroke(&[(7.5, 11.5), (16.5, 11.5), (12.0, 20.0)], 1.1, BLUE, true);
        }
        Icon::Smooth => {
            p.stroke(&[(3.0, 16.0), (6.0, 9.0), (9.0, 15.0), (12.0, 7.0), (15.0, 14.0), (18.0, 8.0), (21.0, 13.0)], 1.0, c, false);
            let sm: Vec<(f32, f32)> = (0..=20).map(|i| {
                let t = i as f32 / 20.0;
                (3.0 + 18.0 * t, 13.5 - 2.5 * (t * PI).sin())
            }).collect();
            p.stroke(&sm, 2.2, BLUE, false);
        }
        Icon::Solidify => {
            p.poly(&[(3.0, 10.0), (14.0, 5.0), (21.0, 9.0), (10.0, 14.0)], TOP);
            p.poly(&[(3.0, 10.0), (10.0, 14.0), (10.0, 19.0), (3.0, 15.0)], LEFT);
            p.poly(&[(10.0, 14.0), (21.0, 9.0), (21.0, 14.0), (10.0, 19.0)], RIGHT);
            p.stroke(&[(3.0, 10.0), (14.0, 5.0), (21.0, 9.0), (21.0, 14.0), (10.0, 19.0), (3.0, 15.0)], 1.1, OUTLINE, true);
            p.stroke(&[(3.0, 10.0), (10.0, 14.0), (21.0, 9.0)], 0.9, OUTLINE, false);
            p.line(10.0, 14.0, 10.0, 19.0, 0.9, OUTLINE);
        }
        Icon::Plane => {
            p.poly(&[(2.0, 14.0), (14.0, 8.0), (22.0, 12.0), (10.0, 18.0)], 0xFFFD_E3C6);
            p.stroke(&[(2.0, 14.0), (14.0, 8.0), (22.0, 12.0), (10.0, 18.0)], 1.2, ORANGE, true);
        }
        Icon::EditMesh => {
            // Куб, верхняя грань выделена и вытянута вверх стрелкой.
            p.poly(&[(3.0, 13.0), (10.0, 17.0), (10.0, 22.0), (3.0, 18.0)], LEFT);
            p.poly(&[(10.0, 17.0), (19.0, 13.0), (19.0, 18.0), (10.0, 22.0)], RIGHT);
            p.poly(&[(3.0, 13.0), (12.0, 9.0), (19.0, 13.0), (10.0, 17.0)], 0xFFFF_D27A);
            p.stroke(&[(3.0, 13.0), (12.0, 9.0), (19.0, 13.0), (10.0, 17.0)], 1.3, ORANGE, true);
            p.stroke(&[(3.0, 13.0), (3.0, 18.0), (10.0, 22.0), (19.0, 18.0), (19.0, 13.0)], 1.0, OUTLINE, false);
            p.line(10.0, 17.0, 10.0, 22.0, 0.9, OUTLINE);
            p.line(11.0, 12.5, 11.0, 4.5, 1.8, BLUE);
            p.arrow_head(11.0, 2.5, 0.0, -1.0, 3.0, BLUE);
            p.circle(19.0, 13.0, 1.8, RED);
        }
        Icon::Plus => {
            p.line(12.0, 5.0, 12.0, 19.0, 2.0, c);
            p.line(5.0, 12.0, 19.0, 12.0, 2.0, c);
        }
        Icon::ViewPlane => {
            p.poly(&[(3.0, 6.0), (21.0, 6.0), (21.0, 19.0), (3.0, 19.0)], 0xFFFD_E3C6);
            p.stroke(&[(3.0, 6.0), (21.0, 6.0), (21.0, 19.0), (3.0, 19.0)], 1.3, ORANGE, true);
            p.ring(12.0, 12.5, 4.0, 1.6, BLUE);
            p.circle(12.0, 12.5, 1.6, BLUE);
        }
    }
}

/// Логотип AlphaCad (128×128 RGBA), встроен в исполняемый файл.
static LOGO_RGBA: &[u8] = include_bytes!("../../assets/logo/logo-128.rgba");

/// Нарисовать логотип приложения в прямоугольник `r`.
pub fn draw_logo(fb: &mut Framebuffer, r: Rect) {
    fb.draw_rgba(LOGO_RGBA, 128, 128, r);
}
