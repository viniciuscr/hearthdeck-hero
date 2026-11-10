//! The Stremio session key. It is a bearer token: never logged, never shown,
//! never part of an error or a crash report.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, PartialEq, Eq)]
pub struct AuthKey(String);

impl AuthKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for AuthKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AuthKey(<redacted>)")
    }
}

impl fmt::Display for AuthKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl Serialize for AuthKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for AuthKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_are_redacted() {
        let key = AuthKey::new("secret-key");
        assert!(!format!("{key:?}").contains("secret-key"));
        assert!(!format!("{key}").contains("secret-key"));
        assert_eq!(format!("{key:?}"), "AuthKey(<redacted>)");
    }

    #[test]
    fn serialization_is_the_wire_value() {
        let key = AuthKey::new("abc");
        assert_eq!(serde_json::to_string(&key).unwrap(), "\"abc\"");
    }
}
