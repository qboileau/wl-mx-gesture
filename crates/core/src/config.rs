//! TOML file format, parsed once into a fully resolved [`Profile`].

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use crate::action::Action;
use crate::profile::{Binding, Bindings, GestureSettings, Profile, Slot, SwipeBindings, Wheel};
use crate::style::Style;

const DEFAULT_SWIPE_THRESHOLD: f32 = 60.0;
const DEFAULT_DEAD_ZONE: f32 = 40.0;
const DEFAULT_HOLD_MS: u64 = 700;
const DEFAULT_DOUBLE_TAP_MS: u64 = 250;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(transparent)]
    Syntax(#[from] toml::de::Error),
    #[error("`{0}` must be greater than zero")]
    NotPositive(&'static str),
    #[error("`bindings.{key}` uses wheel `{name}`, which is not defined as `[wheel.{name}]`")]
    UnknownWheel { key: &'static str, name: String },
    #[error("wheel `{0}` needs at least one slot")]
    EmptyWheel(String),
    #[error("`style.inner_radius` must be smaller than `style.outer_radius`")]
    InnerRadiusTooLarge,
}

pub fn parse(source: &str) -> Result<Profile, ConfigError> {
    let RawConfig { gesture, bindings, wheel: wheels, style } = toml::from_str(source)?;
    let resolve = |binding: Option<RawBinding>, key| binding.map(|binding| binding.resolve(&wheels, key)).transpose();
    Ok(Profile {
        gesture: gesture.validate()?,
        style: validate_style(style)?,
        bindings: Bindings {
            tap: bindings.tap,
            double_tap: resolve(bindings.double_tap, "double_tap")?,
            swipe: bindings.swipe,
            hold: resolve(bindings.hold, "hold")?,
        },
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    gesture: RawGesture,
    #[serde(default)]
    bindings: RawBindings,
    #[serde(default)]
    wheel: HashMap<String, RawWheel>,
    #[serde(default)]
    style: Style,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawGesture {
    swipe_threshold: f32,
    dead_zone: f32,
    hold_ms: u64,
    double_tap_ms: u64,
}

impl Default for RawGesture {
    fn default() -> Self {
        Self {
            swipe_threshold: DEFAULT_SWIPE_THRESHOLD,
            dead_zone: DEFAULT_DEAD_ZONE,
            hold_ms: DEFAULT_HOLD_MS,
            double_tap_ms: DEFAULT_DOUBLE_TAP_MS,
        }
    }
}

impl RawGesture {
    fn validate(self) -> Result<GestureSettings, ConfigError> {
        let positive = |value: f32, name| if value > 0.0 { Ok(value) } else { Err(ConfigError::NotPositive(name)) };
        Ok(GestureSettings {
            swipe_threshold: positive(self.swipe_threshold, "gesture.swipe_threshold")?,
            dead_zone: positive(self.dead_zone, "gesture.dead_zone")?,
            hold_after: Duration::from_millis(self.hold_ms),
            double_tap_within: Duration::from_millis(self.double_tap_ms),
        })
    }
}

fn validate_style(style: Style) -> Result<Style, ConfigError> {
    let sizes = [
        (style.inner_radius, "style.inner_radius"),
        (style.font_size, "style.font_size"),
        (style.icon_size, "style.icon_size"),
    ];
    if let Some((_, name)) = sizes.into_iter().find(|&(size, _)| size <= 0.0) {
        return Err(ConfigError::NotPositive(name));
    }
    if style.inner_radius >= style.outer_radius {
        return Err(ConfigError::InnerRadiusTooLarge);
    }
    Ok(style)
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawBindings {
    tap: Option<Action>,
    double_tap: Option<RawBinding>,
    swipe: SwipeBindings,
    hold: Option<RawBinding>,
}

#[derive(Deserialize)]
#[serde(untagged, expecting = "an action like `{ exec = \"…\" }` / `{ hyprland = \"…\" }`, or `{ wheel = \"name\" }`")]
enum RawBinding {
    Wheel(WheelRef),
    Action(Action),
}

impl RawBinding {
    fn resolve(self, wheels: &HashMap<String, RawWheel>, key: &'static str) -> Result<Binding, ConfigError> {
        match self {
            RawBinding::Action(action) => Ok(Binding::Action(action)),
            RawBinding::Wheel(WheelRef { wheel: name }) => {
                let raw = wheels.get(&name).ok_or_else(|| ConfigError::UnknownWheel { key, name: name.clone() })?;
                Ok(Binding::Wheel(Wheel::new(raw.slots.clone()).ok_or(ConfigError::EmptyWheel(name))?))
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WheelRef {
    wheel: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWheel {
    slots: Vec<Slot>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::CompositorAction;
    use crate::gesture::{Gesture, Output, Trigger};
    use crate::style::Color;

    const SHIPPED_DEFAULT: &str = include_str!("../../../config/default.toml");

    #[test]
    fn shipped_default_config_opens_its_wheel_on_double_tap() {
        let profile = parse(SHIPPED_DEFAULT).unwrap();
        assert!(matches!(profile.bindings.double_tap, Some(Binding::Wheel(_))));
        assert_eq!(
            profile.action_for(Output::WheelSelected(Trigger::DoubleTap, 0)),
            Some(&Action::Compositor(CompositorAction::CloseWindow))
        );
    }

    #[test]
    fn empty_config_uses_defaults_and_binds_nothing() {
        let profile = parse("").unwrap();
        assert_eq!(profile.gesture.swipe_threshold, DEFAULT_SWIPE_THRESHOLD);
        assert_eq!(profile.action_for(Output::Gesture(Gesture::Tap)), None);
        assert_eq!(profile.recognizer_settings().hold, None);
    }

    #[test]
    fn hold_can_be_a_plain_action() {
        let profile = parse("[bindings]\nhold = { exec = \"notify-send hi\" }").unwrap();
        assert_eq!(profile.action_for(Output::Gesture(Gesture::Hold)), Some(&Action::Exec("notify-send hi".into())));
    }

    #[test]
    fn actions_are_tables_or_bare_names() {
        let profile = parse(
            "[bindings]\ntap = \"close_window\"\ndouble_tap = { hyprland = \"hl.dsp.window.close()\" }\n\
             [bindings.swipe]\nleft = { workspace = \"e-1\" }",
        )
        .unwrap();
        assert_eq!(profile.bindings.tap, Some(Action::Compositor(CompositorAction::CloseWindow)));
        assert_eq!(
            profile.action_for(Output::Gesture(Gesture::DoubleTap)),
            Some(&Action::Hyprland("hl.dsp.window.close()".into()))
        );
        assert_eq!(profile.bindings.swipe.left, Some(Action::Compositor(CompositorAction::Workspace("e-1".into()))));
    }

    #[test]
    fn unknown_action_explains_the_choices() {
        let error = parse("[bindings]\ntap = \"close_windw\"").unwrap_err().to_string();
        assert!(error.contains("close_window"), "{error}");
    }

    #[test]
    fn unknown_wheel_is_rejected() {
        let error = parse("[bindings]\ndouble_tap = { wheel = \"missing\" }").unwrap_err();
        assert!(matches!(error, ConfigError::UnknownWheel { key: "double_tap", name } if name == "missing"));
    }

    #[test]
    fn empty_wheel_is_rejected() {
        let error = parse("[bindings]\nhold = { wheel = \"w\" }\n[wheel.w]\nslots = []").unwrap_err();
        assert!(matches!(error, ConfigError::EmptyWheel(_)));
    }

    #[test]
    fn non_positive_threshold_is_rejected() {
        assert!(matches!(
            parse("[gesture]\ndead_zone = 0").unwrap_err(),
            ConfigError::NotPositive("gesture.dead_zone")
        ));
    }

    #[test]
    fn style_overrides_defaults_and_is_validated() {
        let profile = parse("[style]\nouter_radius = 200\nhighlight = \"#ff000080\"").unwrap();
        assert_eq!(profile.style.outer_radius, 200.0);
        assert_eq!(profile.style.highlight, Color::rgba(0xFF, 0, 0, 0x80));
        assert_eq!(profile.style.inner_radius, Style::default().inner_radius);
        assert!(matches!(parse("[style]\ninner_radius = 300").unwrap_err(), ConfigError::InnerRadiusTooLarge));
        assert!(parse("[style]\ntext = \"red\"").unwrap_err().to_string().contains("#rrggbb"));
    }

    #[test]
    fn hold_and_double_tap_can_open_different_wheels() {
        let profile = parse(
            "[bindings]\nhold = { wheel = \"a\" }\ndouble_tap = { wheel = \"b\" }\n\
             [[wheel.a.slots]]\nlabel = \"A\"\naction = \"maximize\"\n\
             [[wheel.b.slots]]\nlabel = \"B\"\naction = \"fullscreen\"",
        )
        .unwrap();
        assert_eq!(
            profile.action_for(Output::WheelSelected(Trigger::Hold, 0)),
            Some(&Action::Compositor(CompositorAction::Maximize))
        );
        assert_eq!(
            profile.action_for(Output::WheelSelected(Trigger::DoubleTap, 0)),
            Some(&Action::Compositor(CompositorAction::Fullscreen))
        );
    }

    #[test]
    fn typos_are_rejected() {
        assert!(parse("[bindings.swipe]\nleft = { exce = \"x\" }").is_err());
        assert!(parse("[gesture]\nhold = 300").is_err());
    }
}
