use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use log::{info, warn};
use resvg::usvg;
use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};

const FALLBACK_THEME: &str = "hicolor";
const THEME_INDEX: &str = "index.theme";
const DEFAULT_DATA_DIRS: &str = "/usr/local/share:/usr/share";

/// Freedesktop icons rasterised at the sizes asked for, loaded once.
pub(crate) struct Icons {
    theme: String,
    loaded: HashMap<(String, u32), Option<Pixmap>>,
}

impl Icons {
    pub fn new(theme: Option<String>) -> Self {
        let theme = theme.or_else(detect_theme).map(theme_directory).unwrap_or_else(|| FALLBACK_THEME.to_owned());
        info!("icon theme: {theme}");
        Self { theme, loaded: HashMap::new() }
    }

    /// `name` is an icon name from the theme or an absolute path to an SVG or PNG file.
    pub fn get(&mut self, name: &str, size: u32) -> Option<&Pixmap> {
        let theme = &self.theme;
        self.loaded
            .entry((name.to_owned(), size))
            .or_insert_with(|| {
                let icon = load(name, size, theme);
                if icon.is_none() {
                    warn!("icon `{name}` not found in theme `{theme}` or not readable");
                }
                icon
            })
            .as_ref()
    }
}

fn load(name: &str, size: u32, theme: &str) -> Option<Pixmap> {
    let path = if Path::new(name).is_absolute() {
        PathBuf::from(name)
    } else {
        freedesktop_icons::lookup(name).with_size(u16::try_from(size).ok()?).with_theme(theme).with_cache().find()?
    };
    let data = fs::read(&path).ok()?;
    let mut icon = Pixmap::new(size, size)?;
    if path.extension().is_some_and(|extension| extension == "svg") {
        let tree = usvg::Tree::from_data(&data, &usvg::Options::default()).ok()?;
        let factor = size as f32 / tree.size().width().max(tree.size().height());
        resvg::render(&tree, Transform::from_scale(factor, factor), &mut icon.as_mut());
    } else {
        let source = Pixmap::decode_png(&data).ok()?;
        let factor = size as f32 / source.width().max(source.height()) as f32;
        let paint = PixmapPaint { quality: FilterQuality::Bicubic, ..PixmapPaint::default() };
        icon.draw_pixmap(0, 0, source.as_ref(), &paint, Transform::from_scale(factor, factor), None);
    }
    Some(icon)
}

/// The theme of the desktop settings: GTK 4, GTK 3, then KDE.
fn detect_theme() -> Option<String> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    let read = |file: &str| fs::read_to_string(config.join(file)).ok();
    read("gtk-4.0/settings.ini")
        .and_then(|text| ini_value(&text, "Settings", "gtk-icon-theme-name"))
        .or_else(|| read("gtk-3.0/settings.ini").and_then(|text| ini_value(&text, "Settings", "gtk-icon-theme-name")))
        .or_else(|| read("kdeglobals").and_then(|text| ini_value(&text, "Icons", "Theme")))
        .or_else(freedesktop_icons::default_theme_gtk)
}

/// Desktop settings may hold a theme's display name (`Breeze Dark`) while lookups need its directory
/// (`breeze-dark`); maps the former to the latter by reading each theme's `index.theme`.
fn theme_directory(name: String) -> String {
    let bases = icon_bases();
    if bases.iter().any(|base| base.join(&name).join(THEME_INDEX).is_file()) {
        return name;
    }
    bases
        .iter()
        .filter_map(|base| fs::read_dir(base).ok())
        .flatten()
        .filter_map(Result::ok)
        .find(|entry| {
            fs::read_to_string(entry.path().join(THEME_INDEX))
                .is_ok_and(|index| ini_value(&index, "Icon Theme", "Name").as_deref() == Some(name.as_str()))
        })
        .and_then(|entry| entry.file_name().into_string().ok())
        .unwrap_or(name)
}

/// Icon theme base directories, per the freedesktop icon theme specification.
fn icon_bases() -> Vec<PathBuf> {
    let home = env::var_os("HOME").map(PathBuf::from);
    let data_home =
        env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| home.as_ref().map(|home| home.join(".local/share")));
    let data_dirs = env::var("XDG_DATA_DIRS").unwrap_or_else(|_| DEFAULT_DATA_DIRS.to_owned());
    home.map(|home| home.join(".icons"))
        .into_iter()
        .chain(data_home.into_iter().chain(data_dirs.split(':').map(PathBuf::from)).map(|dir| dir.join("icons")))
        .collect()
}

fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|line| line.strip_suffix(']')) {
            in_section = name == section;
        } else if in_section
            && let Some((name, value)) = line.split_once('=')
            && name.trim() == key
        {
            return Some(value.trim().to_owned()).filter(|value| !value.is_empty());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_key_of_the_right_section_only() {
        let text = "[General]\nTheme=wrong\n\n[Icons]\n Theme = Breeze Dark \n";
        assert_eq!(ini_value(text, "Icons", "Theme"), Some("Breeze Dark".to_owned()));
        assert_eq!(ini_value(text, "Settings", "Theme"), None);
    }
}
