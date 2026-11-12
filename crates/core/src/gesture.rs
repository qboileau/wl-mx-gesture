use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use crate::geometry::{Direction, Vector, slot_at};

/// Caps the wheel pointer so reversing direction never needs a long trip back.
const WHEEL_TRAVEL_IN_DEAD_ZONES: f32 = 2.0;
/// Pressing the thumb button jolts the mouse by more than a swipe threshold within ~15 ms (seen with
/// `--test` on an MX Master 3S); movement this soon after a press is ignored.
const PRESS_SETTLE: Duration = Duration::from_millis(50);

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

/// The gestures that can open a wheel instead of firing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    DoubleTap,
    Hold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Gesture(Gesture),
    WheelOpened(Trigger),
    WheelHovered(Option<usize>),
    WheelSelected(Trigger, usize),
    WheelCancelled,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    Gesture,
    Wheel { slots: NonZeroUsize },
}

/// A bound trigger: how long it takes (hold) or may take (double tap), and what it does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timed {
    pub duration: Duration,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub swipe_threshold: f32,
    pub dead_zone: f32,
    /// `None` when nothing is bound to hold: a long still press is then a tap.
    pub hold: Option<Timed>,
    /// `None` when nothing is bound to double tap, so taps fire without waiting.
    pub double_tap: Option<Timed>,
}

#[derive(Debug, Clone, Copy)]
enum State {
    Idle,
    Pressed { since: Instant, travel: Vector },
    Swiping { travel: Vector },
    AwaitingSecondTap { until: Instant, outcome: Outcome },
    SecondPress,
    Holding,
    Wheel { trigger: Trigger, slots: NonZeroUsize, pointer: Vector, hovered: Option<usize> },
}

pub struct Recognizer {
    settings: Settings,
    state: State,
    last_press: Option<Instant>,
}

impl Recognizer {
    pub fn new(settings: Settings) -> Self {
        Self { settings, state: State::Idle, last_press: None }
    }

    /// When to feed [`Input::Tick`] if no other input arrives first.
    pub fn deadline(&self) -> Option<Instant> {
        match self.state {
            State::Pressed { since, .. } => self.settings.hold.map(|hold| since + hold.duration),
            State::AwaitingSecondTap { until, .. } => Some(until),
            _ => None,
        }
    }

    pub fn handle(&mut self, input: Input, now: Instant) -> Option<Output> {
        match input {
            Input::Moved(_) if self.last_press.is_some_and(|pressed| now < pressed + PRESS_SETTLE) => return None,
            Input::Pressed => self.last_press = Some(now),
            _ => {}
        }
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
                Some(hold) if now >= since + hold.duration => match hold.outcome {
                    Outcome::Gesture => (State::Holding, Some(Output::Gesture(Gesture::Hold))),
                    Outcome::Wheel { slots } => open_wheel(Trigger::Hold, slots),
                },
                _ => (self.state, None),
            },
            (State::Pressed { .. }, Input::Released) => match settings.double_tap {
                Some(double_tap) => {
                    (State::AwaitingSecondTap { until: now + double_tap.duration, outcome: double_tap.outcome }, None)
                }
                None => (State::Idle, Some(Output::Gesture(Gesture::Tap))),
            },
            (State::Swiping { travel }, Input::Moved(delta)) => (State::Swiping { travel: travel + delta }, None),
            (State::Swiping { travel }, Input::Released) => (
                State::Idle,
                (travel.length() >= settings.swipe_threshold)
                    .then(|| Output::Gesture(Gesture::Swipe(Direction::of(travel)))),
            ),
            (State::AwaitingSecondTap { outcome, .. }, Input::Pressed) => match outcome {
                Outcome::Gesture => (State::SecondPress, None),
                Outcome::Wheel { slots } => open_wheel(Trigger::DoubleTap, slots),
            },
            (State::AwaitingSecondTap { until, .. }, Input::Tick) if now >= until => {
                (State::Idle, Some(Output::Gesture(Gesture::Tap)))
            }
            (State::SecondPress, Input::Released) => (State::Idle, Some(Output::Gesture(Gesture::DoubleTap))),
            (State::Holding, Input::Released) => (State::Idle, None),
            (State::Wheel { trigger, slots, pointer, hovered }, Input::Moved(delta)) => {
                let pointer = (pointer + delta).clamped(settings.dead_zone * WHEEL_TRAVEL_IN_DEAD_ZONES);
                let now_hovered = slot_at(pointer, slots, settings.dead_zone);
                (
                    State::Wheel { trigger, slots, pointer, hovered: now_hovered },
                    (now_hovered != hovered).then_some(Output::WheelHovered(now_hovered)),
                )
            }
            (State::Wheel { trigger, hovered, .. }, Input::Released) => {
                (State::Idle, Some(hovered.map_or(Output::WheelCancelled, |slot| Output::WheelSelected(trigger, slot))))
            }
            (state, _) => (state, None),
        }
    }
}

fn open_wheel(trigger: Trigger, slots: NonZeroUsize) -> (State, Option<Output>) {
    (State::Wheel { trigger, slots, pointer: Vector::ZERO, hovered: None }, Some(Output::WheelOpened(trigger)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOLD: Duration = Duration::from_millis(300);
    const DOUBLE_TAP: Duration = Duration::from_millis(250);
    const FOUR_SLOTS: NonZeroUsize = NonZeroUsize::new(4).unwrap();

    const WHEEL: Outcome = Outcome::Wheel { slots: FOUR_SLOTS };

    fn recognizer(hold: Option<Outcome>, double_tap: Option<Outcome>) -> Recognizer {
        Recognizer::new(Settings {
            swipe_threshold: 50.0,
            dead_zone: 20.0,
            hold: hold.map(|outcome| Timed { duration: HOLD, outcome }),
            double_tap: double_tap.map(|outcome| Timed { duration: DOUBLE_TAP, outcome }),
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
        let outputs = feed(&mut recognizer(None, None), &[(0, Input::Pressed), (50, Input::Released)]);
        assert_eq!(outputs, [Output::Gesture(Gesture::Tap)]);
    }

    #[test]
    fn tap_waits_for_double_tap_window() {
        let mut recognizer = recognizer(None, Some(Outcome::Gesture));
        let outputs = feed(&mut recognizer, &[(0, Input::Pressed), (50, Input::Released), (300, Input::Tick)]);
        assert_eq!(outputs, [Output::Gesture(Gesture::Tap)]);
    }

    #[test]
    fn early_tick_does_not_end_double_tap_window() {
        let mut recognizer = recognizer(None, Some(Outcome::Gesture));
        let outputs = feed(&mut recognizer, &[(0, Input::Pressed), (50, Input::Released), (100, Input::Tick)]);
        assert!(outputs.is_empty());
    }

    #[test]
    fn double_tap() {
        let outputs = feed(
            &mut recognizer(None, Some(Outcome::Gesture)),
            &[(0, Input::Pressed), (50, Input::Released), (100, Input::Pressed), (150, Input::Released)],
        );
        assert_eq!(outputs, [Output::Gesture(Gesture::DoubleTap)]);
    }

    #[test]
    fn swipe_uses_total_travel() {
        let outputs = feed(
            &mut recognizer(None, None),
            &[(0, Input::Pressed), (60, moved(30.0, 5.0)), (70, moved(30.0, 0.0)), (80, Input::Released)],
        );
        assert_eq!(outputs, [Output::Gesture(Gesture::Swipe(Direction::Right))]);
    }

    #[test]
    fn swipe_is_cancelled_by_coming_back() {
        let outputs = feed(
            &mut recognizer(None, None),
            &[(0, Input::Pressed), (60, moved(0.0, -60.0)), (70, moved(0.0, 55.0)), (80, Input::Released)],
        );
        assert!(outputs.is_empty());
    }

    #[test]
    fn hold_fires_once_then_waits_for_release() {
        let outputs = feed(
            &mut recognizer(Some(Outcome::Gesture), None),
            &[(0, Input::Pressed), (300, Input::Tick), (400, moved(80.0, 0.0)), (500, Input::Released)],
        );
        assert_eq!(outputs, [Output::Gesture(Gesture::Hold)]);
    }

    #[test]
    fn hold_opens_wheel_and_release_selects_hovered_slot() {
        let outputs = feed(
            &mut recognizer(Some(WHEEL), None),
            &[(0, Input::Pressed), (300, Input::Tick), (320, moved(30.0, 0.0)), (340, Input::Released)],
        );
        assert_eq!(
            outputs,
            [
                Output::WheelOpened(Trigger::Hold),
                Output::WheelHovered(Some(1)),
                Output::WheelSelected(Trigger::Hold, 1)
            ]
        );
    }

    #[test]
    fn wheel_pointer_is_capped_so_reversing_is_quick() {
        let outputs = feed(
            &mut recognizer(Some(WHEEL), None),
            &[(0, Input::Pressed), (300, Input::Tick), (310, moved(500.0, 0.0)), (320, moved(-80.0, 0.0))],
        );
        assert_eq!(
            outputs,
            [Output::WheelOpened(Trigger::Hold), Output::WheelHovered(Some(1)), Output::WheelHovered(Some(3))]
        );
    }

    #[test]
    fn wheel_release_in_dead_zone_cancels() {
        let outputs = feed(
            &mut recognizer(Some(WHEEL), None),
            &[(0, Input::Pressed), (300, Input::Tick), (320, moved(5.0, 0.0)), (340, Input::Released)],
        );
        assert_eq!(outputs, [Output::WheelOpened(Trigger::Hold), Output::WheelCancelled]);
    }

    #[test]
    fn double_tap_opens_wheel_on_second_press_ignoring_movement_in_between() {
        let outputs = feed(
            &mut recognizer(None, Some(WHEEL)),
            &[
                (0, Input::Pressed),
                (80, Input::Released),
                (120, moved(200.0, 0.0)),
                (160, Input::Pressed),
                (260, moved(0.0, 30.0)),
                (300, Input::Released),
            ],
        );
        assert_eq!(
            outputs,
            [
                Output::WheelOpened(Trigger::DoubleTap),
                Output::WheelHovered(Some(2)),
                Output::WheelSelected(Trigger::DoubleTap, 2)
            ]
        );
    }

    #[test]
    fn jolt_right_after_press_is_not_a_swipe() {
        let outputs =
            feed(&mut recognizer(None, None), &[(0, Input::Pressed), (6, moved(80.0, 0.0)), (100, Input::Released)]);
        assert_eq!(outputs, [Output::Gesture(Gesture::Tap)]);
    }

    #[test]
    fn jolt_right_after_second_press_does_not_hover() {
        let outputs = feed(
            &mut recognizer(None, Some(WHEEL)),
            &[(0, Input::Pressed), (80, Input::Released), (160, Input::Pressed), (170, moved(60.0, 0.0))],
        );
        assert_eq!(outputs, [Output::WheelOpened(Trigger::DoubleTap)]);
    }
}
