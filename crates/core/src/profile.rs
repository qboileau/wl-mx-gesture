use std::num::NonZeroUsize;
use std::time::Duration;

use serde::Deserialize;

use crate::action::Action;
use crate::geometry::Direction;
use crate::gesture::{self, Gesture, Outcome, Output, Timed, Trigger};
use crate::style::Style;
use crate::workspace::Navigation;

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub gesture: GestureSettings,
    pub bindings: Bindings,
    pub style: Style,
    pub navigation: Navigation,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GestureSettings {
    /// Raw mouse units (roughly sensor counts).
    pub swipe_threshold: f32,
    /// Raw mouse units around the wheel centre where releasing cancels.
    pub dead_zone: f32,
    pub hold_after: Duration,
    pub double_tap_within: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bindings {
    pub tap: Option<Action>,
    pub double_tap: Option<Binding>,
    pub swipe: SwipeBindings,
    pub hold: Option<Binding>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SwipeBindings {
    pub up: Option<Action>,
    pub down: Option<Action>,
    pub left: Option<Action>,
    pub right: Option<Action>,
}

impl SwipeBindings {
    pub fn get(&self, direction: Direction) -> Option<&Action> {
        match direction {
            Direction::Up => self.up.as_ref(),
            Direction::Down => self.down.as_ref(),
            Direction::Left => self.left.as_ref(),
            Direction::Right => self.right.as_ref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Binding {
    Action(Action),
    Wheel(Wheel),
}

impl Binding {
    fn timed(&self, duration: Duration) -> Timed {
        let outcome = match self {
            Binding::Action(_) => Outcome::Gesture,
            Binding::Wheel(wheel) => Outcome::Wheel { slots: wheel.slot_count() },
        };
        Timed { duration, outcome }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Wheel {
    slots: Vec<Slot>,
}

impl Wheel {
    pub fn new(slots: Vec<Slot>) -> Option<Self> {
        (!slots.is_empty()).then_some(Self { slots })
    }

    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    pub fn slot_count(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.slots.len()).expect("a wheel is built non-empty")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    pub label: String,
    /// Freedesktop icon name or path to an image.
    pub icon: Option<String>,
    pub action: Action,
}

impl Profile {
    pub fn recognizer_settings(&self) -> gesture::Settings {
        gesture::Settings {
            swipe_threshold: self.gesture.swipe_threshold,
            dead_zone: self.gesture.dead_zone,
            hold: self.bindings.hold.as_ref().map(|binding| binding.timed(self.gesture.hold_after)),
            double_tap: self.bindings.double_tap.as_ref().map(|binding| binding.timed(self.gesture.double_tap_within)),
        }
    }

    pub fn wheel(&self, trigger: Trigger) -> Option<&Wheel> {
        match self.binding(trigger) {
            Some(Binding::Wheel(wheel)) => Some(wheel),
            _ => None,
        }
    }

    pub fn wheels(&self) -> impl Iterator<Item = &Wheel> {
        [Trigger::DoubleTap, Trigger::Hold].into_iter().filter_map(|trigger| self.wheel(trigger))
    }

    pub fn action_for(&self, output: Output) -> Option<&Action> {
        let bound_action = |trigger| match self.binding(trigger) {
            Some(Binding::Action(action)) => Some(action),
            _ => None,
        };
        match output {
            Output::Gesture(Gesture::Tap) => self.bindings.tap.as_ref(),
            Output::Gesture(Gesture::DoubleTap) => bound_action(Trigger::DoubleTap),
            Output::Gesture(Gesture::Hold) => bound_action(Trigger::Hold),
            Output::Gesture(Gesture::Swipe(direction)) => self.bindings.swipe.get(direction),
            Output::WheelSelected(trigger, slot) => {
                self.wheel(trigger).and_then(|wheel| wheel.slots().get(slot)).map(|slot| &slot.action)
            }
            Output::WheelOpened(_) | Output::WheelHovered(_) | Output::WheelCancelled => None,
        }
    }

    fn binding(&self, trigger: Trigger) -> Option<&Binding> {
        match trigger {
            Trigger::DoubleTap => self.bindings.double_tap.as_ref(),
            Trigger::Hold => self.bindings.hold.as_ref(),
        }
    }
}
