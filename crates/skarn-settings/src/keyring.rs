//! OS keyring access for the Skarn API key (PLAN.md §5, ARCHITECTURE.md §7).
//!
//! The key's home is the platform credential store — Windows Credential
//! Manager in the shipped build, via the `keyring` crate — under service
//! [`SERVICE`], entry [`ENTRY`].
//!
//! A plaintext fallback file in the settings dir (`%APPDATA%\skarn\
//! .api_key` / `.brave_key`) is kept **only** as a resilience path for
//! when the credential store is unavailable:
//! - `set` writes the keyring and drops any stale fallback copy; it writes
//!   the fallback only if the keyring write fails;
//! - `get` reads the keyring, migrates from [`LEGACY_SERVICE`] using
//!   read-old -> write-new -> verify-read-back -> delete-old -> delete-fallback,
//!   falls back to the file when the keyring has no usable entry, and
//!   opportunistically restores the keyring — a read never (re)writes the
//!   plaintext file;
//! - non-`NoEntry` keyring read errors are never swallowed into `Ok(None)`
//!   when no fallback key exists;
//! - `delete` removes the fallback first and surfaces a failure, so a key
//!   can never silently survive on disk after Disconnect.
//!
//! Semantics the callers rely on (ARCHITECTURE.md §7):
//! - a missing key is a *state*, not an error: `get` → `Ok(None)`;
//! - `delete` of a missing entry is `Ok(())` (Disconnect is idempotent);
//! - failures surface as [`SkarnError::Config`] carrying the keyring's
//!   own message — which never contains the key itself.

use std::path::{Path, PathBuf};

use skarn_core::SkarnError;

/// The keyring service: the app's stable identity in the credential store.
pub const SERVICE: &str = "skarn-ide";
/// Legacy v0.1.0 keyring service, read-only migrated on first access.
pub const LEGACY_SERVICE: &str = "selora-ide"; // skarn-v1-migration-compat
/// The keyring entry (the `user` field) holding the API key.
pub const ENTRY: &str = "api-key";
/// The entry holding the Brave Search API key (Phase R5: the agent
/// section's field stores it here — never in settings.json).
pub const BRAVE_ENTRY: &str = "brave-key";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CredentialError {
    NoEntry,
    Other(String),
}

pub(crate) trait CredentialStore {
    fn get_password(&self, service: &str, user: &str) -> Result<String, CredentialError>;
    fn set_password(&self, service: &str, user: &str, password: &str)
        -> Result<(), CredentialError>;
    fn delete_credential(&self, service: &str, user: &str) -> Result<(), CredentialError>;
}

struct OsCredentialStore;

impl OsCredentialStore {
    fn map_err(err: keyring::Error) -> CredentialError {
        match err {
            keyring::Error::NoEntry => CredentialError::NoEntry,
            other => CredentialError::Other(other.to_string()),
        }
    }
}

impl CredentialStore for OsCredentialStore {
    fn get_password(&self, service: &str, user: &str) -> Result<String, CredentialError> {
        let entry = keyring::Entry::new(service, user)
            .map_err(|err| CredentialError::Other(format!("keyring unavailable: {err}")))?;
        entry.get_password().map_err(Self::map_err)
    }

    fn set_password(
        &self,
        service: &str,
        user: &str,
        password: &str,
    ) -> Result<(), CredentialError> {
        let entry = keyring::Entry::new(service, user)
            .map_err(|err| CredentialError::Other(format!("keyring unavailable: {err}")))?;
        entry.set_password(password).map_err(Self::map_err)
    }

    fn delete_credential(&self, service: &str, user: &str) -> Result<(), CredentialError> {
        let entry = keyring::Entry::new(service, user)
            .map_err(|err| CredentialError::Other(format!("keyring unavailable: {err}")))?;
        entry.delete_credential().map_err(Self::map_err)
    }
}

/// The entry for `service` and `user`.
fn entry_for(service: &str, user: &str) -> Result<keyring::Entry, SkarnError> {
    keyring::Entry::new(service, user)
        .map_err(|err| SkarnError::Config(format!("keyring unavailable: {err}")))
}

/// The file the fallback copy of `user`'s key lives in inside `dir`.
///
/// `None` for a name Skarn does not own — the fallback only ever mirrors
/// the two documented entries.
fn fallback_path_in(dir: &Path, user: &str) -> Option<PathBuf> {
    let filename = match user {
        ENTRY => ".api_key",
        BRAVE_ENTRY => ".brave_key",
        _ => return None,
    };
    Some(dir.join(filename))
}

/// Read the fallback key for `user` from `dir`. `None` = absent, unreadable
/// or blank: the fallback is a resilience path, never an error state.
fn fallback_read_in(dir: &Path, user: &str) -> Option<String> {
    let path = fallback_path_in(dir, user)?;
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Write `key` (trimmed) as the fallback copy for `user` inside `dir`.
fn fallback_write_in(dir: &Path, user: &str, key: &str) -> Result<(), SkarnError> {
    let Some(path) = fallback_path_in(dir, user) else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            SkarnError::Config(format!(
                "could not create the fallback key directory: {err}"
            ))
        })?;
    }
    std::fs::write(&path, key.trim()).map_err(|err| {
        SkarnError::Config(format!(
            "could not write the fallback key file {}: {err}",
            path.display()
        ))
    })
}

/// Write the live fallback copy for `user` (settings dir).
fn fallback_set(user: &str, key: &str) -> Result<(), SkarnError> {
    let dir = crate::Settings::settings_dir()?;
    fallback_write_in(&dir, user, key)
}

/// Remove the fallback copy for `user` inside `dir`.
///
/// A missing file is `Ok(())` (Disconnect is idempotent); every other
/// failure is reported so a key can never silently survive on disk.
fn fallback_delete_in(dir: &Path, user: &str) -> Result<(), SkarnError> {
    let Some(path) = fallback_path_in(dir, user) else {
        return Ok(());
    };
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(SkarnError::Config(format!(
            "could not remove the fallback key file {}: {err}",
            path.display()
        ))),
    }
}

/// Remove the live fallback copy for `user` (settings dir).
fn fallback_delete(user: &str) -> Result<(), SkarnError> {
    match crate::Settings::settings_dir() {
        Ok(dir) => fallback_delete_in(&dir, user),
        // Without a settings directory there is no fallback file to remove
        // (`set` could never have written one either).
        Err(_) => Ok(()),
    }
}

fn find_fallback_in(
    user: &str,
    new_dir: Option<&Path>,
    old_dir: Option<&Path>,
) -> Option<String> {
    new_dir
        .and_then(|dir| fallback_read_in(dir, user))
        .or_else(|| old_dir.and_then(|dir| fallback_read_in(dir, user)))
}

fn cleanup_fallbacks_in(user: &str, new_dir: Option<&Path>, old_dir: Option<&Path>) {
    if let Some(dir) = new_dir {
        let _ = fallback_delete_in(dir, user);
    }
    if let Some(dir) = old_dir {
        let _ = fallback_delete_in(dir, user);
    }
}

/// Strict migration sequence for `user`:
/// 1. Read `(new_service, user)`: if non-blank, clean fallbacks in both dirs and return `Ok(Some(k))`.
///    If `Err(Other(e))`, check fallback first else return `Err(SkarnError::Config(...))` (never swallow).
/// 2. On `NoEntry` (or blank), read `(old_service, user)`:
///    - If `Ok(old_k)` (non-blank): write `new_service`, verify read-back equality, delete `old_service`,
///      delete plaintext fallbacks in both dirs, log one secret-free line, return `Ok(Some(old_k))`.
///      Any write/verify failure aborts immediately with `Err` without deleting `old_service` or fallbacks.
///    - If `NoEntry` (or blank): check fallback in `new_dir`, then `old_dir`; if found, restore + verify
///      into `new_service` and delete both fallbacks only when verified.
///    - If `Err(Other(e))`: check fallback first else return `Err(SkarnError::Config(...))`.
pub(crate) fn migrate_and_get_in(
    store: &dyn CredentialStore,
    new_service: &str,
    old_service: &str,
    user: &str,
    new_dir: Option<&Path>,
    old_dir: Option<&Path>,
) -> Result<Option<String>, SkarnError> {
    match store.get_password(new_service, user) {
        Ok(key) if !key.trim().is_empty() => {
            cleanup_fallbacks_in(user, new_dir, old_dir);
            return Ok(Some(key));
        }
        Ok(_) | Err(CredentialError::NoEntry) => {}
        Err(CredentialError::Other(err)) => {
            if let Some(key) = find_fallback_in(user, new_dir, old_dir) {
                return Ok(Some(key));
            }
            return Err(SkarnError::Config(format!("keyring read failed: {err}")));
        }
    }

    match store.get_password(old_service, user) {
        Ok(old_key) if !old_key.trim().is_empty() => {
            store.set_password(new_service, user, &old_key).map_err(|err| {
                let msg = match err {
                    CredentialError::NoEntry => "no entry after write".to_string(),
                    CredentialError::Other(m) => m,
                };
                SkarnError::Config(format!("keyring migration write failed: {msg}"))
            })?;
            let read_back = store.get_password(new_service, user).map_err(|err| {
                let msg = match err {
                    CredentialError::NoEntry => "missing after write".to_string(),
                    CredentialError::Other(m) => m,
                };
                SkarnError::Config(format!("keyring migration verify failed: {msg}"))
            })?;
            if read_back != old_key {
                return Err(SkarnError::Config(
                    "keyring migration verify failed: read-back mismatch".into(),
                ));
            }
            match store.delete_credential(old_service, user) {
                Ok(()) | Err(CredentialError::NoEntry) => {}
                Err(CredentialError::Other(err)) => {
                    eprintln!(
                        "[skarn-keyring] warning: could not delete legacy credential \"{user}\" from {old_service}: {err}"
                    );
                }
            }
            cleanup_fallbacks_in(user, new_dir, old_dir);
            eprintln!(
                "[skarn-keyring] migrated credential \"{user}\" from {old_service} to {new_service}"
            );
            Ok(Some(old_key))
        }
        Ok(_) | Err(CredentialError::NoEntry) => {
            if let Some(key) = find_fallback_in(user, new_dir, old_dir) {
                if store.set_password(new_service, user, &key).is_ok()
                    && store.get_password(new_service, user).as_deref() == Ok(key.as_str())
                {
                    cleanup_fallbacks_in(user, new_dir, old_dir);
                }
                Ok(Some(key))
            } else {
                Ok(None)
            }
        }
        Err(CredentialError::Other(err)) => {
            if let Some(key) = find_fallback_in(user, new_dir, old_dir) {
                return Ok(Some(key));
            }
            Err(SkarnError::Config(format!("keyring read failed: {err}")))
        }
    }
}

/// Store `key` under `user`: the keyring is the home.
///
/// Only when the credential store cannot take it does the plaintext
/// fallback become the resilience copy — the read path prefers the keyring
/// and restores it once the store is available again. `Ok` means the key
/// was persisted somewhere; `Err` means it was persisted nowhere.
fn store_key(
    user: &str,
    key: &str,
    entry: Result<keyring::Entry, SkarnError>,
) -> Result<(), SkarnError> {
    let keyring_error = match entry {
        Ok(entry) => match entry.set_password(key) {
            Ok(()) => {
                // The keyring is authoritative now: drop a stale plaintext
                // copy left by an older build or an earlier keyring outage.
                // Best-effort — the write itself already succeeded.
                let _ = fallback_delete(user);
                if let Some(old_dir) = crate::settings::legacy_settings_dir_if_active() {
                    let _ = fallback_delete_in(&old_dir, user);
                }
                return Ok(());
            }
            Err(err) => format!("keyring write failed: {err}"),
        },
        Err(SkarnError::Config(message)) => message,
        Err(other) => other.to_string(),
    };
    fallback_set(user, key)
        .map_err(|fallback_error| SkarnError::Config(format!("{keyring_error}; {fallback_error}")))
}

/// Read the stored API key. `Ok(None)` = nothing stored yet (first run,
/// or after Disconnect).
pub fn api_key_get() -> Result<Option<String>, SkarnError> {
    let new_dir = crate::Settings::settings_dir().ok();
    let old_dir = crate::settings::legacy_settings_dir_if_active();
    migrate_and_get_in(
        &OsCredentialStore,
        SERVICE,
        LEGACY_SERVICE,
        ENTRY,
        new_dir.as_deref(),
        old_dir.as_deref(),
    )
}

/// Store (overwrite) the API key — the Connect flow's final step, after
/// validation succeeded.
pub fn api_key_set(key: &str) -> Result<(), SkarnError> {
    store_key(ENTRY, key, entry_for(SERVICE, ENTRY))
}

/// Read the stored Brave Search API key. `Ok(None)` = nothing stored.
/// Phase R5: the agent's web_search tool reads it through the shell's
/// turn-request seam; settings.json never carries it.
pub fn brave_key_get() -> Result<Option<String>, SkarnError> {
    let new_dir = crate::Settings::settings_dir().ok();
    let old_dir = crate::settings::legacy_settings_dir_if_active();
    migrate_and_get_in(
        &OsCredentialStore,
        SERVICE,
        LEGACY_SERVICE,
        BRAVE_ENTRY,
        new_dir.as_deref(),
        old_dir.as_deref(),
    )
}

/// Migrate both [`ENTRY`] and [`BRAVE_ENTRY`] from [`LEGACY_SERVICE`] into [`SERVICE`].
pub fn migrate_legacy_credentials() -> Result<(), SkarnError> {
    api_key_get()?;
    brave_key_get()?;
    Ok(())
}

/// Store (overwrite) the Brave Search API key — the Agent section's
/// field, on commit.
pub fn brave_key_set(key: &str) -> Result<(), SkarnError> {
    store_key(BRAVE_ENTRY, key, entry_for(SERVICE, BRAVE_ENTRY))
}

fn delete_key(user: &str) -> Result<(), SkarnError> {
    // The fallback goes first: if the plaintext copy cannot be removed the
    // caller must see the error — the key must not silently stay on disk
    // after Disconnect.
    fallback_delete(user)?;
    if let Some(old_dir) = crate::settings::legacy_settings_dir_if_active() {
        let _ = fallback_delete_in(&old_dir, user);
    }
    match entry_for(SERVICE, user)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(err) => return Err(SkarnError::Config(format!("keyring delete failed: {err}"))),
    }
    if let Ok(legacy) = entry_for(LEGACY_SERVICE, user) {
        let _ = legacy.delete_credential();
    }
    Ok(())
}

/// Remove the stored Brave Search API key. A missing entry is a no-op.
pub fn brave_key_delete() -> Result<(), SkarnError> {
    delete_key(BRAVE_ENTRY)
}

/// Remove the stored API key (Disconnect). A missing entry is a no-op.
pub fn api_key_delete() -> Result<(), SkarnError> {
    delete_key(ENTRY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    /// A throwaway service name so the round-trips below never read,
    /// overwrite, or delete the owner's real `skarn-ide` entry.
    fn test_service() -> String {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        format!("skarn-ide-keyring-test-{}-{n}", std::process::id())
    }

    #[test]
    fn round_trip_set_get_delete() {
        let service = test_service();
        let entry = entry_for(&service, ENTRY).expect("entry");

        entry.set_password("sk-gw-roundtrip").expect("set");
        assert_eq!(entry.get_password().expect("get"), "sk-gw-roundtrip");

        entry.delete_credential().expect("delete");
        // The raw entry delete of a missing entry is NoEntry — the public
        // `api_key_delete` normalizes that to `Ok(())` (idempotent
        // Disconnect); this test asserts the raw backend's shape.
        match entry.delete_credential() {
            Err(keyring::Error::NoEntry) => {}
            other => panic!("expected NoEntry on second delete, got {other:?}"),
        }
        match entry.get_password() {
            Err(keyring::Error::NoEntry) => {}
            other => panic!("expected NoEntry after delete, got {other:?}"),
        }
    }

    /// Phase R5: the Brave entry round-trips under the same throwaway
    /// service discipline (the real service's entry is never touched).
    #[test]
    fn brave_entry_round_trip() {
        let service = test_service();
        let entry = entry_for(&service, BRAVE_ENTRY).expect("brave entry");

        entry.set_password("BSA-key").expect("set");
        assert_eq!(entry.get_password().expect("get"), "BSA-key");
        entry.delete_credential().expect("delete");
        match entry.get_password() {
            Err(keyring::Error::NoEntry) => {}
            other => panic!("expected NoEntry after delete, got {other:?}"),
        }
    }

    #[test]
    fn get_and_delete_of_a_missing_entry_are_ok() {
        let service = test_service();
        // Never written under this unique service name.
        match entry_for(&service, ENTRY).expect("entry").get_password() {
            Err(keyring::Error::NoEntry) => {}
            other => panic!("expected NoEntry, got {other:?}"),
        }
        // Raw backend: delete of a missing entry is NoEntry (the public
        // `api_key_delete` normalizes it to `Ok(())`).
        match entry_for(&service, ENTRY)
            .expect("entry")
            .delete_credential()
        {
            Err(keyring::Error::NoEntry) => {}
            other => panic!("expected NoEntry, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // The plaintext fallback: exercised through the directory seam, so
    // these tests never touch a credential store at all (throwaway or
    // otherwise) — they only read/write a temp settings dir.
    // -----------------------------------------------------------------

    fn fallback_dir(label: &str) -> PathBuf {
        crate::settings::tests::temp_settings_dir(label)
    }

    #[test]
    fn fallback_write_read_delete_round_trips_in_a_temp_dir() {
        let dir = fallback_dir("keyring-fallback-round-trip");
        std::fs::create_dir_all(&dir).expect("temp dir");

        // Absent → None (a missing fallback is a state, not an error).
        assert_eq!(fallback_read_in(&dir, ENTRY), None);

        // Written trimmed, under exactly the documented filename.
        fallback_write_in(&dir, ENTRY, "  sk-gw-fallback \n").expect("write");
        assert_eq!(
            fallback_read_in(&dir, ENTRY).as_deref(),
            Some("sk-gw-fallback")
        );
        assert!(
            dir.join(".api_key").is_file(),
            "the fallback lives at <settings dir>/.api_key"
        );

        fallback_delete_in(&dir, ENTRY).expect("delete");
        assert_eq!(fallback_read_in(&dir, ENTRY), None);
        // Disconnect is idempotent: a second delete is still Ok.
        fallback_delete_in(&dir, ENTRY).expect("second delete");

        // The Brave key lives in its own file and never collides.
        fallback_write_in(&dir, BRAVE_ENTRY, "BSA-fallback").expect("brave write");
        assert!(dir.join(".brave_key").is_file());
        assert_eq!(
            fallback_read_in(&dir, BRAVE_ENTRY).as_deref(),
            Some("BSA-fallback")
        );
        assert_eq!(fallback_read_in(&dir, ENTRY), None);
        fallback_delete_in(&dir, BRAVE_ENTRY).expect("brave delete");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fallback_read_rejects_blank_and_unknown_entries() {
        let dir = fallback_dir("keyring-fallback-blank");
        std::fs::create_dir_all(&dir).expect("temp dir");

        // Whitespace-only content is not a key.
        fallback_write_in(&dir, ENTRY, "  \n\t ").expect("write");
        assert_eq!(fallback_read_in(&dir, ENTRY), None);

        // Only the two documented entries have a fallback file.
        assert!(fallback_path_in(&dir, "not-a-skarn-entry").is_none());
        assert!(fallback_path_in(&dir, ENTRY).is_some());
        assert!(fallback_path_in(&dir, BRAVE_ENTRY).is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fallback_delete_reports_a_path_it_cannot_remove() {
        let dir = fallback_dir("keyring-fallback-delete-error");
        // A directory squatting on the key path makes `remove_file` fail on
        // every supported platform: the helper must report it instead of
        // pretending the key is gone (goal: no silent plaintext survivor).
        std::fs::create_dir_all(dir.join(".api_key")).expect("dir at the key path");

        let err = fallback_delete_in(&dir, ENTRY).expect_err("must report the failure");
        assert!(
            matches!(err, SkarnError::Config(ref message) if message.contains("could not remove the fallback key file")),
            "unexpected error: {err:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[derive(Default)]
    struct MockCredentialStore {
        entries: std::sync::Mutex<std::collections::HashMap<(String, String), String>>,
        get_error: std::sync::Mutex<Option<(String, String)>>,
        set_error: std::sync::Mutex<Option<String>>,
        read_back_override: std::sync::Mutex<Option<String>>,
    }

    impl MockCredentialStore {
        fn insert(&self, service: &str, user: &str, value: &str) {
            self.entries
                .lock()
                .unwrap()
                .insert((service.to_owned(), user.to_owned()), value.to_owned());
        }

        fn get(&self, service: &str, user: &str) -> Option<String> {
            self.entries
                .lock()
                .unwrap()
                .get(&(service.to_owned(), user.to_owned()))
                .cloned()
        }
    }

    impl CredentialStore for MockCredentialStore {
        fn get_password(&self, service: &str, user: &str) -> Result<String, CredentialError> {
            if let Some((err_svc, msg)) = self.get_error.lock().unwrap().as_ref() {
                if err_svc == service {
                    return Err(CredentialError::Other(msg.clone()));
                }
            }
            let stored = self
                .entries
                .lock()
                .unwrap()
                .get(&(service.to_owned(), user.to_owned()))
                .cloned();
            if service == SERVICE && stored.is_some() {
                if let Some(corrupt) = self.read_back_override.lock().unwrap().clone() {
                    return Ok(corrupt);
                }
            }
            stored.ok_or(CredentialError::NoEntry)
        }

        fn set_password(
            &self,
            service: &str,
            user: &str,
            password: &str,
        ) -> Result<(), CredentialError> {
            if let Some(msg) = self.set_error.lock().unwrap().as_ref() {
                return Err(CredentialError::Other(msg.clone()));
            }
            self.entries
                .lock()
                .unwrap()
                .insert((service.to_owned(), user.to_owned()), password.to_owned());
            Ok(())
        }

        fn delete_credential(&self, service: &str, user: &str) -> Result<(), CredentialError> {
            self.entries
                .lock()
                .unwrap()
                .remove(&(service.to_owned(), user.to_owned()))
                .map(|_| ())
                .ok_or(CredentialError::NoEntry)
        }
    }

    #[test]
    fn keyring_migration_copies_verifies_deletes_old_and_cleans_plaintext() {
        let store = MockCredentialStore::default();
        store.insert(LEGACY_SERVICE, ENTRY, "sk-legacy-secret");
        let new_dir = fallback_dir("keyring-migrate-new");
        let old_dir = fallback_dir("keyring-migrate-old");
        fallback_write_in(&new_dir, ENTRY, "sk-stale-new").expect("write new fallback");
        fallback_write_in(&old_dir, ENTRY, "sk-stale-old").expect("write old fallback");

        let got = migrate_and_get_in(
            &store,
            SERVICE,
            LEGACY_SERVICE,
            ENTRY,
            Some(&new_dir),
            Some(&old_dir),
        )
        .expect("migration succeeds");

        assert_eq!(got.as_deref(), Some("sk-legacy-secret"));
        assert_eq!(store.get(SERVICE, ENTRY).as_deref(), Some("sk-legacy-secret"));
        assert_eq!(
            store.get(LEGACY_SERVICE, ENTRY),
            None,
            "legacy service entry must be deleted after verified write"
        );
        assert!(!new_dir.join(".api_key").exists(), "new fallback must be deleted");
        assert!(!old_dir.join(".api_key").exists(), "old fallback must be deleted");

        let _ = std::fs::remove_dir_all(&new_dir);
        let _ = std::fs::remove_dir_all(&old_dir);
    }

    #[test]
    fn keyring_migration_aborts_on_write_or_verify_failure_without_data_loss() {
        // Case 1: write failure retains old_service and fallback.
        let store = MockCredentialStore::default();
        store.insert(LEGACY_SERVICE, ENTRY, "sk-legacy-keep");
        *store.set_error.lock().unwrap() = Some("vault locked".into());
        let old_dir = fallback_dir("keyring-abort-old");
        fallback_write_in(&old_dir, ENTRY, "sk-fallback-keep").expect("write fallback");

        let err = migrate_and_get_in(
            &store,
            SERVICE,
            LEGACY_SERVICE,
            ENTRY,
            None,
            Some(&old_dir),
        )
        .expect_err("write failure must abort");
        assert!(matches!(err, SkarnError::Config(ref m) if m.contains("vault locked")));
        assert_eq!(
            store.get(LEGACY_SERVICE, ENTRY).as_deref(),
            Some("sk-legacy-keep"),
            "legacy entry must stay intact on write failure"
        );
        assert!(old_dir.join(".api_key").is_file(), "fallback must stay intact");

        // Case 2: read-back mismatch aborts without deleting old_service or fallback.
        *store.set_error.lock().unwrap() = None;
        *store.read_back_override.lock().unwrap() = Some("sk-corrupted".into());
        let _ = store.delete_credential(SERVICE, ENTRY);
        let err = migrate_and_get_in(
            &store,
            SERVICE,
            LEGACY_SERVICE,
            ENTRY,
            None,
            Some(&old_dir),
        );
        assert!(err.is_err());
        assert_eq!(
            store.get(LEGACY_SERVICE, ENTRY).as_deref(),
            Some("sk-legacy-keep"),
            "legacy entry must stay intact on verify mismatch"
        );
        assert!(old_dir.join(".api_key").is_file(), "fallback must stay intact");

        let _ = std::fs::remove_dir_all(&old_dir);
    }

    #[test]
    fn keyring_read_error_surfaces_instead_of_returning_none() {
        let store = MockCredentialStore::default();
        *store.get_error.lock().unwrap() = Some((SERVICE.to_owned(), "access denied".into()));
        let empty_dir = fallback_dir("keyring-read-err");
        std::fs::create_dir_all(&empty_dir).expect("mkdir");

        let err = migrate_and_get_in(
            &store,
            SERVICE,
            LEGACY_SERVICE,
            ENTRY,
            Some(&empty_dir),
            None,
        )
        .expect_err("non-NoEntry error must never be swallowed into Ok(None)");
        assert!(
            matches!(err, SkarnError::Config(ref m) if m.contains("access denied")),
            "unexpected error: {err:?}"
        );

        let _ = std::fs::remove_dir_all(&empty_dir);
    }
}
