//! TOML file format, parsed once into a fully resolved [`Profile`].

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use crate::action::Action;
use crate::profile::{Bindings, GestureSettings, HoldBinding, Profile, Slot, SwipeBindings, Wheel};

const DEFAULT_SWIPE_THRESHOLD: f32 = 60.0;
const DEFAULT_DEAD_ZONE: f32 = 40.0;
const DEFAULT_HOLD_MS: u64 = 700;
const DEFAULT_DOUBLE_TAP_MS: u64 = 250;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(transparent)]
    Syntax(#[from] toml::de::Error),
    #[error("`gesture.{0}` must be greater than zero")]
    NotPositive(&'static str),
    #[error("`bindings.hold` uses wheel `{0}`, which is not defined as `[wheel.{0}]`")]
    UnknownWheel(String),
    #[error("wheel `{0}` needs at least one slot")]
    EmptyWheel(String),
}

pub fn parse(source: &str) -> Result<Profile, ConfigError> {
    let RawConfig { gesture, bindings, mut wheel } = toml::from_str(source)?;
    let hold = match bindings.hold {
        None => None,
        Some(RawHold::Action(action)) => Some(HoldBinding::Action(action)),
        Some(RawHold::Wheel(WheelRef { wheel: name })) => {
            let raw = wheel.remove(&name).ok_or_else(|| ConfigError::UnknownWheel(name.clone()))?;
            Some(HoldBinding::Wheel(Wheel::new(raw.slots).ok_or(ConfigError::EmptyWheel(name))?))
        }
    };
    Ok(Profile {
        gesture: gesture.validate()?,
        bindings: Bindings { tap: bindings.tap, double_tap: bindings.double_tap, swipe: bindings.swipe, hold },
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
            swipe_threshold: positive(self.swipe_threshold, "swipe_threshold")?,
            dead_zone: positive(self.dead_zone, "dead_zone")?,
            hold_after: Duration::from_millis(self.hold_ms),
            double_tap_within: Duration::from_millis(self.double_tap_ms),
        })
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawBindings {
    tap: Option<Action>,
    double_tap: Option<Action>,
    swipe: SwipeBindings,
    hold: Option<RawHold>,
}

#[derive(Deserialize)]
#[serde(untagged, expecting = "an action like `{ exec = \"…\" }` / `{ hyprland = \"…\" }`, or `{ wheel = \"name\" }`")]
enum RawHold {
    Wheel(WheelRef),
    Action(Action),
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
    use crate::gesture::{Gesture, Output};

    const SHIPPED_DEFAULT: &str = include_str!("../../../config/default.toml");

    #[test]
    fn shipped_default_config_resolves_its_wheel() {
        let profile = parse(SHIPPED_DEFAULT).unwrap();
        assert!(matches!(profile.bindings.hold, Some(HoldBinding::Wheel(_))));
        assert_eq!(
            profile.action_for(Output::WheelSelected(0)),
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
        assert_eq!(profile.bindings.double_tap, Some(Action::Hyprland("hl.dsp.window.close()".into())));
        assert_eq!(profile.bindings.swipe.left, Some(Action::Compositor(CompositorAction::Workspace("e-1".into()))));
    }

    #[test]
    fn unknown_action_explains_the_choices() {
        let error = parse("[bindings]\ntap = \"close_windw\"").unwrap_err().to_string();
        assert!(error.contains("close_window"), "{error}");
    }

    #[test]
    fn unknown_wheel_is_rejected() {
        let error = parse("[bindings]\nhold = { wheel = \"missing\" }").unwrap_err();
        assert!(matches!(error, ConfigError::UnknownWheel(name) if name == "missing"));
    }

    #[test]
    fn empty_wheel_is_rejected() {
        let error = parse("[bindings]\nhold = { wheel = \"w\" }\n[wheel.w]\nslots = []").unwrap_err();
        assert!(matches!(error, ConfigError::EmptyWheel(_)));
    }

    #[test]
    fn non_positive_threshold_is_rejected() {
        assert!(matches!(parse("[gesture]\ndead_zone = 0").unwrap_err(), ConfigError::NotPositive("dead_zone")));
    }

    #[test]
    fn typos_are_rejected() {
        assert!(parse("[bindings.swipe]\nleft = { exce = \"x\" }").is_err());
        assert!(parse("[gesture]\nhold = 300").is_err());
    }
}
