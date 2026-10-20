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
//!
//! Callers usually keep the set-aside entries in a flattened catch-all map,
//! next to the keys they do not model. [`merge_catch_all`] writes such a value
//! back: a set-aside entry is kept only until code assigns its field.

use serde::de::DeserializeOwned;
use serde::Serialize;
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
pub fn from_value_lenient<T: DeserializeOwned>(
    value: Value,
) -> Result<Lenient<T>, serde_json::Error> {
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

/// Build the JSON written back for a value loaded with [`from_value_lenient`].
///
/// `typed` is the value serialised without its catch-all map, and
/// `catch_all` that map's entries. An entry whose key `typed` does not write
/// is a key this version does not model and is added unchanged. An entry
/// whose key `typed` also writes was set aside because its stored value did
/// not fit the field. It is written back only while the field still holds the
/// value an absent key loads as. Once code has assigned the field something
/// else, the assigned value is written and the stale stored value is dropped.
///
/// Code that assigns a field exactly its absent-key value cannot be told
/// apart from code that left it alone, so the stored value stays. A field
/// whose absent-key value is generated anew (a fresh id) always counts as
/// assigned.
pub fn merge_catch_all<T>(
    typed: Value,
    catch_all: impl IntoIterator<Item = (String, Value)>,
) -> Value
where
    T: Serialize + DeserializeOwned,
{
    let Value::Object(mut out) = typed else {
        return typed;
    };
    // Built on the first collision only; most stores have none.
    let mut absent: Option<Map<String, Value>> = None;
    for (key, stored) in catch_all {
        let Some(current) = out.get(&key) else {
            out.insert(key, stored);
            continue;
        };
        let absent = absent.get_or_insert_with(absent_key_values::<T>);
        match absent.get(&key) {
            // Code changed the field: the typed value stays.
            Some(loaded_as) if loaded_as != current => {}
            _ => {
                out.insert(key, stored);
            }
        }
    }
    Value::Object(out)
}

/// What each field of `T` holds when its key is absent, serialised. Empty
/// when `T` does not accept an empty object.
fn absent_key_values<T: Serialize + DeserializeOwned>() -> Map<String, Value> {
    match serde_json::from_value::<T>(Value::Object(Map::new())).and_then(serde_json::to_value) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;
    use std::collections::HashMap;

    #[derive(Debug, Deserialize, Default, PartialEq)]
    #[serde(default)]
    struct Sample {
        port: u16,
        name: String,
        tags: Vec<String>,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(default)]
    struct WithCatchAll {
        port: u16,
        enabled: bool,
        #[serde(flatten)]
        extra: HashMap<String, Value>,
    }

    impl Default for WithCatchAll {
        fn default() -> Self {
            Self {
                port: 3030,
                enabled: false,
                extra: HashMap::new(),
            }
        }
    }

    fn load(value: Value) -> WithCatchAll {
        let loaded: Lenient<WithCatchAll> = from_value_lenient(value).unwrap();
        let mut typed = loaded.value;
        typed.extra.extend(loaded.rejected);
        typed
    }

    fn write(value: &WithCatchAll) -> Value {
        let mut typed = value.clone();
        let catch_all = std::mem::take(&mut typed.extra);
        merge_catch_all::<WithCatchAll>(serde_json::to_value(&typed).unwrap(), catch_all)
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
    fn untouched_set_aside_entries_are_written_back_as_stored() {
        let value = load(json!({"port": "not a port", "enabled": true, "future": [1]}));
        assert_eq!(value.port, 3030);
        let written = write(&value);
        assert_eq!(written["port"], json!("not a port"));
        assert_eq!(written["enabled"], json!(true));
        assert_eq!(written["future"], json!([1]));
    }

    #[test]
    fn an_assigned_field_replaces_its_set_aside_entry() {
        let mut value = load(json!({"port": "not a port", "enabled": "yes"}));
        value.port = 4040;
        let written = write(&value);
        assert_eq!(written["port"], json!(4040), "the assigned value wins");
        assert_eq!(written["enabled"], json!("yes"), "untouched entries stay");
    }

    #[test]
    fn non_objects_still_fail() {
        assert!(from_value_lenient::<Sample>(json!(["a", "b"])).is_err());
        assert!(from_value_lenient::<Sample>(json!("text")).is_err());
    }
}
