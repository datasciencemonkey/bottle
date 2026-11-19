//! Mixer (red skeleton).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Track { Mic, System }
pub fn soft_clip(_x: f32) -> f32 { unimplemented!("red") }
pub struct Mixer;
impl Mixer {
    pub fn new(_max_lag: usize) -> Self { Self }
    pub fn push(&mut self, _t: Track, _s: &[f32], _rate: u32) { unimplemented!("red") }
    pub fn drain(&mut self, _out: &mut Vec<f32>) { unimplemented!("red") }
    pub fn flush(&mut self, _out: &mut Vec<f32>) { unimplemented!("red") }
    pub fn buffered(&self, _t: Track) -> usize { unimplemented!("red") }
}
