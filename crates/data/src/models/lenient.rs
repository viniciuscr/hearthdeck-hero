//! Tolerant serde deserializers for files written by Heroic, `gogdl`, `nile`
//! or older versions of Heronite, where text fields may arrive as numbers or
//! `null` (e.g. GOG `buildId: 44535815`, numeric `appName`, `versionEtag: null`).

use serde::{Deserialize, Deserializer};
use serde_json::Value;

fn value_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null => None,
        _ => None,
    }
}

/// Deserializes a string, number, bool or `null` into a `String` (`null` -> `""`).
pub fn string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(v.as_ref().and_then(value_to_string).unwrap_or_default())
}

/// Deserializes an optional string, accepting numbers (`null` -> `None`).
pub fn opt_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(v.as_ref().and_then(value_to_string))
}

/// Deserializes a list of string ids, accepting numbers, `null`, or Heroic-style
/// objects (`{"id": ...}` / `{"appName": ...}`).
pub fn vec_string<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let Some(Value::Array(items)) = Option::<Value>::deserialize(d)? else {
        return Ok(Vec::new());
    };
    let out = items
        .iter()
        .filter_map(|item| {
            value_to_string(item).or_else(|| {
                let obj = item.as_object()?;
                obj.get("id").or_else(|| obj.get("appName")).and_then(value_to_string)
            })
        })
        .filter(|s| !s.is_empty())
        .collect();
    Ok(out)
}

/// Deserializes a bool, treating `null` as `false`.
pub fn bool_default<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(match v {
        Some(Value::Bool(b)) => b,
        Some(Value::Number(n)) => n.as_i64().is_some_and(|i| i != 0),
        Some(Value::String(s)) => s.eq_ignore_ascii_case("true") || s == "1",
        _ => false,
    })
}

/// Deserializes an optional bool, tolerating `null` and unexpected types.
pub fn opt_bool<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(match v {
        Some(Value::Bool(b)) => Some(b),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    #[derive(Debug, Default, Deserialize, PartialEq)]
    struct Sample {
        #[serde(default, deserialize_with = "super::string")]
        id: String,
        #[serde(default, deserialize_with = "super::opt_string")]
        size: Option<String>,
        #[serde(default, deserialize_with = "super::vec_string")]
        dlcs: Vec<String>,
        #[serde(default, deserialize_with = "super::bool_default")]
        pinned: bool,
    }

    #[test]
    fn accepts_numbers_nulls_and_objects() {
        let s: Sample =
            serde_json::from_str(r#"{"id": 44535815, "size": 1024, "dlcs": ["1", 2, {"id": 3}, {"appName": "4"}, null], "pinned": null}"#)
                .unwrap();
        assert_eq!(s.id, "44535815");
        assert_eq!(s.size.as_deref(), Some("1024"));
        assert_eq!(s.dlcs, ["1", "2", "3", "4"]);
        assert!(!s.pinned);

        let empty: Sample = serde_json::from_str(r#"{"id": null, "size": null, "dlcs": null}"#).unwrap();
        assert_eq!(empty, Sample::default());
    }
}
