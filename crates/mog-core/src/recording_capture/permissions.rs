//! Permissions (red skeleton).
pub trait PermissionAsker { fn ask_mic(&self); fn ask_system_audio(&self); }
pub fn ask_on_enable(_was: bool, _now: bool, _a: &dyn PermissionAsker) { unimplemented!("red") }
