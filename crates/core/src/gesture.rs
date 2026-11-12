use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use crate::geometry::{Direction, Vector, slot_at};

/// Caps the wheel pointer so reversing direction never needs a long trip back.
const WHEEL_TRAVEL_IN_DEAD_ZONES: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Input {
    Pressed,
    Released,
    Moved(Vector),
    /// A [`Recognizer::deadline`] was reached without other input.
    Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    Tap,
    DoubleTap,
    Hold,
    Swipe(Direction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Gesture(Gesture),
    WheelOpened,
    WheelHovered(Option<usize>),
    WheelSelected(usize),
    WheelCancelled,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HoldOutcome {
    Gesture,
    Wheel { slots: NonZeroUsize },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hold {
    pub after: Duration,
    pub outcome: HoldOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub swipe_threshold: f32,
    pub dead_zone: f32,
    /// `None` when nothing is bound to hold: a long still press is then a tap.
    pub hold: Option<Hold>,
    /// `None` when nothing is bound to double tap, so taps fire without waiting.
    pub double_tap: Option<Duration>,
}

#[derive(Debug, Clone, Copy)]
enum State {
    Idle,
    Pressed { since: Instant, travel: Vector },
    Swiping { travel: Vector },
    AwaitingSecondTap { until: Instant },
    SecondPress,
    Holding,
    Wheel { slots: NonZeroUsize, pointer: Vector, hovered: Option<usize> },
}

pub struct Recognizer {
    settings: Settings,
    state: State,
}

impl Recognizer {
    pub fn new(settings: Settings) -> Self {
        Self { settings, state: State::Idle }
    }

    /// When to feed [`Input::Tick`] if no other input arrives first.
    pub fn deadline(&self) -> Option<Instant> {
        match self.state {
            State::Pressed { since, .. } => self.settings.hold.map(|hold| since + hold.after),
            State::AwaitingSecondTap { until } => Some(until),
            _ => None,
        }
    }

    pub fn handle(&mut self, input: Input, now: Instant) -> Option<Output> {
        let (state, output) = self.next(input, now);
        self.state = state;
        output
    }

    fn next(&self, input: Input, now: Instant) -> (State, Option<Output>) {
        let settings = &self.settings;
        match (self.state, input) {
            (State::Idle, Input::Pressed) => (State::Pressed { since: now, travel: Vector::ZERO }, None),
            (State::Pressed { since, travel }, Input::Moved(delta)) => {
                let travel = travel + delta;
                let state = if travel.length() >= settings.swipe_threshold {
                    State::Swiping { travel }
                } else {
                    State::Pressed { since, travel }
                };
                (state, None)
            }
            (State::Pressed { since, .. }, Input::Tick) => match settings.hold {
                Some(hold) if now >= since + hold.after => match hold.outcome {
                    HoldOutcome::Gesture => (State::Holding, Some(Output::Gesture(Gesture::Hold))),
                    HoldOutcome::Wheel { slots } => {
                        (State::Wheel { slots, pointer: Vector::ZERO, hovered: None }, Some(Output::WheelOpened))
                    }
                },
                _ => (self.state, None),
            },
            (State::Pressed { .. }, Input::Released) => match settings.double_tap {
                Some(window) => (State::AwaitingSecondTap { until: now + window }, None),
                None => (State::Idle, Some(Output::Gesture(Gesture::Tap))),
            },
            (State::Swiping { travel }, Input::Moved(delta)) => (State::Swiping { travel: travel + delta }, None),
            (State::Swiping { travel }, Input::Released) => (
                State::Idle,
                (travel.length() >= settings.swipe_threshold)
                    .then(|| Output::Gesture(Gesture::Swipe(Direction::of(travel)))),
            ),
            (State::AwaitingSecondTap { .. }, Input::Pressed) => (State::SecondPress, None),
            (State::AwaitingSecondTap { until }, Input::Tick) if now >= until => {
                (State::Idle, Some(Output::Gesture(Gesture::Tap)))
            }
            (State::SecondPress, Input::Released) => (State::Idle, Some(Output::Gesture(Gesture::DoubleTap))),
            (State::Holding, Input::Released) => (State::Idle, None),
            (State::Wheel { slots, pointer, hovered }, Input::Moved(delta)) => {
                let pointer = (pointer + delta).clamped(settings.dead_zone * WHEEL_TRAVEL_IN_DEAD_ZONES);
                let now_hovered = slot_at(pointer, slots, settings.dead_zone);
                (
                    State::Wheel { slots, pointer, hovered: now_hovered },
                    (now_hovered != hovered).then_some(Output::WheelHovered(now_hovered)),
                )
            }
            (State::Wheel { hovered, .. }, Input::Released) => {
                (State::Idle, Some(hovered.map_or(Output::WheelCancelled, Output::WheelSelected)))
            }
            (state, _) => (state, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOLD: Duration = Duration::from_millis(300);
    const DOUBLE_TAP: Duration = Duration::from_millis(250);
    const FOUR_SLOTS: NonZeroUsize = NonZeroUsize::new(4).unwrap();

    fn recognizer(hold: Option<HoldOutcome>, double_tap: bool) -> Recognizer {
        Recognizer::new(Settings {
            swipe_threshold: 50.0,
            dead_zone: 20.0,
            hold: hold.map(|outcome| Hold { after: HOLD, outcome }),
            double_tap: double_tap.then_some(DOUBLE_TAP),
        })
    }

    fn feed(recognizer: &mut Recognizer, inputs: &[(u64, Input)]) -> Vec<Output> {
        let start = Instant::now();
        inputs.iter().filter_map(|&(ms, input)| recognizer.handle(input, start + Duration::from_millis(ms))).collect()
    }

    fn moved(x: f32, y: f32) -> Input {
        Input::Moved(Vector::new(x, y))
    }

    #[test]
    fn tap_fires_on_release_without_double_tap_binding() {
        let outputs = feed(&mut recognizer(None, false), &[(0, Input::Pressed), (50, Input::Released)]);
        assert_eq!(outputs, [Output::Gesture(Gesture::Tap)]);
    }

    #[test]
    fn tap_waits_for_double_tap_window() {
        let mut recognizer = recognizer(None, true);
        let outputs = feed(&mut recognizer, &[(0, Input::Pressed), (50, Input::Released), (300, Input::Tick)]);
        assert_eq!(outputs, [Output::Gesture(Gesture::Tap)]);
    }

    #[test]
    fn early_tick_does_not_end_double_tap_window() {
        let mut recognizer = recognizer(None, true);
        let outputs = feed(&mut recognizer, &[(0, Input::Pressed), (50, Input::Released), (100, Input::Tick)]);
        assert!(outputs.is_empty());
    }

    #[test]
    fn double_tap() {
        let outputs = feed(
            &mut recognizer(None, true),
            &[(0, Input::Pressed), (50, Input::Released), (100, Input::Pressed), (150, Input::Released)],
        );
        assert_eq!(outputs, [Output::Gesture(Gesture::DoubleTap)]);
    }

    #[test]
    fn swipe_uses_total_travel() {
        let outputs = feed(
            &mut recognizer(None, false),
            &[(0, Input::Pressed), (10, moved(30.0, 5.0)), (20, moved(30.0, 0.0)), (30, Input::Released)],
        );
        assert_eq!(outputs, [Output::Gesture(Gesture::Swipe(Direction::Right))]);
    }

    #[test]
    fn swipe_is_cancelled_by_coming_back() {
        let outputs = feed(
            &mut recognizer(None, false),
            &[(0, Input::Pressed), (10, moved(0.0, -60.0)), (20, moved(0.0, 55.0)), (30, Input::Released)],
        );
        assert!(outputs.is_empty());
    }

    #[test]
    fn hold_fires_once_then_waits_for_release() {
        let outputs = feed(
            &mut recognizer(Some(HoldOutcome::Gesture), false),
            &[(0, Input::Pressed), (300, Input::Tick), (400, moved(80.0, 0.0)), (500, Input::Released)],
        );
        assert_eq!(outputs, [Output::Gesture(Gesture::Hold)]);
    }

    #[test]
    fn hold_opens_wheel_and_release_selects_hovered_slot() {
        let outputs = feed(
            &mut recognizer(Some(HoldOutcome::Wheel { slots: FOUR_SLOTS }), false),
            &[(0, Input::Pressed), (300, Input::Tick), (320, moved(30.0, 0.0)), (340, Input::Released)],
        );
        assert_eq!(outputs, [Output::WheelOpened, Output::WheelHovered(Some(1)), Output::WheelSelected(1)]);
    }

    #[test]
    fn wheel_pointer_is_capped_so_reversing_is_quick() {
        let outputs = feed(
            &mut recognizer(Some(HoldOutcome::Wheel { slots: FOUR_SLOTS }), false),
            &[(0, Input::Pressed), (300, Input::Tick), (310, moved(500.0, 0.0)), (320, moved(-80.0, 0.0))],
        );
        assert_eq!(outputs, [Output::WheelOpened, Output::WheelHovered(Some(1)), Output::WheelHovered(Some(3))]);
    }

    #[test]
    fn wheel_release_in_dead_zone_cancels() {
        let outputs = feed(
            &mut recognizer(Some(HoldOutcome::Wheel { slots: FOUR_SLOTS }), false),
            &[(0, Input::Pressed), (300, Input::Tick), (320, moved(5.0, 0.0)), (340, Input::Released)],
        );
        assert_eq!(outputs, [Output::WheelOpened, Output::WheelCancelled]);
    }
}
