//! Piercer revolver timing (`Revolver`, gunVariation 0).

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shot {
    Normal,
    Pierce,
}

#[derive(Clone, Debug)]
pub struct Revolver {
    pub shoot_charge: f32,
    pub shoot_ready: bool,
    /// Alt-fire charge, 0..100 while holding.
    pub pierce_shot_charge: f32,
    /// Battery for the next pierce shot, refills 40/s.
    pub pierce_charge: f32,
    pub pierce_ready: bool,
}

impl Default for Revolver {
    fn default() -> Self {
        Self { shoot_charge: 100.0, shoot_ready: true, pierce_shot_charge: 0.0, pierce_charge: 100.0, pierce_ready: true }
    }
}

impl Revolver {
    pub fn update(&mut self, fire1_held: bool, fire2_held: bool, fire2_released: bool, dt: f32) -> Option<Shot> {
        if !self.shoot_ready {
            if self.shoot_charge + 200.0 * dt < 100.0 {
                self.shoot_charge += 200.0 * dt;
            } else {
                self.shoot_charge = 100.0;
                self.shoot_ready = true;
            }
        }
        if !self.pierce_ready {
            if self.pierce_charge + 40.0 * dt < 100.0 {
                self.pierce_charge += 40.0 * dt;
            } else {
                self.pierce_charge = 100.0;
                self.pierce_ready = true;
            }
        }
        if (fire2_released || fire1_held) && self.shoot_ready && self.pierce_shot_charge == 100.0 {
            self.shoot_ready = false;
            self.shoot_charge = 0.0;
            self.pierce_shot_charge = 0.0;
            self.pierce_ready = false;
            self.pierce_charge = 0.0;
            return Some(Shot::Pierce);
        }
        if fire1_held && self.shoot_ready && self.pierce_shot_charge == 0.0 {
            self.shoot_ready = false;
            self.shoot_charge = 0.0;
            return Some(Shot::Normal);
        }
        if fire2_held && self.shoot_ready && self.pierce_ready {
            self.pierce_shot_charge = (self.pierce_shot_charge + 175.0 * dt).min(100.0);
        } else {
            self.pierce_shot_charge = (self.pierce_shot_charge - 175.0 * dt).max(0.0);
        }
        None
    }
}
