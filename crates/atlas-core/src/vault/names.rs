//! A locked display name is readable by this Windows account, after the UI grant
//! check. It is separate from the password-encrypted manifest and never plaintext
//! in the public header, profile preferences or the physical container name.
use super::Envelope;

#[cfg(windows)]
fn context(envelope: &Envelope) -> Vec<u8> {
    // Bind the label to both this container and its exact authorized profiles.
    serde_json::to_vec(&(
        "atlas-vault-display-name-v1",
        &envelope.id,
        &envelope.profiles,
    ))
    .expect("serializing display-name context")
}

pub(super) fn seal(envelope: &Envelope, name: &str) -> Vec<u8> {
    #[cfg(windows)]
    {
        match atlas_win_space::data_protection::protect(name.as_bytes(), &context(envelope)) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(%error, "Private folder display name could not be remembered");
                Vec::new()
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (envelope, name);
        Vec::new()
    }
}

pub(super) fn open(envelope: &Envelope) -> Option<String> {
    #[cfg(windows)]
    {
        let bytes = zeroize::Zeroizing::new(
            atlas_win_space::data_protection::unprotect(&envelope.sealed_name, &context(envelope))
                .ok()?,
        );
        let name = std::str::from_utf8(&bytes).ok()?;
        super::validate_name(name).ok()?;
        Some(name.to_owned())
    }
    #[cfg(not(windows))]
    {
        let _ = envelope;
        None
    }
}
