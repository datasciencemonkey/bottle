//! CliRunner (component R5): the only way Bottle talks to the Databricks CLI.
//!
//! - resolves `databricks` to an absolute path (W-02a),
//! - refuses a CLI in a world-writable or temporary directory (W-02b),
//! - needs CLI version 1.17 or later (W-02c),
//! - checks the path and version again at each launch (W-02d),
//! - runs every CLI child with a clean env (W-09).
//!
//! All filesystem, environment and process access goes through the small
//! traits in [`sys`] so that tests can use fakes.

pub mod env;
pub mod error;
pub mod location;
pub mod resolve;
pub mod runner;
pub mod sys;
pub mod version;

pub use env::{clean_env, clean_env_for, is_allowed};
pub use error::{CliError, UnsafeReason};
pub use location::{check_location, path_within, temp_roots};
pub use resolve::{binary_name, fallback_dirs, resolve, CliPolicy, Resolved, CLI_NAME};
pub use runner::{CliRecord, CliRunner, CliStatus, CliWarning, LaunchReport};
pub use sys::{Env, Fs, ProcOutput, Process, RealEnv, RealFs, RealProcess};
pub use version::{Version, MIN_VERSION, RECOMMENDED_VERSION};
