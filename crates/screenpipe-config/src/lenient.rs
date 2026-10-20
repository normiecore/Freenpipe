// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Key-by-key tolerant loading of settings objects.
//!
//! A settings store can be written by several app versions (screenpipe and
//! Freenpipe releases share `store.bin`). When one key holds a value of a
//! type this version does not expect, strict deserialisation would reject
//! the whole object, and the caller would either refuse to start or fall
//! back to defaults. Neither is acceptable for user data.
//!
//! [`from_value_lenient`] instead sets such keys aside: the typed value gets
//! its default for them, and the caller keeps the original JSON so it can be
//! written back untouched. Only when the input is not an object at all does
//! loading fail.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// Result of a tolerant load.
#[derive(Debug)]
pub struct Lenient<T> {
    /// The typed value. Keys listed in `rejected` hold their defaults.
    pub value: T,
    /// Entries whose values did not fit their typed field, unchanged.
    pub rejected: Map<String, Value>,
}

/// Deserialise `value` into `T`, setting aside top-level entries that do not
/// fit. `T` must accept an empty object (every field has a default).
///
/// The fast path is a single strict parse. Only when that fails is each entry
/// probed on its own, so the cost is paid by broken stores alone.
pub fn from_value_lenient<T: DeserializeOwned>(value: Value) -> Result<Lenient<T>, serde_json::Error> {
    let first_error = match serde_json::from_value::<T>(value.clone()) {
        Ok(parsed) => {
            return Ok(Lenient {
                value: parsed,
                rejected: Map::new(),
            })
        }
        Err(e) => e,
    };
    let Value::Object(entries) = value else {
        return Err(first_error);
    };

    let mut kept = Map::new();
    let mut rejected = Map::new();
    for (key, entry) in entries {
        let mut probe = Map::new();
        probe.insert(key.clone(), entry.clone());
        if serde_json::from_value::<T>(Value::Object(probe)).is_ok() {
            kept.insert(key, entry);
        } else {
            rejected.insert(key, entry);
        }
    }

    let parsed = serde_json::from_value::<T>(Value::Object(kept))?;
    Ok(Lenient {
        value: parsed,
        rejected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Debug, Deserialize, Default, PartialEq)]
    #[serde(default)]
    struct Sample {
        port: u16,
        name: String,
        tags: Vec<String>,
    }

    #[test]
    fn well_formed_input_takes_the_fast_path() {
        let loaded: Lenient<Sample> =
            from_value_lenient(json!({"port": 4040, "name": "x", "extra": true})).unwrap();
        assert_eq!(loaded.value.port, 4040);
        assert!(loaded.rejected.is_empty());
    }

    #[test]
    fn mistyped_entries_are_set_aside_unchanged() {
        let loaded: Lenient<Sample> = from_value_lenient(json!({
            "port": "not a number",
            "name": "kept",
            "tags": {"unexpected": "shape"}
        }))
        .unwrap();
        assert_eq!(loaded.value.name, "kept");
        assert_eq!(loaded.value.port, 0);
        assert!(loaded.value.tags.is_empty());
        assert_eq!(loaded.rejected["port"], json!("not a number"));
        assert_eq!(loaded.rejected["tags"], json!({"unexpected": "shape"}));
        assert_eq!(loaded.rejected.len(), 2);
    }

    #[test]
    fn non_objects_still_fail() {
        assert!(from_value_lenient::<Sample>(json!(["a", "b"])).is_err());
        assert!(from_value_lenient::<Sample>(json!("text")).is_err());
    }
}
