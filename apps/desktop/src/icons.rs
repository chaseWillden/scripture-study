//! Small line icons, drawn with the painter so they stay crisp at any scale.

use eframe::egui::{pos2, vec2, Color32, Painter, Pos2, Rect, Stroke, StrokeKind};

const WIDTH: f32 = 1.4;

/// A window with a sidebar: rounded rectangle split by a vertical line.
pub fn sidebar(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    let rect = Rect::from_center_size(center, vec2(16.0, 13.0));
    painter.rect_stroke(rect, 3.0, stroke, StrokeKind::Middle);
    let x = rect.left() + 5.5;
    painter.vline(x, rect.y_range().shrink(0.5), stroke);
}

/// A magnifying glass.
pub fn search(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    let lens = center + vec2(-1.5, -1.5);
    painter.circle_stroke(lens, 5.0, stroke);
    let d = 5.0 / std::f32::consts::SQRT_2;
    painter.line_segment([lens + vec2(d, d), lens + vec2(d + 4.0, d + 4.0)], stroke);
}

/// A square with a pencil stroke through its corner ("compose").
pub fn compose(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    let (l, t, r, b) = (
        center.x - 7.0,
        center.y - 6.0,
        center.x + 6.0,
        center.y + 7.0,
    );
    // Open square: the top-right corner is left for the pencil.
    painter.line(
        vec![
            pos2(l + 6.0, t),
            pos2(l, t),
            pos2(l, b),
            pos2(r, b),
            pos2(r, t + 6.0),
        ],
        stroke,
    );
    painter.line_segment(
        [pos2(center.x - 2.0, center.y + 2.0), pos2(r + 1.5, t - 1.5)],
        stroke,
    );
}

/// An open book.
pub fn book(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    let p = |x: f32, y: f32| center + vec2(x, y);
    painter.line(
        vec![
            p(0.0, -6.2),
            p(-7.2, -4.6),
            p(-7.2, 5.4),
            p(0.0, 7.0),
            p(7.2, 5.4),
            p(7.2, -4.6),
            p(0.0, -6.2),
        ],
        stroke,
    );
}

/// A chevron pointing left ("back").
pub fn chevron_left(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    painter.line(
        vec![
            center + vec2(1.5, -3.5),
            center + vec2(-2.0, 0.0),
            center + vec2(1.5, 3.5),
        ],
        stroke,
    );
}

/// A page with lines of text ("all notes").
pub fn notes(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    let page = Rect::from_center_size(center, vec2(14.0, 17.0));
    painter.rect_stroke(page, 3.0, stroke, StrokeKind::Middle);
    for (n, width) in [8.0, 8.0, 5.0].into_iter().enumerate() {
        let y = page.top() + 5.0 + 3.5 * n as f32;
        let x = page.left() + 3.5;
        painter.hline(x..=x + width, y, stroke);
    }
}

/// A folder with a tab ("organize").
pub fn folder(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    let (l, r) = (center.x - 8.0, center.x + 8.0);
    let (t, b) = (center.y - 6.0, center.y + 6.5);
    let tab = l + 6.0;
    painter.line(
        vec![
            pos2(l, b - 1.5),
            pos2(l, t + 1.5),
            pos2(l + 1.5, t),
            pos2(tab - 1.0, t),
            pos2(tab + 1.5, t + 2.5),
            pos2(r - 1.5, t + 2.5),
            pos2(r, t + 4.0),
            pos2(r, b - 1.5),
            pos2(r - 1.5, b),
            pos2(l + 1.5, b),
            pos2(l, b - 1.5),
        ],
        stroke,
    );
}

/// A folder with a plus sign.
pub fn new_folder(painter: &Painter, center: Pos2, color: Color32) {
    folder(painter, center, color);
    let stroke = Stroke::new(WIDTH, color);
    let c = center + vec2(0.0, 1.5);
    painter.line_segment([c - vec2(3.0, 0.0), c + vec2(3.0, 0.0)], stroke);
    painter.line_segment([c - vec2(0.0, 3.0), c + vec2(0.0, 3.0)], stroke);
}

/// A disclosure chevron: pointing right when collapsed, down when expanded.
pub fn chevron(painter: &Painter, center: Pos2, expanded: bool, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    let points = if expanded {
        vec![
            center + vec2(-3.5, -1.5),
            center + vec2(0.0, 2.0),
            center + vec2(3.5, -1.5),
        ]
    } else {
        vec![
            center + vec2(-1.5, -3.5),
            center + vec2(2.0, 0.0),
            center + vec2(-1.5, 3.5),
        ]
    };
    painter.line(points, stroke);
}

/// A small page (a note in the folder tree).
pub fn page(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH * 0.9, color);
    let rect = Rect::from_center_size(center, vec2(10.0, 13.0));
    painter.rect_stroke(rect, 2.0, stroke, StrokeKind::Middle);
}

/// A pencil pointing down-left ("rename").
pub fn pencil(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    let p = |x: f32, y: f32| center + vec2(x, y);
    painter.line(
        vec![
            p(-6.0, 6.0),
            p(-5.3, 2.8),
            p(3.2, -5.7),
            p(5.7, -3.2),
            p(-2.8, 5.3),
            p(-6.0, 6.0),
        ],
        stroke,
    );
    painter.line_segment([p(1.2, -3.7), p(3.7, -1.2)], stroke);
}

/// A trash can with a lid.
pub fn trash(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    let p = |x: f32, y: f32| center + vec2(x, y);
    painter.hline(center.x - 6.5..=center.x + 6.5, center.y - 4.0, stroke);
    painter.line(
        vec![p(-2.0, -4.0), p(-2.0, -6.5), p(2.0, -6.5), p(2.0, -4.0)],
        stroke,
    );
    painter.line(
        vec![p(-5.0, -4.0), p(-4.2, 6.5), p(4.2, 6.5), p(5.0, -4.0)],
        stroke,
    );
    painter.vline(center.x - 1.5, center.y - 1.0..=center.y + 3.8, stroke);
    painter.vline(center.x + 1.5, center.y - 1.0..=center.y + 3.8, stroke);
}

/// A folder with an arrow going in ("move to").
pub fn move_to(painter: &Painter, center: Pos2, color: Color32) {
    folder(painter, center, color);
    let stroke = Stroke::new(WIDTH, color);
    let c = center + vec2(0.0, 1.5);
    painter.line_segment([c - vec2(3.5, 0.0), c + vec2(3.0, 0.0)], stroke);
    painter.line(
        vec![c + vec2(0.5, -2.5), c + vec2(3.0, 0.0), c + vec2(0.5, 2.5)],
        stroke,
    );
}

/// A check mark.
pub fn check(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    painter.line(
        vec![
            center + vec2(-4.0, 0.0),
            center + vec2(-1.2, 3.0),
            center + vec2(4.0, -3.0),
        ],
        stroke,
    );
}

/// A price-tag shape with a hole ("tags").
pub fn tag(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH * 0.9, color);
    let c = center;
    painter.line(
        vec![
            c + vec2(-6.0, -6.0),
            c + vec2(0.5, -6.0),
            c + vec2(6.5, 0.0),
            c + vec2(0.0, 6.5),
            c + vec2(-6.0, 0.5),
            c + vec2(-6.0, -6.0),
        ],
        stroke,
    );
    painter.circle_filled(c + vec2(-2.8, -2.8), 1.1, color);
}

/// Head and shoulders ("people").
pub fn person(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH * 0.9, color);
    painter.circle_stroke(center + vec2(0.0, -3.0), 3.0, stroke);
    let shoulders: Vec<Pos2> = (0..=12)
        .map(|n| {
            let t = std::f32::consts::PI * (n as f32 / 12.0);
            center + vec2(-6.0 * t.cos(), 6.5 - 4.0 * t.sin())
        })
        .collect();
    painter.line(shoulders, stroke);
}

/// A clock face.
pub fn clock(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH * 0.9, color);
    painter.circle_stroke(center, 5.5, stroke);
    painter.line(
        vec![center + vec2(0.0, -3.0), center, center + vec2(2.2, 1.6)],
        stroke,
    );
}

/// A plus sign.
pub fn plus(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    painter.line_segment([center - vec2(4.5, 0.0), center + vec2(4.5, 0.0)], stroke);
    painter.line_segment([center - vec2(0.0, 4.5), center + vec2(0.0, 4.5)], stroke);
}

/// A marker pen ("highlight").
pub fn highlight(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    let tip = center + vec2(-6.5, 5.0);
    let end = center + vec2(5.5, -6.0);
    painter.line_segment([tip, end], Stroke::new(3.2, color));
    painter.line_segment([end, end + vec2(2.2, -2.2)], stroke);
    painter.line_segment([tip + vec2(-1.2, 1.2), tip + vec2(2.4, 1.2)], stroke);
}

/// A line under a short stroke ("underline").
pub fn underline(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    painter.hline(center.x - 5.5..=center.x + 5.5, center.y - 1.5, stroke);
    painter.hline(
        center.x - 6.5..=center.x + 6.5,
        center.y + 4.0,
        Stroke::new(1.8, color),
    );
}

/// A small ×.
pub fn close(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    let d = 3.2;
    painter.line_segment([center + vec2(-d, -d), center + vec2(d, d)], stroke);
    painter.line_segment([center + vec2(-d, d), center + vec2(d, -d)], stroke);
}

/// A box with an arrow leaving its corner ("reveal elsewhere").
pub fn reveal(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    let p = |x: f32, y: f32| center + vec2(x, y);
    painter.line(
        vec![
            p(-1.0, -6.0),
            p(-6.0, -6.0),
            p(-6.0, 6.0),
            p(6.0, 6.0),
            p(6.0, 1.0),
        ],
        stroke,
    );
    painter.line_segment([p(-1.0, 1.0), p(6.5, -6.5)], stroke);
    painter.line(vec![p(1.5, -6.5), p(6.5, -6.5), p(6.5, -1.5)], stroke);
}

/// Two overlapping pages ("copy").
pub fn copy(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    let back = Rect::from_min_size(center + vec2(-6.5, -6.5), vec2(9.0, 10.0));
    let front = Rect::from_min_size(center + vec2(-2.5, -3.0), vec2(9.0, 10.0));
    painter.line(
        vec![
            pos2(back.left(), back.bottom() - 1.0),
            pos2(back.left(), back.top() + 1.5),
            pos2(back.left() + 1.5, back.top()),
            pos2(back.right() - 1.5, back.top()),
            pos2(back.right(), back.top() + 1.0),
        ],
        stroke,
    );
    painter.rect_stroke(front, 2.0, stroke, StrokeKind::Middle);
}

/// Two chain links ("link").
pub fn link(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH, color);
    // Two capsules on a diagonal, overlapping in the middle.
    let along = vec2(1.0, -1.0) / std::f32::consts::SQRT_2;
    let across = vec2(along.y, -along.x);
    let (half, r) = (2.6, 2.4);
    for offset in [-2.4, 2.4] {
        let c = center + along * offset;
        let mut points = Vec::new();
        for (end, start_angle) in [(half, -90.0_f32), (-half, 90.0)] {
            for step in 0..=8 {
                let a = (start_angle + step as f32 * 22.5).to_radians();
                points.push(c + along * (end + r * a.cos()) + across * (r * a.sin()));
            }
        }
        points.push(points[0]);
        painter.line(points, stroke);
    }
}

/// A chain link with a slash through it ("remove link").
pub fn unlink(painter: &Painter, center: Pos2, color: Color32) {
    link(painter, center, color);
    painter.line_segment(
        [center + vec2(-6.5, -6.5), center + vec2(6.5, 6.5)],
        Stroke::new(WIDTH, color),
    );
}

/// A pair of quotation marks ("citation").
pub fn quote(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(WIDTH * 1.1, color);
    for dx in [-3.5, 3.0] {
        let dot = center + vec2(dx, -1.5);
        painter.circle_filled(dot, 2.2, color);
        painter.line(
            vec![
                dot + vec2(2.1, 0.0),
                dot + vec2(1.8, 3.2),
                dot + vec2(-0.8, 5.5),
            ],
            stroke,
        );
    }
}
