//! The local store key and where it lives (MEM-08b). Red skeleton for #35.

use crate::error::{CoreError, CoreResult};

/// Length of the raw SQLCipher key: 256 bits.
pub const KEY_LEN: usize = 32;

/// The raw 256-bit key of the local store.
pub struct LocalKey([u8; KEY_LEN]);

impl LocalKey {
    /// Make a new random key.
    pub fn generate() -> CoreResult<Self> {
        Err(CoreError::NotImplemented { component: "R10" })
    }
    /// Wrap key bytes.
    pub fn from_bytes(b: [u8; KEY_LEN]) -> Self {
        LocalKey(b)
    }
    /// The key bytes.
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl std::fmt::Debug for LocalKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

/// Where the key is kept.
pub trait KeyVault: Send + Sync {
    /// Load the key, or `None` if there is none.
    fn load(&self) -> CoreResult<Option<LocalKey>>;
    /// Store the key.
    fn store(&self, key: &LocalKey) -> CoreResult<()>;
}

/// In-memory test double.
#[derive(Default)]
pub struct MemoryKeyVault;

impl MemoryKeyVault {
    /// An empty vault.
    pub fn new() -> Self {
        MemoryKeyVault
    }
    /// Lose the key.
    pub fn forget(&self) {}
}

impl KeyVault for MemoryKeyVault {
    fn load(&self) -> CoreResult<Option<LocalKey>> {
        Ok(None)
    }
    fn store(&self, _key: &LocalKey) -> CoreResult<()> {
        Err(CoreError::NotImplemented { component: "R10" })
    }
}

/// macOS Keychain. Red skeleton.
#[cfg(target_os = "macos")]
pub mod macos {
    /// Keychain item accessibility.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Accessibility {
        /// `kSecAttrAccessibleWhenUnlocked`.
        WhenUnlocked,
        /// `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`.
        AfterFirstUnlockThisDeviceOnly,
    }
    /// The item description.
    #[derive(Debug, Clone)]
    pub struct KeychainSpec {
        /// Accessibility class.
        pub accessibility: Accessibility,
        /// Use the data protection keychain.
        pub data_protection_keychain: bool,
        /// Sync to iCloud.
        pub synchronizable: bool,
    }
    /// The Keychain vault.
    pub struct KeychainVault;
    impl KeychainVault {
        /// The Bottle item.
        pub fn bottle() -> Self {
            KeychainVault
        }
        /// The item description.
        pub fn spec(&self) -> KeychainSpec {
            KeychainSpec {
                accessibility: Accessibility::WhenUnlocked,
                data_protection_keychain: false,
                synchronizable: true,
            }
        }
    }
}
