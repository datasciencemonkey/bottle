//! The local store key and where it lives (MEM-08b).
//!
//! The key is 256 random bits. It is kept only in the OS key store: the macOS Keychain with
//! `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, or the Windows Credential Manager (DPAPI).
//! A macOS build with no keychain access group entitlement uses the login keychain instead
//! (#541, ADR 0071 amendment 1).
//! It is never logged, never sent over IPC and never written to a file. Tests use
//! `MemoryKeyVault` (tests and the `test-support` feature only); they never touch the real key store.

use std::sync::{Arc, Mutex};

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

/// One OS key store read per launch: the first key that loads (or is stored) stays in process
/// memory, so the store, a reopen and every later caller share it, and macOS asks at most once.
/// A failed read is not kept: a later Try again reads the OS store again. Nothing is written
/// to disk, and `Debug` shows no key.
pub struct CachedKeyVault {
    inner: Arc<dyn KeyVault>,
    key: Mutex<Option<LocalKey>>,
}

impl CachedKeyVault {
    pub fn new(inner: Arc<dyn KeyVault>) -> Self {
        Self {
            inner,
            key: Mutex::new(None),
        }
    }
}

impl KeyVault for CachedKeyVault {
    fn load(&self) -> CoreResult<Option<LocalKey>> {
        let mut g = self.key.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(k) = g.as_ref() {
            return Ok(Some(k.clone()));
        }
        let k = self.inner.load()?;
        g.clone_from(&k);
        Ok(k)
    }

    fn store(&self, key: &LocalKey) -> CoreResult<()> {
        self.inner.store(key)?;
        *self.key.lock().unwrap_or_else(|e| e.into_inner()) = Some(key.clone());
        Ok(())
    }
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

/// The macOS keychain that the vault uses. It is chosen one time, from the entitlement.
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeychainTarget {
    /// The data protection keychain (`kSecUseDataProtectionKeychain`). Signed builds use it.
    DataProtection,
    /// The user's login (file-based) keychain. Only a build with no entitlement uses it.
    Login,
}

/// The keychain for a process that has (`true`) or does not have a keychain entitlement
/// ([`entitled_by`], #541). There is no fallback between them: an entitled build never reads or writes
/// the login keychain, so it never uses a login item that another process of the user planted.
#[cfg(any(target_os = "macos", test))]
pub fn target_for(entitled: bool) -> KeychainTarget {
    if entitled {
        KeychainTarget::DataProtection
    } else {
        KeychainTarget::Login
    }
}

/// The entitlements that give a process a keychain access group in the data protection keychain
/// (Apple: keychain access groups; TN3137). A process with any of them is entitled.
#[cfg(any(target_os = "macos", test))]
pub const KEYCHAIN_ENTITLEMENTS: &[&str] = &[
    "keychain-access-groups",
    "com.apple.application-identifier",
    "com.apple.security.application-groups",
];

/// Whether a process is entitled, given `has(key)` for each entitlement key: true when any of
/// [`KEYCHAIN_ENTITLEMENTS`] is present.
#[cfg(any(target_os = "macos", test))]
pub fn entitled_by(has: impl Fn(&str) -> bool) -> bool {
    KEYCHAIN_ENTITLEMENTS.iter().any(|k| has(k))
}

/// The item attributes that the vault sends for one keychain.
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemPlan {
    /// `kSecUseDataProtectionKeychain`.
    pub data_protection_keychain: bool,
    /// `kSecAttrAccessControl` with `AfterFirstUnlockThisDeviceOnly` on a store (MEM-08b).
    pub access_control: bool,
    /// `kSecAttrSynchronizable`. Always false: the key never syncs to iCloud.
    pub synchronizable: bool,
}

/// The item attributes for `target`. The login keychain does not honour the accessibility class,
/// so its item gets no access control (ADR 0071 amendment 1).
#[cfg(any(target_os = "macos", test))]
pub fn item_plan(target: KeychainTarget) -> ItemPlan {
    let dp = target == KeychainTarget::DataProtection;
    ItemPlan {
        data_protection_keychain: dp,
        access_control: dp,
        synchronizable: false,
    }
}

/// The key store of this OS: the macOS Keychain or the Windows Credential Manager.
/// On other systems it returns a vault that always fails with `keychain_unavailable`.
/// The vault reads the OS store at most once per launch on success ([`CachedKeyVault`]).
pub fn os_key_vault() -> Box<dyn KeyVault> {
    #[cfg(target_os = "macos")]
    let inner: Arc<dyn KeyVault> = Arc::new(macos::KeychainVault::bottle());
    #[cfg(windows)]
    let inner: Arc<dyn KeyVault> = Arc::new(windows::CredentialVault::bottle());
    #[cfg(not(any(target_os = "macos", windows)))]
    let inner: Arc<dyn KeyVault> = Arc::new(NoKeyVault);
    Box::new(CachedKeyVault::new(inner))
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
/// A signed build has the `keychain-access-groups` entitlement. Its item is a generic password in
/// the data protection keychain, with `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` and no
/// iCloud sync. macOS honours the accessibility class only in that keychain. An unsigned or ad-hoc
/// build (for example `pnpm tauri dev`) has no entitlement, and the data protection keychain
/// refuses it. The vault checks the entitlements one time at creation ([`entitled_by`],
/// [`target_for`], #541).
/// Without it, the vault uses only the login keychain, with the same service and account and no
/// iCloud sync. MEM-08b accessibility is then not enforced (ADR 0071 amendment 1). There is no
/// fallback between the two keychains. TODO(#34): verify on a signed build (needs:mac) that
/// `KeychainVault::bottle().target()` is `DataProtection` and that no login item is made.
#[cfg(target_os = "macos")]
pub mod macos {
    use security_framework::access_control::{ProtectionMode, SecAccessControl};
    use security_framework::passwords::{
        generic_password, set_generic_password_options, PasswordOptions,
    };

    use super::{
        item_plan, target_for, KeyVault, KeychainTarget, LocalKey, ID, KEY_ACCOUNT, KEY_LEN,
        KEY_SERVICE,
    };
    use crate::error::{CoreError, CoreResult};

    /// `errSecItemNotFound`.
    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    /// Whether this process has any of [`KEYCHAIN_ENTITLEMENTS`](super::KEYCHAIN_ENTITLEMENTS).
    /// A signed release build has one; an unsigned or ad-hoc build has none. A failed check of a
    /// key reads as "not present".
    pub fn has_keychain_entitlement() -> bool {
        use std::ffi::c_void;
        type CFRef = *const c_void;
        #[link(name = "CoreFoundation", kind = "framework")]
        extern "C" {
            fn CFStringCreateWithBytes(
                alloc: CFRef,
                bytes: *const u8,
                len: isize,
                encoding: u32,
                external: u8,
            ) -> CFRef;
            fn CFRelease(cf: CFRef);
        }
        #[link(name = "Security", kind = "framework")]
        extern "C" {
            fn SecTaskCreateFromSelf(alloc: CFRef) -> CFRef;
            fn SecTaskCopyValueForEntitlement(
                task: CFRef,
                entitlement: CFRef,
                error: *mut CFRef,
            ) -> CFRef;
        }
        const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8
                                       // SAFETY: plain CoreFoundation and Security calls. Each object that a Create or Copy
                                       // call returns is released once, and a null is never released.
        unsafe {
            let task = SecTaskCreateFromSelf(std::ptr::null());
            if task.is_null() {
                return false;
            }
            let entitled = super::entitled_by(|key| {
                let name = CFStringCreateWithBytes(
                    std::ptr::null(),
                    key.as_ptr(),
                    key.len() as isize,
                    UTF8,
                    0,
                );
                if name.is_null() {
                    return false;
                }
                let value = SecTaskCopyValueForEntitlement(task, name, std::ptr::null_mut());
                let present = !value.is_null();
                if present {
                    CFRelease(value);
                }
                CFRelease(name);
                present
            });
            CFRelease(task);
            entitled
        }
    }

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
        target: KeychainTarget,
    }

    impl KeychainVault {
        /// The Bottle local store key item, in the keychain that this process may use.
        pub fn bottle() -> Self {
            Self::for_entitlement(has_keychain_entitlement())
        }

        /// The Bottle item for a process that has (`true`) or does not have a keychain
        /// entitlement. The seam for tests.
        pub fn for_entitlement(entitled: bool) -> Self {
            KeychainVault {
                target: target_for(entitled),
                spec: KeychainSpec {
                    service: KEY_SERVICE,
                    account: KEY_ACCOUNT,
                    accessibility: Accessibility::AfterFirstUnlockThisDeviceOnly,
                    data_protection_keychain: true,
                    synchronizable: false,
                },
            }
        }

        /// The item description of a signed build.
        pub fn spec(&self) -> KeychainSpec {
            self.spec.clone()
        }

        /// The keychain that this vault uses.
        pub fn target(&self) -> KeychainTarget {
            self.target
        }

        fn options(&self) -> PasswordOptions {
            let plan = item_plan(self.target);
            let mut o = PasswordOptions::new_generic_password(self.spec.service, self.spec.account);
            if self.spec.data_protection_keychain && plan.data_protection_keychain {
                o.use_protected_keychain();
            }
            o.set_access_synchronized(Some(self.spec.synchronizable && plan.synchronizable));
            o
        }
    }

    impl KeyVault for KeychainVault {
        fn load(&self) -> CoreResult<Option<LocalKey>> {
            match generic_password(self.options()) {
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
                Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
                Err(_) => Err(CoreError::KeychainUnavailable { component: ID }),
            }
        }

        fn store(&self, key: &LocalKey) -> CoreResult<()> {
            let mut o = self.options();
            if item_plan(self.target).access_control {
                let Accessibility::AfterFirstUnlockThisDeviceOnly = self.spec.accessibility;
                let ac = SecAccessControl::create_with_protection(
                    Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly),
                    0,
                )
                .map_err(|_| CoreError::KeychainUnavailable { component: ID })?;
                o.set_access_control(ac);
            }
            set_generic_password_options(key.as_bytes(), o)
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
