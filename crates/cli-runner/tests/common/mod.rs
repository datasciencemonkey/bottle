//! Fakes for the Fs, Env and Process seams.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cli_runner::{binary_name, Env, Fs, ProcOutput, Process};

/// An absolute path on this OS: `/a/b` on unix, `C:\a\b` on Windows.
pub fn abs(rel: &str) -> PathBuf {
    let mut p = if cfg!(windows) {
        PathBuf::from("C:\\")
    } else {
        PathBuf::from("/")
    };
    for part in rel.split('/').filter(|s| !s.is_empty()) {
        p.push(part);
    }
    p
}

/// `dir/databricks` (or `databricks.exe` on Windows).
pub fn cli_in(dir: &Path) -> PathBuf {
    dir.join(binary_name(cfg!(windows)))
}

#[derive(Default, Clone)]
pub struct FakeFs {
    /// Existing files and dirs with their unix mode bits.
    pub entries: HashMap<PathBuf, (bool, u32)>,
    /// Symlinks: link -> target.
    pub links: HashMap<PathBuf, PathBuf>,
}

impl FakeFs {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn dir(mut self, p: &Path, mode: u32) -> Self {
        self.entries.insert(p.to_path_buf(), (false, mode));
        self
    }
    /// A file and its parent dir (mode 0o755 unless already set).
    pub fn file(mut self, p: &Path, mode: u32) -> Self {
        if let Some(parent) = p.parent() {
            self.entries
                .entry(parent.to_path_buf())
                .or_insert((false, 0o755));
        }
        self.entries.insert(p.to_path_buf(), (true, mode));
        self
    }
    pub fn link(mut self, link: &Path, target: &Path) -> Self {
        self.links.insert(link.to_path_buf(), target.to_path_buf());
        self
    }
    fn target(&self, p: &Path) -> PathBuf {
        self.links
            .get(p)
            .cloned()
            .unwrap_or_else(|| p.to_path_buf())
    }
}

impl Fs for FakeFs {
    fn is_file(&self, path: &Path) -> bool {
        matches!(self.entries.get(&self.target(path)), Some((true, _)))
    }
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        let t = self.target(path);
        if self.entries.contains_key(&t) {
            Ok(t)
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "fake: not found"))
        }
    }
    fn unix_mode(&self, path: &Path) -> Option<u32> {
        self.entries.get(&self.target(path)).map(|(_, m)| *m)
    }
}

#[derive(Clone)]
pub struct FakeEnv {
    pub vars: Vec<(OsString, OsString)>,
    pub temp: PathBuf,
}

impl FakeEnv {
    pub fn new() -> Self {
        Self {
            vars: Vec::new(),
            temp: abs("fake-os-temp"),
        }
    }
    pub fn with_path(self, dirs: &[PathBuf]) -> Self {
        let joined = std::env::join_paths(dirs).expect("join PATH");
        self.set("PATH", joined)
    }
    pub fn set(mut self, k: &str, v: impl Into<OsString>) -> Self {
        self.vars.push((k.into(), v.into()));
        self
    }
    pub fn temp(mut self, t: PathBuf) -> Self {
        self.temp = t;
        self
    }
}

impl Env for FakeEnv {
    fn var(&self, name: &str) -> Option<OsString> {
        self.vars
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }
    fn vars(&self) -> Vec<(OsString, OsString)> {
        self.vars.clone()
    }
    fn temp_dir(&self) -> PathBuf {
        self.temp.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
}

/// Answers `--version` with `version_output` and records every call.
#[derive(Clone)]
pub struct FakeProcess {
    pub version_output: Rc<RefCell<String>>,
    pub calls: Rc<RefCell<Vec<Call>>>,
    pub fail_spawn: bool,
}

impl FakeProcess {
    pub fn new(version_output: &str) -> Self {
        Self {
            version_output: Rc::new(RefCell::new(version_output.to_string())),
            calls: Rc::new(RefCell::new(Vec::new())),
            fail_spawn: false,
        }
    }
    pub fn set_version_output(&self, s: &str) {
        *self.version_output.borrow_mut() = s.to_string();
    }
}

impl Process for FakeProcess {
    fn run(
        &self,
        program: &Path,
        args: &[OsString],
        env: &[(OsString, OsString)],
    ) -> io::Result<ProcOutput> {
        self.calls.borrow_mut().push(Call {
            program: program.to_path_buf(),
            args: args.to_vec(),
            env: env.to_vec(),
        });
        if self.fail_spawn {
            return Err(io::Error::new(io::ErrorKind::NotFound, "fake: spawn"));
        }
        let stdout = if args.first().map(|a| a == "--version").unwrap_or(false) {
            self.version_output.borrow().clone().into_bytes()
        } else {
            b"{}".to_vec()
        };
        Ok(ProcOutput {
            code: Some(0),
            stdout,
            stderr: Vec::new(),
        })
    }
}
