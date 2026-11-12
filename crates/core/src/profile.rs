use std::num::NonZeroUsize;
use std::time::Duration;

use serde::Deserialize;

use crate::action::Action;
use crate::geometry::Direction;
use crate::gesture::{self, Gesture, Hold, HoldOutcome, Output};

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub gesture: GestureSettings,
    pub bindings: Bindings,
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
    pub double_tap: Option<Action>,
    pub swipe: SwipeBindings,
    pub hold: Option<HoldBinding>,
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
pub enum HoldBinding {
    Action(Action),
    Wheel(Wheel),
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
            hold: self.bindings.hold.as_ref().map(|binding| Hold {
                after: self.gesture.hold_after,
                outcome: match binding {
                    HoldBinding::Action(_) => HoldOutcome::Gesture,
                    HoldBinding::Wheel(wheel) => HoldOutcome::Wheel { slots: wheel.slot_count() },
                },
            }),
            double_tap: self.bindings.double_tap.as_ref().map(|_| self.gesture.double_tap_within),
        }
    }

    pub fn action_for(&self, output: Output) -> Option<&Action> {
        match (output, &self.bindings.hold) {
            (Output::Gesture(Gesture::Tap), _) => self.bindings.tap.as_ref(),
            (Output::Gesture(Gesture::DoubleTap), _) => self.bindings.double_tap.as_ref(),
            (Output::Gesture(Gesture::Swipe(direction)), _) => self.bindings.swipe.get(direction),
            (Output::Gesture(Gesture::Hold), Some(HoldBinding::Action(action))) => Some(action),
            (Output::WheelSelected(slot), Some(HoldBinding::Wheel(wheel))) => {
                wheel.slots().get(slot).map(|slot| &slot.action)
            }
            _ => None,
        }
    }
}
