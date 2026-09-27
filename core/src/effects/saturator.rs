use crate::math;

pub struct Saturator {
    drive: f32,
    inv_tanh_drive: f32, // 1.0 / tanh(drive)
}

impl Saturator {
    pub fn new(drive: f32) -> Self {
        let d = if drive < 0.1 { 0.1 } else { drive };
        Self { drive: d, inv_tanh_drive: 1.0 / math::tanh(d) }
    }

    pub fn set_drive(&mut self, drive: f32) {
        self.drive = if drive < 0.1 { 0.1 } else { drive };
        self.inv_tanh_drive = 1.0 / math::tanh(self.drive);
    }

    /// output = tanh(input * drive) / tanh(drive)
    #[inline]
    pub fn process(&self, input: f32) -> f32 {
        math::tanh(input * self.drive) * self.inv_tanh_drive
    }
}
