//! The local store key and where it lives (MEM-08b).
//!
//! The key is 256 random bits. It is kept only in the OS key store: the macOS Keychain with
//! `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, or the Windows Credential Manager (DPAPI).
//! A macOS build with no keychain access group entitlement uses the login keychain instead
//! (#541, ADR 0071 amendment 1).
//! It is never logged, never sent over IPC and never written to a file. Tests use
//! `MemoryKeyVault` (tests and the `test-support` feature only); they never touch the real key store.

#[cfg(any(test, feature = "test-support"))]
use std::sync::Mutex;

use crate::error::{CoreError, CoreResult};

const ID: &str = "R10";

/// Length of the raw SQLCipher key: 256 bits.
pub const KEY_LEN: usize = 32;

/// The Keychain service and Credential Manager target name of the local store key.
pub const KEY_SERVICE: &str = "com.databricks.bottle.local-store";
/// The Keychain account of the local store key.
pub const KEY_ACCOUNT: &str = "memory.db";

/// The raw 256-bit key of the local store. The bytes are overwritten with zeros on drop, and
/// `Debug` does not show them.
pub struct LocalKey([u8; KEY_LEN]);

impl LocalKey {
    /// Make a new key from the OS random source.
    pub fn generate() -> CoreResult<Self> {
        let mut b = [0u8; KEY_LEN];
        getrandom::fill(&mut b).map_err(|_| CoreError::StoreUnavailable { component: ID })?;
        Ok(LocalKey(b))
    }

    /// Wrap key bytes (for a key store implementation).
    pub fn from_bytes(b: [u8; KEY_LEN]) -> Self {
        LocalKey(b)
    }

    /// The key bytes (for a key store implementation).
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// The key as 64 lower-case hex digits, for `PRAGMA key = "x'...'"`.
    pub(crate) fn to_hex(&self) -> Zeroed {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(KEY_LEN * 2);
        for b in self.0 {
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 0xf) as usize] as char);
        }
        Zeroed(s)
    }
}

impl Clone for LocalKey {
    fn clone(&self) -> Self {
        LocalKey(self.0)
    }
}

impl Drop for LocalKey {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

impl std::fmt::Debug for LocalKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LocalKey(<redacted>)")
    }
}

/// A string that holds key material. Its bytes are overwritten with zeros on drop.
pub(crate) struct Zeroed(pub(crate) String);

impl Drop for Zeroed {
    fn drop(&mut self) {
        // SAFETY: zero bytes are valid UTF-8.
        wipe(unsafe { self.0.as_bytes_mut() });
    }
}

/// Overwrite with zeros in a way the optimizer keeps.
pub(crate) fn wipe(b: &mut [u8]) {
    for x in b.iter_mut() {
        // SAFETY: `x` is a valid, aligned `&mut u8`.
        unsafe { std::ptr::write_volatile(x, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

/// Where the local store key is kept.
pub trait KeyVault: Send + Sync {
    /// Load the key, or `Ok(None)` if the key store has no key.
    fn load(&self) -> CoreResult<Option<LocalKey>>;
    /// Store the key, replacing any old one.
    fn store(&self, key: &LocalKey) -> CoreResult<()>;
}

/// The in-memory test double. It holds the key in process memory only, so it exists only in
/// tests and with the `test-support` feature: a release build can reach only the OS key store.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct MemoryKeyVault {
    slot: Mutex<Option<LocalKey>>,
}

#[cfg(any(test, feature = "test-support"))]
impl MemoryKeyVault {
    /// An empty vault.
    pub fn new() -> Self {
        Self::default()
    }

    /// Lose the key, as when the keychain loses it.
    pub fn forget(&self) {
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[cfg(any(test, feature = "test-support"))]
impl KeyVault for MemoryKeyVault {
    fn load(&self) -> CoreResult<Option<LocalKey>> {
        Ok(self.slot.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }

    fn store(&self, key: &LocalKey) -> CoreResult<()> {
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(key.clone());
        Ok(())
    }
}

/// `errSecMissingEntitlement`: the app has no keychain access group entitlement (an unsigned or
/// ad-hoc macOS build), so the data protection keychain refuses it.
#[cfg(any(target_os = "macos", test))]
pub const ERR_SEC_MISSING_ENTITLEMENT: i32 = -34018;

/// The macOS keychain that one call goes to.
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeychainTarget {
    /// The data protection keychain (`kSecUseDataProtectionKeychain`). Signed builds use it.
    DataProtection,
    /// The user's login (file-based) keychain. Only for a build with no entitlement.
    Login,
}

/// Run `op` on the data protection keychain. Only when it fails with
/// [`ERR_SEC_MISSING_ENTITLEMENT`], run it once more on the login keychain (#541). Every other
/// result, and the result of the login try, is returned as is. `op` gets and returns only
/// `OSStatus` codes, so no key bytes go through an error.
#[cfg(any(target_os = "macos", test))]
pub fn with_login_fallback<T>(
    mut op: impl FnMut(KeychainTarget) -> Result<T, i32>,
) -> Result<T, i32> {
    match op(KeychainTarget::DataProtection) {
        Err(ERR_SEC_MISSING_ENTITLEMENT) => op(KeychainTarget::Login),
        other => other,
    }
}

/// The key store of this OS: the macOS Keychain or the Windows Credential Manager.
/// On other systems it returns a vault that always fails with `keychain_unavailable`.
pub fn os_key_vault() -> Box<dyn KeyVault> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::KeychainVault::bottle())
    }
    #[cfg(windows)]
    {
        Box::new(windows::CredentialVault::bottle())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Box::new(NoKeyVault)
    }
}

/// The vault for a system with no supported key store.
#[cfg(not(any(target_os = "macos", windows)))]
struct NoKeyVault;

#[cfg(not(any(target_os = "macos", windows)))]
impl KeyVault for NoKeyVault {
    fn load(&self) -> CoreResult<Option<LocalKey>> {
        Err(CoreError::KeychainUnavailable { component: ID })
    }
    fn store(&self, _key: &LocalKey) -> CoreResult<()> {
        Err(CoreError::KeychainUnavailable { component: ID })
    }
}

/// The macOS Keychain vault.
///
/// The item is a generic password in the data protection keychain, with
/// `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` and no iCloud sync. macOS honours the
/// accessibility class only in the data protection keychain, and that keychain needs a signed app
/// with a keychain access group entitlement. An unsigned or ad-hoc build gets
/// `errSecMissingEntitlement` (-34018). Only on that code the vault retries the same call in the
/// login keychain, with the same service and account and no iCloud sync ([`with_login_fallback`],
/// #541). The login keychain does not honour the accessibility class, so MEM-08b accessibility is
/// not enforced on such a build (ADR 0071 amendment 1). Every other error is `keychain_unavailable`.
/// TODO(#34): verify on a signed build (needs:mac).
#[cfg(target_os = "macos")]
pub mod macos {
    use security_framework::access_control::{ProtectionMode, SecAccessControl};
    use security_framework::passwords::{
        generic_password, set_generic_password_options, PasswordOptions,
    };

    use super::{
        with_login_fallback, KeyVault, KeychainTarget, LocalKey, ID, KEY_ACCOUNT, KEY_LEN,
        KEY_SERVICE,
    };
    use crate::error::{CoreError, CoreResult};

    /// `errSecItemNotFound`.
    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    /// Keychain item accessibility class.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Accessibility {
        /// `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` (MEM-08b).
        AfterFirstUnlockThisDeviceOnly,
    }

    /// The description of the Keychain item. [`KeychainVault::spec`] builds it without calling
    /// the Keychain, so a test can check it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct KeychainSpec {
        /// `kSecAttrService`.
        pub service: &'static str,
        /// `kSecAttrAccount`.
        pub account: &'static str,
        /// `kSecAttrAccessible` (through `kSecAttrAccessControl`).
        pub accessibility: Accessibility,
        /// `kSecUseDataProtectionKeychain`.
        pub data_protection_keychain: bool,
        /// `kSecAttrSynchronizable`.
        pub synchronizable: bool,
    }

    /// The macOS Keychain vault for the local store key.
    pub struct KeychainVault {
        spec: KeychainSpec,
    }

    impl KeychainVault {
        /// The Bottle local store key item.
        pub fn bottle() -> Self {
            KeychainVault {
                spec: KeychainSpec {
                    service: KEY_SERVICE,
                    account: KEY_ACCOUNT,
                    accessibility: Accessibility::AfterFirstUnlockThisDeviceOnly,
                    data_protection_keychain: true,
                    synchronizable: false,
                },
            }
        }

        /// The item description.
        pub fn spec(&self) -> KeychainSpec {
            self.spec.clone()
        }

        fn options(&self, target: KeychainTarget) -> PasswordOptions {
            let mut o = PasswordOptions::new_generic_password(self.spec.service, self.spec.account);
            if self.spec.data_protection_keychain && target == KeychainTarget::DataProtection {
                o.use_protected_keychain();
            }
            o.set_access_synchronized(Some(self.spec.synchronizable));
            o
        }
    }

    impl KeyVault for KeychainVault {
        fn load(&self) -> CoreResult<Option<LocalKey>> {
            match with_login_fallback(|t| generic_password(self.options(t)).map_err(|e| e.code())) {
                Ok(mut v) => {
                    let out = if v.len() == KEY_LEN {
                        let mut b = [0u8; KEY_LEN];
                        b.copy_from_slice(&v);
                        Ok(Some(LocalKey::from_bytes(b)))
                    } else {
                        Err(CoreError::KeychainUnavailable { component: ID })
                    };
                    super::wipe(&mut v);
                    out
                }
                Err(ERR_SEC_ITEM_NOT_FOUND) => Ok(None),
                Err(_) => Err(CoreError::KeychainUnavailable { component: ID }),
            }
        }

        fn store(&self, key: &LocalKey) -> CoreResult<()> {
            let Accessibility::AfterFirstUnlockThisDeviceOnly = self.spec.accessibility;
            with_login_fallback(|t| {
                let mut o = self.options(t);
                // The login keychain does not honour the accessibility class (ADR 0071
                // amendment 1), so only the data protection item gets the access control.
                if t == KeychainTarget::DataProtection {
                    let ac = SecAccessControl::create_with_protection(
                        Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly),
                        0,
                    )
                    .map_err(|e| e.code())?;
                    o.set_access_control(ac);
                }
                set_generic_password_options(key.as_bytes(), o).map_err(|e| e.code())
            })
            .map_err(|_| CoreError::KeychainUnavailable { component: ID })
        }
    }
}

/// The Windows Credential Manager vault.
///
/// The key is a generic credential with `CRED_PERSIST_LOCAL_MACHINE` (it does not roam). Windows
/// protects the credential blob with DPAPI for the user. TODO(#34): verify on a Windows device
/// (needs:windows).
#[cfg(windows)]
pub mod windows {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
    use windows_sys::Win32::Security::Credentials::{
        CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };

    use super::{KeyVault, LocalKey, ID, KEY_LEN, KEY_SERVICE};
    use crate::error::{CoreError, CoreResult};

    /// The description of the credential. [`CredentialVault::spec`] builds it without calling
    /// the Credential Manager, so a test can check it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CredentialSpec {
        /// The target name.
        pub target: &'static str,
        /// `CRED_TYPE_GENERIC`.
        pub generic: bool,
        /// `CRED_PERSIST_LOCAL_MACHINE`: this device only, no roaming.
        pub persist_local_machine: bool,
    }

    /// The Windows Credential Manager vault for the local store key.
    pub struct CredentialVault {
        spec: CredentialSpec,
    }

    impl CredentialVault {
        /// The Bottle local store key credential.
        pub fn bottle() -> Self {
            CredentialVault {
                spec: CredentialSpec {
                    target: KEY_SERVICE,
                    generic: true,
                    persist_local_machine: true,
                },
            }
        }

        /// The credential description.
        pub fn spec(&self) -> CredentialSpec {
            self.spec.clone()
        }

        fn target_w(&self) -> Vec<u16> {
            self.spec
                .target
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect()
        }
    }

    impl KeyVault for CredentialVault {
        fn load(&self) -> CoreResult<Option<LocalKey>> {
            let target = self.target_w();
            let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
            // SAFETY: `target` is NUL-terminated; `cred` receives a buffer that we free below.
            let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) };
            if ok == 0 {
                // SAFETY: plain FFI call.
                let err = unsafe { GetLastError() };
                return if err == ERROR_NOT_FOUND {
                    Ok(None)
                } else {
                    Err(CoreError::KeychainUnavailable { component: ID })
                };
            }
            // SAFETY: CredReadW succeeded, so `cred` points to a valid CREDENTIALW.
            let out = unsafe {
                let c = &*cred;
                if c.CredentialBlobSize as usize == KEY_LEN && !c.CredentialBlob.is_null() {
                    let blob = std::slice::from_raw_parts_mut(c.CredentialBlob, KEY_LEN);
                    let mut b = [0u8; KEY_LEN];
                    b.copy_from_slice(blob);
                    super::wipe(blob);
                    Ok(Some(LocalKey::from_bytes(b)))
                } else {
                    Err(CoreError::KeychainUnavailable { component: ID })
                }
            };
            // SAFETY: `cred` came from CredReadW.
            unsafe { CredFree(cred as *const _) };
            out
        }

        fn store(&self, key: &LocalKey) -> CoreResult<()> {
            let mut target = self.target_w();
            let mut blob = *key.as_bytes();
            // SAFETY: an all-zero CREDENTIALW is valid; we set the fields we use.
            let mut c: CREDENTIALW = unsafe { std::mem::zeroed() };
            c.Type = CRED_TYPE_GENERIC;
            c.TargetName = target.as_mut_ptr();
            c.CredentialBlobSize = KEY_LEN as u32;
            c.CredentialBlob = blob.as_mut_ptr();
            c.Persist = CRED_PERSIST_LOCAL_MACHINE;
            // SAFETY: every pointer in `c` is valid for the call.
            let ok = unsafe { CredWriteW(&c, 0) };
            super::wipe(&mut blob);
            if ok == 0 {
                Err(CoreError::KeychainUnavailable { component: ID })
            } else {
                Ok(())
            }
        }
    }
}
