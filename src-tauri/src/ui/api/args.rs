// Argument extraction for the native-UI command facade (ui/api.rs): the same rules `tauri::command` applies to the JS `invoke(cmd, {camelCase})` object.
//   - the JS key is the camelCase of the Rust parameter (`copy_only` <- `copyOnly`); the snake_case spelling is accepted too, for native callers
//   - a missing or `null` key is `None` for an `Option<T>` parameter and an error for any other type
//   - extra keys are ignored; `Value::Null` stands for "no arguments" (`invoke('cmd_toggle')`)
//   - a whole number that arrives as a float (`3.0`) is accepted where an integer is expected (native code computes with f64)
// Error texts are stable ("missing required key", "invalid args") because renderers show them. Argument VALUES are never put into an error
// (API keys and passwords travel through here), only the key name and serde's type complaint.
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// `copy_only` -> `copyOnly` (what Tauri does to a Rust parameter name to get the JS key).
pub fn to_camel(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut up = false;
    for c in snake.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// `copyOnly` -> `copy_only` (the inverse, for reading JS-shaped keys back into Rust names).
pub fn to_snake(camel: &str) -> String {
    let mut out = String::with_capacity(camel.len() + 4);
    for c in camel.chars() {
        if c.is_uppercase() {
            out.push('_');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A float with no fraction becomes the equal integer number (`3.0` -> `3`); anything else is None.
fn whole(v: &Value) -> Option<Value> {
    let f = v.as_f64().filter(|_| v.is_f64())?;
    if f.fract() != 0.0 {
        return None;
    }
    if f >= 0.0 && f <= u64::MAX as f64 {
        Some(Value::from(f as u64))
    } else if f < 0.0 && f >= i64::MIN as f64 {
        Some(Value::from(f as i64))
    } else {
        None
    }
}

/// What is wrong, without the value: serde's own text quotes it (`invalid type: string "sk-...", expected u64`) and these arguments can be keys.
fn describe<T>(v: &Value, e: &serde_json::Error) -> String {
    let m = e.to_string();
    if m.starts_with("missing field") || m.starts_with("unknown field") {
        return m.split(" at line").next().unwrap_or("").to_string(); // field names only
    }
    let kind = match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    let want = std::any::type_name::<T>();
    format!("expected {}, got {kind}", want.rsplit("::").next().unwrap_or(want))
}

fn decode<T: DeserializeOwned>(v: &Value) -> Result<T, String> {
    match serde_json::from_value::<T>(v.clone()) {
        Ok(t) => Ok(t),
        Err(e) => match whole(v).map(serde_json::from_value::<T>) {
            Some(Ok(t)) => Ok(t),
            _ => Err(describe::<T>(v, &e)),
        },
    }
}

/// The argument object of one call, with the command name for error texts.
pub struct Args<'a> {
    cmd: &'a str,
    map: Option<&'a Map<String, Value>>,
}

impl<'a> Args<'a> {
    /// `Null` = no arguments; an object = the arguments; anything else is an error (Tauri also wants an object).
    pub fn new(cmd: &'a str, v: &'a Value) -> Result<Args<'a>, String> {
        match v {
            Value::Null => Ok(Args { cmd, map: None }),
            Value::Object(m) => Ok(Args { cmd, map: Some(m) }),
            _ => Err(format!("command `{cmd}`: arguments must be an object")),
        }
    }

    /// The value under the camelCase key of `name` (a Rust parameter name), or under `name` itself.
    fn find(&self, name: &str) -> Option<&'a Value> {
        let m = self.map?;
        m.get(&to_camel(name)).or_else(|| m.get(name))
    }

    /// A required parameter: absent or of the wrong type is an error.
    pub fn req<T: DeserializeOwned>(&self, name: &str) -> Result<T, String> {
        let key = to_camel(name);
        match self.find(name) {
            None => Err(format!("command `{}` missing required key `{}`", self.cmd, key)),
            Some(v) => decode(v).map_err(|e| format!("invalid args `{}` for command `{}`: {}", key, self.cmd, e)),
        }
    }

    /// An `Option<T>` parameter: absent and `null` are None.
    pub fn opt<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>, String> {
        match self.find(name) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => decode(v)
                .map(Some)
                .map_err(|e| format!("invalid args `{}` for command `{}`: {}", to_camel(name), self.cmd, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn case_conversion_round_trips() {
        assert_eq!(to_camel("copy_only"), "copyOnly");
        assert_eq!(to_camel("json_text"), "jsonText");
        assert_eq!(to_camel("id"), "id");
        assert_eq!(to_camel("new_index"), "newIndex");
        assert_eq!(to_snake("copyOnly"), "copy_only");
        assert_eq!(to_snake("jsonText"), "json_text");
        assert_eq!(to_snake("id"), "id");
        for n in ["folder_id", "body_html", "remaining_ms"] {
            assert_eq!(to_snake(&to_camel(n)), n);
        }
    }

    #[test]
    fn required_and_optional_follow_the_tauri_rules() {
        let v = json!({ "copyOnly": true, "text": "x", "nothing": null, "limit": 5 });
        let a = Args::new("clip_paste", &v).unwrap();
        assert_eq!(a.req::<String>("text").unwrap(), "x");
        assert_eq!(a.opt::<bool>("copy_only").unwrap(), Some(true));
        assert_eq!(a.opt::<bool>("plain").unwrap(), None, "absent -> None");
        assert_eq!(a.opt::<String>("nothing").unwrap(), None, "null -> None");
        assert_eq!(a.opt::<i64>("limit").unwrap(), Some(5));
    }

    #[test]
    fn missing_null_and_wrong_type_give_clear_errors() {
        let v = json!({ "id": "not a number", "nothing": null });
        let a = Args::new("clip_pin", &v).unwrap();
        assert_eq!(a.req::<bool>("pinned").unwrap_err(), "command `clip_pin` missing required key `pinned`");
        assert_eq!(a.req::<i64>("id").unwrap_err(), "invalid args `id` for command `clip_pin`: expected i64, got string");
        assert!(a.req::<String>("nothing").unwrap_err().starts_with("invalid args `nothing` for command `clip_pin`"), "null is not a String");
        assert!(a.opt::<i64>("id").unwrap_err().starts_with("invalid args `id`"));
        let a = Args::new("x", &Value::Null).unwrap();
        assert!(a.req::<i64>("n").is_err());
        assert_eq!(a.opt::<i64>("n").unwrap(), None);
        assert!(Args::new("x", &json!([1])).is_err());
        assert!(Args::new("x", &json!("s")).is_err());
    }

    #[test]
    fn camel_key_wins_and_snake_key_is_accepted() {
        let v = json!({ "new_index": 1, "newIndex": 2 });
        assert_eq!(Args::new("c", &v).unwrap().req::<usize>("new_index").unwrap(), 2);
        let v = json!({ "new_index": 7 });
        assert_eq!(Args::new("c", &v).unwrap().req::<usize>("new_index").unwrap(), 7);
    }

    #[test]
    fn whole_floats_pass_for_integers_but_fractions_do_not() {
        let v = json!({ "ms": 1500.0, "neg": -3.0, "frac": 1.5, "f": 2.5 });
        let a = Args::new("media_seek", &v).unwrap();
        assert_eq!(a.req::<u64>("ms").unwrap(), 1500);
        assert_eq!(a.req::<i64>("neg").unwrap(), -3);
        assert!(a.req::<u64>("frac").is_err());
        assert_eq!(a.req::<f64>("f").unwrap(), 2.5);
        assert!(a.req::<u64>("neg").is_err(), "a negative number is not a u64");
    }

    #[test]
    fn values_never_appear_in_errors() {
        let v = json!({ "key": 12345, "k2": "sk-secret" });
        let a = Args::new("ai_set_key", &v).unwrap();
        let e = a.req::<String>("key").unwrap_err();
        assert!(!e.contains("12345"), "{e}");
        let e = a.req::<u64>("k2").unwrap_err();
        assert!(!e.contains("sk-secret"), "{e}");
    }

    #[test]
    fn value_and_structured_parameters() {
        let v = json!({ "value": { "a": 1 }, "paths": ["a", "b"], "ids": null });
        let a = Args::new("c", &v).unwrap();
        assert_eq!(a.req::<Value>("value").unwrap(), json!({ "a": 1 }));
        assert_eq!(a.req::<Vec<String>>("paths").unwrap(), ["a", "b"]);
        assert_eq!(a.opt::<Vec<i64>>("ids").unwrap(), None);
    }
}
