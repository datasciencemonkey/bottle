//! FLAC writer (red skeleton).
use std::io::{self, Seek, Write};
pub struct FlacWriter<W: Write + Seek> { _out: W }
impl<W: Write + Seek> FlacWriter<W> {
    pub fn new(out: W) -> io::Result<Self> { Ok(Self { _out: out }) }
    pub fn write(&mut self, _samples: &[f32]) -> io::Result<()> { unimplemented!("red") }
    pub fn samples(&self) -> u64 { 0 }
    pub fn finish(self) -> io::Result<W> { unimplemented!("red") }
}
