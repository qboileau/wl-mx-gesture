use std::collections::HashMap;

use ab_glyph::{Font, FontArc, FontVec, Glyph, PxScale, ScaleFont, point};
use tiny_skia::{Pixmap, PremultipliedColorU8};
use wlmg_core::geometry::Vector;
use wlmg_core::style::Color;

use crate::Error;

/// Tried in order when no font is configured; fontdb does not resolve fontconfig's generic aliases.
const DEFAULT_FAMILIES: [&str; 5] = ["Noto Sans", "DejaVu Sans", "Cantarell", "Liberation Sans", "Inter"];
const REGULAR_WEIGHT: fontdb::Weight = fontdb::Weight::NORMAL;

/// The primary font plus, per character it lacks, the first installed font that has it.
pub(crate) struct Fonts {
    database: fontdb::Database,
    loaded: Vec<FontArc>,
    fallback: HashMap<char, Option<usize>>,
}

impl Fonts {
    pub fn load(family: Option<&str>) -> Result<Self, Error> {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();
        let find = |name| database.query(&fontdb::Query { families: &[name], ..fontdb::Query::default() });
        let id = match family {
            Some(name) => find(fontdb::Family::Name(name)).ok_or_else(|| Error::FontNotFound(name.to_owned()))?,
            None => DEFAULT_FAMILIES
                .iter()
                .find_map(|&name| find(fontdb::Family::Name(name)))
                .or_else(|| find(fontdb::Family::SansSerif))
                .or_else(|| database.faces().next().map(|face| face.id))
                .ok_or(Error::NoFont)?,
        };
        let primary = font(&database, id).ok_or(Error::NoFont)?;
        Ok(Self { database, loaded: vec![primary], fallback: HashMap::new() })
    }

    /// Draws one line of text centred on `centre`.
    pub fn draw(&mut self, pixmap: &mut Pixmap, text: &str, centre: Vector, size: f32, colour: Color) {
        let scale = PxScale::from(size);
        let mut glyphs: Vec<(usize, Glyph)> = Vec::new();
        let mut caret = 0.0;
        for character in text.chars() {
            let index = self.font_index(character);
            let font = self.loaded[index].as_scaled(scale);
            let id = font.glyph_id(character);
            if let Some((previous_index, previous)) = glyphs.last()
                && *previous_index == index
            {
                caret += font.kern(previous.id, id);
            }
            glyphs.push((index, id.with_scale_and_position(scale, point(caret, 0.0))));
            caret += font.h_advance(id);
        }
        let primary = self.loaded[0].as_scaled(scale);
        let origin = point(centre.x - caret / 2.0, centre.y + (primary.ascent() + primary.descent()) / 2.0);
        for (index, mut glyph) in glyphs {
            glyph.position += origin;
            let Some(outline) = self.loaded[index].outline_glyph(glyph) else { continue };
            let bounds = outline.px_bounds();
            outline.draw(|x, y, coverage| {
                blend(pixmap, bounds.min.x as i32 + x as i32, bounds.min.y as i32 + y as i32, colour, coverage)
            });
        }
    }

    fn font_index(&mut self, character: char) -> usize {
        if covers(&self.loaded[0], character) {
            return 0;
        }
        if let Some(&index) = self.fallback.get(&character) {
            return index.unwrap_or(0);
        }
        let index = self.find_covering(character);
        self.fallback.insert(character, index);
        index.unwrap_or(0)
    }

    fn find_covering(&mut self, character: char) -> Option<usize> {
        if let Some(index) = self.loaded.iter().position(|font| covers(font, character)) {
            return Some(index);
        }
        let mut faces: Vec<&fontdb::FaceInfo> = self.database.faces().collect();
        faces.sort_by_key(|face| (face.style != fontdb::Style::Normal, face.weight.0.abs_diff(REGULAR_WEIGHT.0)));
        let found =
            faces.into_iter().find_map(|face| font(&self.database, face.id).filter(|font| covers(font, character)))?;
        self.loaded.push(found);
        Some(self.loaded.len() - 1)
    }
}

fn font(database: &fontdb::Database, id: fontdb::ID) -> Option<FontArc> {
    database
        .with_face_data(id, |data, index| FontVec::try_from_vec_and_index(data.to_vec(), index))
        .and_then(Result::ok)
        .map(FontArc::new)
}

fn covers(font: &FontArc, character: char) -> bool {
    font.glyph_id(character).0 != 0
}

fn blend(pixmap: &mut Pixmap, x: i32, y: i32, colour: Color, coverage: f32) {
    let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else { return };
    if x >= pixmap.width() || y >= pixmap.height() {
        return;
    }
    let index = (y * pixmap.width() + x) as usize;
    let destination = pixmap.pixels()[index];
    let alpha = f32::from(colour.alpha) / 255.0 * coverage.clamp(0.0, 1.0);
    let blended_alpha = (alpha * 255.0 + f32::from(destination.alpha()) * (1.0 - alpha)) as u8;
    let over = |source: u8, destination: u8| {
        ((f32::from(source) * alpha + f32::from(destination) * (1.0 - alpha)) as u8).min(blended_alpha)
    };
    let pixel = PremultipliedColorU8::from_rgba(
        over(colour.red, destination.red()),
        over(colour.green, destination.green()),
        over(colour.blue, destination.blue()),
        blended_alpha,
    );
    if let Some(pixel) = pixel {
        pixmap.pixels_mut()[index] = pixel;
    }
}
