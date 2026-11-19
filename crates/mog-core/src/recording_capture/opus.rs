//! Opus in Ogg (red skeleton).
use std::io::{self, Seek, Write};
pub struct OpusWriter<W: Write + Seek> {
    _o: W,
}
impl<W: Write + Seek> OpusWriter<W> {
    pub fn new(out: W) -> io::Result<Self> {
        Ok(Self { _o: out })
    }
    pub fn write(&mut self, _s: &[f32]) -> io::Result<()> {
        unimplemented!("red")
    }
    pub fn samples(&self) -> u64 {
        0
    }
    pub fn finish(self) -> io::Result<W> {
        unimplemented!("red")
    }
}
