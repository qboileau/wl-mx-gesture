use std::f32::consts::TAU;

use tiny_skia::{
    Color as SkiaColor, FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, PremultipliedColorU8, Transform,
};
use wlmg_core::geometry::{Vector, direction, slot_angle};
use wlmg_core::profile::{Slot, Wheel};
use wlmg_core::style::{Color, Style};

use crate::Error;
use crate::icons::Icons;
use crate::text::Fonts;

/// Room around the ring, in logical pixels, so anti-aliased edges are not clipped.
const MARGIN: f32 = 4.0;
const ARC_SEGMENTS_PER_TURN: f32 = 180.0;
const ICON_LABEL_GAP: f32 = 4.0;

pub(crate) struct Renderer {
    style: Style,
    fonts: Fonts,
    icons: Icons,
}

impl Renderer {
    pub fn new(style: Style) -> Result<Self, Error> {
        let fonts = Fonts::load(style.font.as_deref())?;
        let icons = Icons::new(style.icon_theme.clone());
        Ok(Self { style, fonts, icons })
    }

    /// Logical radius of the square the wheel is drawn in.
    pub fn radius(&self) -> f32 {
        self.style.outer_radius + MARGIN
    }

    /// The wheel centred in a square pixmap of side `2 * radius() * scale`.
    pub fn render(&mut self, wheel: &Wheel, hovered: Option<usize>, scale: f32) -> Option<Pixmap> {
        let side = (2.0 * self.radius() * scale).ceil() as u32;
        let mut pixmap = Pixmap::new(side, side)?;
        let centre = Vector::new(side as f32 / 2.0, side as f32 / 2.0);
        let (outer, inner) = (self.style.outer_radius * scale, self.style.inner_radius * scale);

        let mut ring = PathBuilder::new();
        ring.push_circle(centre.x, centre.y, outer);
        ring.push_circle(centre.x, centre.y, inner);
        fill(&mut pixmap, ring, self.style.background, FillRule::EvenOdd);

        let slots = wheel.slot_count();
        let sector = TAU / slots.get() as f32;
        if let Some(slot) = hovered {
            let middle = slot_angle(slot, slots);
            let highlight = annular_sector(centre, inner, outer, middle - sector / 2.0, middle + sector / 2.0);
            fill(&mut pixmap, highlight, self.style.highlight, FillRule::Winding);
        }
        for (index, slot) in wheel.slots().iter().enumerate() {
            let anchor = centre + direction(slot_angle(index, slots)) * ((outer + inner) / 2.0);
            let colour = if hovered == Some(index) { self.style.highlighted_text } else { self.style.text };
            self.draw_slot(&mut pixmap, slot, anchor, colour, scale);
        }
        Some(pixmap)
    }

    fn draw_slot(&mut self, pixmap: &mut Pixmap, slot: &Slot, anchor: Vector, colour: Color, scale: f32) {
        let font_size = self.style.font_size * scale;
        let icon_size = (self.style.icon_size * scale).round() as u32;
        let Some(icon) = slot.icon.as_deref().and_then(|name| self.icons.get(name, icon_size)) else {
            return self.fonts.draw(pixmap, &slot.label, anchor, font_size, colour);
        };
        let tinted = self.style.tint_icons.then(|| tint(icon, colour));
        let icon = tinted.as_ref().unwrap_or(icon);
        let gap = ICON_LABEL_GAP * scale;
        let top = anchor.y - (icon.height() as f32 + gap + font_size) / 2.0;
        let left = anchor.x - icon.width() as f32 / 2.0;
        let paint = PixmapPaint::default();
        pixmap.draw_pixmap(left.round() as i32, top.round() as i32, icon.as_ref(), &paint, Transform::identity(), None);
        let label_centre = Vector::new(anchor.x, top + icon.height() as f32 + gap + font_size / 2.0);
        self.fonts.draw(pixmap, &slot.label, label_centre, font_size, colour);
    }
}

/// The icon's shape filled with `colour`, keeping only its coverage.
fn tint(icon: &Pixmap, colour: Color) -> Pixmap {
    let mut tinted = icon.clone();
    for pixel in tinted.pixels_mut() {
        let alpha = u16::from(pixel.alpha()) * u16::from(colour.alpha) / 255;
        let channel = |value: u8| (u16::from(value) * alpha / 255) as u8;
        let coloured = PremultipliedColorU8::from_rgba(
            channel(colour.red),
            channel(colour.green),
            channel(colour.blue),
            alpha as u8,
        );
        *pixel = coloured.unwrap_or(PremultipliedColorU8::TRANSPARENT);
    }
    tinted
}

fn fill(pixmap: &mut Pixmap, path: PathBuilder, colour: Color, rule: FillRule) {
    let Some(path) = path.finish() else { return };
    let mut paint = Paint::default();
    paint.set_color(SkiaColor::from_rgba8(colour.red, colour.green, colour.blue, colour.alpha));
    paint.anti_alias = true;
    pixmap.fill_path(&path, &paint, rule, Transform::identity(), None);
}

fn annular_sector(centre: Vector, inner: f32, outer: f32, from: f32, to: f32) -> PathBuilder {
    let segments = ((to - from) / TAU * ARC_SEGMENTS_PER_TURN).ceil().max(1.0) as usize;
    let at = |radius: f32, step: usize| centre + direction(from + (to - from) * step as f32 / segments as f32) * radius;
    let mut path = PathBuilder::new();
    let start = at(outer, 0);
    path.move_to(start.x, start.y);
    for point in (1..=segments).map(|step| at(outer, step)).chain((0..=segments).rev().map(|step| at(inner, step))) {
        path.line_to(point.x, point.y);
    }
    path.close();
    path
}
