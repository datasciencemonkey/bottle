//! Parts (red skeleton).
use std::io;
use std::path::{Path, PathBuf};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartFile { pub index: u32, pub file_name: String, pub path: PathBuf, pub samples: u64, pub offset_ms: u64 }
impl PartFile { pub fn duration_ms(&self) -> u64 { unimplemented!("red") } }
pub fn part_file_name(_name: &str, _index: u32) -> String { unimplemented!("red") }
pub fn create_private_dir(_path: &Path) -> io::Result<()> { unimplemented!("red") }
pub struct PartWriter;
impl PartWriter {
    pub fn new(_dir: &Path, _name: &str, _max: u64) -> io::Result<Self> { unimplemented!("red") }
    pub fn write(&mut self, _s: &[f32]) -> io::Result<()> { unimplemented!("red") }
    pub fn finish(self) -> io::Result<Vec<PartFile>> { unimplemented!("red") }
}
