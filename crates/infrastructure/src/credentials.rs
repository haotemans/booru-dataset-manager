use bdm_ports::{
    ApiCredentialStore, ApiCredentials, CredentialError, SourceCredentialStore, SourceCredentials,
};

#[derive(Default)]
pub struct NativeApiCredentialStore;

impl NativeApiCredentialStore {
    fn profile_store(profile_id: &str) -> Result<NativeCredentialStore, CredentialError> {
        if profile_id.is_empty()
            || profile_id.len() > 128
            || !profile_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err(CredentialError::InvalidData);
        }
        Ok(NativeCredentialStore {
            #[cfg(windows)]
            service: "Booru Dataset Manager".into(),
            #[cfg(windows)]
            account: format!("ai-{profile_id}"),
        })
    }
}

impl ApiCredentialStore for NativeApiCredentialStore {
    fn load(&self, profile_id: &str) -> Result<Option<ApiCredentials>, CredentialError> {
        Ok(Self::profile_store(profile_id)?
            .load()?
            .map(|value| ApiCredentials {
                endpoint: value.username,
                api_key: value.api_key,
            }))
    }
    fn save(&self, profile_id: &str, credentials: &ApiCredentials) -> Result<(), CredentialError> {
        Self::profile_store(profile_id)?.save(&SourceCredentials {
            username: credentials.endpoint.clone(),
            api_key: credentials.api_key.clone(),
        })
    }
    fn clear(&self, profile_id: &str) -> Result<(), CredentialError> {
        Self::profile_store(profile_id)?.clear()
    }
}

pub struct NativeCredentialStore {
    #[cfg(windows)]
    service: String,
    #[cfg(windows)]
    account: String,
}

impl NativeCredentialStore {
    pub fn danbooru() -> Self {
        Self {
            #[cfg(windows)]
            service: "Booru Dataset Manager".into(),
            #[cfg(windows)]
            account: "danbooru".into(),
        }
    }

    #[cfg(windows)]
    fn entry(&self) -> Result<keyring::Entry, CredentialError> {
        keyring::Entry::new(&self.service, &self.account).map_err(|_| CredentialError::Unavailable)
    }
}

#[cfg(windows)]
impl SourceCredentialStore for NativeCredentialStore {
    fn load(&self) -> Result<Option<SourceCredentials>, CredentialError> {
        match self.entry()?.get_password() {
            Ok(value) => {
                let (username, api_key) =
                    value.split_once('\n').ok_or(CredentialError::InvalidData)?;
                if username.trim().is_empty()
                    || username.chars().any(char::is_control)
                    || api_key.trim().is_empty()
                    || api_key.chars().any(char::is_control)
                {
                    return Err(CredentialError::InvalidData);
                }
                Ok(Some(SourceCredentials {
                    username: username.into(),
                    api_key: api_key.into(),
                }))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(CredentialError::Access),
        }
    }

    fn save(&self, credentials: &SourceCredentials) -> Result<(), CredentialError> {
        if credentials.username.is_empty()
            || credentials.api_key.is_empty()
            || credentials.username.chars().any(char::is_control)
            || credentials.api_key.chars().any(char::is_control)
            || credentials.username.len() + credentials.api_key.len() > 1279
        {
            return Err(CredentialError::InvalidData);
        }
        // Store the identity/key as one credential to avoid cross-account partial writes.
        self.entry()?
            .set_password(&format!(
                "{}\n{}",
                credentials.username, credentials.api_key
            ))
            .map_err(|_| CredentialError::Access)
    }

    fn clear(&self) -> Result<(), CredentialError> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(CredentialError::Access),
        }
    }
}

#[cfg(not(windows))]
impl SourceCredentialStore for NativeCredentialStore {
    fn load(&self) -> Result<Option<SourceCredentials>, CredentialError> {
        Err(CredentialError::Unavailable)
    }
    fn save(&self, _: &SourceCredentials) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
    fn clear(&self) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Explicit Windows Credential Manager smoke test; uses only a unique synthetic test entry"]
    fn native_credential_round_trip() {
        let store = NativeCredentialStore {
            service: format!(
                "Booru Dataset Manager integration test {}",
                std::process::id()
            ),
            account: "synthetic-danbooru".into(),
        };
        assert!(store.load().unwrap().is_none());
        store
            .save(&SourceCredentials {
                username: "synthetic_user".into(),
                api_key: "synthetic-key-not-a-real-token".into(),
            })
            .unwrap();
        let loaded = store.load();
        // Clean up before assertions, including a failed-read assertion.
        let cleanup = store.clear();
        let loaded = loaded.unwrap().unwrap();
        assert_eq!(loaded.username, "synthetic_user");
        assert_eq!(loaded.api_key, "synthetic-key-not-a-real-token");
        cleanup.unwrap();
        assert!(store.load().unwrap().is_none());
        store.clear().unwrap();

        let api_store = NativeApiCredentialStore;
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let a = format!("synthetic-test-{}-{suffix}-a", std::process::id());
        let b = format!("synthetic-test-{}-{suffix}-b", std::process::id());
        let result = (|| -> Result<_, CredentialError> {
            api_store.save(
                &a,
                &ApiCredentials {
                    endpoint: "https://example.invalid/v1".into(),
                    api_key: "synthetic-a".into(),
                },
            )?;
            api_store.save(
                &b,
                &ApiCredentials {
                    endpoint: "https://other.invalid/v1".into(),
                    api_key: "synthetic-b".into(),
                },
            )?;
            let first = api_store.load(&a)?;
            api_store.clear(&a)?;
            Ok((first, api_store.load(&a)?, api_store.load(&b)?))
        })();
        // Always attempt both cleanups before unwrapping the test's result.
        let cleanup_a = api_store.clear(&a);
        let cleanup_b = api_store.clear(&b);
        let (first, removed, second) = result.unwrap();
        cleanup_a.unwrap();
        cleanup_b.unwrap();
        let first = first.unwrap();
        let second = second.unwrap();
        assert_eq!(first.endpoint, "https://example.invalid/v1");
        assert_eq!(first.api_key, "synthetic-a");
        assert!(removed.is_none());
        assert_eq!(second.api_key, "synthetic-b");
        assert!(api_store.load(&a).unwrap().is_none());
        assert!(api_store.load(&b).unwrap().is_none());
    }
}
