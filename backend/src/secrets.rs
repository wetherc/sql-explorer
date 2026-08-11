//! The password store.
//!
//! A password belongs in the keychain of the operating system and not in
//! the settings file. This module keeps the two apart and moves a password
//! that an older version of the application wrote in clear text.

use crate::error::Result;

/// The name under which the application registers its entries.
pub const SERVICE: &str = "com.c-wetherill.sql-explorer";

/// The key of the secret access key of an Athena connection. A connection
/// holds more than one secret, so each secret takes a key of its own under
/// the identifier of the connection.
pub fn aws_secret_key(id: &str) -> String {
    format!("{id}:aws-secret-access-key")
}

/// The key of the session token of an Athena connection.
pub fn aws_token_key(id: &str) -> String {
    format!("{id}:aws-session-token")
}

/// The operations the command layer needs from a password store. The trait
/// exists so that the tests can run without a keychain.
pub trait SecretStore: Send + Sync {
    fn set(&self, id: &str, password: &str) -> Result<()>;
    fn get(&self, id: &str) -> Result<Option<String>>;
    fn delete(&self, id: &str) -> Result<()>;
}

/// The store that the operating system provides.
pub struct KeychainStore;

impl KeychainStore {
    fn entry(id: &str) -> Result<keyring::Entry> {
        Ok(keyring::Entry::new(SERVICE, id)?)
    }
}

impl SecretStore for KeychainStore {
    fn set(&self, id: &str, password: &str) -> Result<()> {
        Self::entry(id)?.set_password(password)?;
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Option<String>> {
        match Self::entry(id)?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn delete(&self, id: &str) -> Result<()> {
        match Self::entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

/// A store that holds the passwords in memory. It serves a session in
/// which the keychain is not reachable, for example a container without a
/// desktop session. The passwords stay in the process and reach no file.
#[derive(Default)]
pub struct MemoryStore {
    entries: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl SecretStore for MemoryStore {
    fn set(&self, id: &str, password: &str) -> Result<()> {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(id.to_string(), password.to_string());
        }
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(id).cloned()))
    }

    fn delete(&self, id: &str) -> Result<()> {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(id);
        }
        Ok(())
    }
}

/// Selects the store to use. The keychain is tried first, and the store in
/// memory takes over when the keychain refuses to work.
pub fn build_store() -> Box<dyn SecretStore> {
    let candidate = KeychainStore;
    let probe = "sql-explorer-probe";
    match candidate
        .set(probe, "probe")
        .and_then(|()| candidate.delete(probe))
    {
        Ok(()) => Box::new(candidate),
        Err(error) => {
            log::warn!(
                "The keychain of the system is not reachable, so the passwords stay in memory \
                 for this session only: {error}"
            );
            Box::new(MemoryStore::default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_secret_of_a_connection_takes_a_key_of_its_own() {
        assert_eq!(aws_secret_key("c1"), "c1:aws-secret-access-key");
        assert_eq!(aws_token_key("c1"), "c1:aws-session-token");
        // The three keys of one connection differ from each other.
        assert_ne!(aws_secret_key("c1"), aws_token_key("c1"));
        assert_ne!(aws_secret_key("c1"), "c1");
    }

    #[test]
    fn the_store_in_memory_holds_and_removes_a_password() {
        let store = MemoryStore::default();
        assert_eq!(store.get("a").unwrap(), None);

        store.set("a", "secret").unwrap();
        assert_eq!(store.get("a").unwrap().as_deref(), Some("secret"));

        store.set("a", "other").unwrap();
        assert_eq!(store.get("a").unwrap().as_deref(), Some("other"));

        store.delete("a").unwrap();
        assert_eq!(store.get("a").unwrap(), None);
        // A second removal is accepted.
        store.delete("a").unwrap();
    }

    #[test]
    fn the_selected_store_works() {
        let store = build_store();
        store.set("sql-explorer-test", "value").unwrap();
        assert_eq!(
            store.get("sql-explorer-test").unwrap().as_deref(),
            Some("value")
        );
        store.delete("sql-explorer-test").unwrap();
        assert_eq!(store.get("sql-explorer-test").unwrap(), None);
    }

    #[test]
    fn the_service_name_is_set() {
        assert_eq!(SERVICE, "com.c-wetherill.sql-explorer");
    }
}
