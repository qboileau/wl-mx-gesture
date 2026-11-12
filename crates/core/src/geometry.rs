use std::f32::consts::TAU;
use std::num::NonZeroUsize;
use std::ops::{Add, Mul};

/// Screen orientation: y grows downwards, like the mouse's raw deltas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector {
    pub x: f32,
    pub y: f32,
}

impl Vector {
    pub const ZERO: Self = Self::new(0.0, 0.0);

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn length(self) -> f32 {
        self.x.hypot(self.y)
    }

    pub fn clamped(self, max_length: f32) -> Self {
        let length = self.length();
        if length <= max_length { self } else { self * (max_length / length) }
    }
}

impl Add for Vector {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y)
    }
}

impl Mul<f32> for Vector {
    type Output = Self;

    fn mul(self, factor: f32) -> Self {
        Self::new(self.x * factor, self.y * factor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    pub fn of(vector: Vector) -> Self {
        match (vector.x.abs() > vector.y.abs(), vector.x > 0.0, vector.y > 0.0) {
            (true, true, _) => Self::Right,
            (true, false, _) => Self::Left,
            (false, _, true) => Self::Down,
            (false, _, false) => Self::Up,
        }
    }
}

/// Slot 0 is centred on "up" and indices grow clockwise.
pub fn slot_at(pointer: Vector, slots: NonZeroUsize, dead_zone: f32) -> Option<usize> {
    if pointer.length() < dead_zone {
        return None;
    }
    let sector = TAU / slots.get() as f32;
    let angle = pointer.x.atan2(-pointer.y).rem_euclid(TAU);
    Some(((angle + sector / 2.0) / sector) as usize % slots.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOUR: NonZeroUsize = NonZeroUsize::new(4).unwrap();
    const SIX: NonZeroUsize = NonZeroUsize::new(6).unwrap();
    const DEAD_ZONE: f32 = 10.0;

    #[test]
    fn slots_go_clockwise_from_up() {
        let at = |x, y| slot_at(Vector::new(x, y), FOUR, DEAD_ZONE);
        assert_eq!(
            [at(0.0, -20.0), at(20.0, 0.0), at(0.0, 20.0), at(-20.0, 0.0)],
            [Some(0), Some(1), Some(2), Some(3)]
        );
    }

    #[test]
    fn slot_zero_wraps_around_up() {
        assert_eq!(slot_at(Vector::new(-5.0, -20.0), SIX, DEAD_ZONE), Some(0));
        assert_eq!(slot_at(Vector::new(5.0, -20.0), SIX, DEAD_ZONE), Some(0));
    }

    #[test]
    fn dead_zone_selects_nothing() {
        assert_eq!(slot_at(Vector::new(3.0, 4.0), FOUR, DEAD_ZONE), None);
    }

    #[test]
    fn direction_follows_dominant_axis() {
        assert_eq!(Direction::of(Vector::new(10.0, -3.0)), Direction::Right);
        assert_eq!(Direction::of(Vector::new(-10.0, 3.0)), Direction::Left);
        assert_eq!(Direction::of(Vector::new(2.0, 10.0)), Direction::Down);
        assert_eq!(Direction::of(Vector::new(2.0, -10.0)), Direction::Up);
    }

    #[test]
    fn clamped_keeps_direction() {
        assert_eq!(Vector::new(30.0, 40.0).clamped(5.0), Vector::new(3.0, 4.0));
        assert_eq!(Vector::new(3.0, 4.0).clamped(10.0), Vector::new(3.0, 4.0));
    }
}
