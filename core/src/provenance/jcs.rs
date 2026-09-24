//! JSON Canonicalization Scheme (RFC 8785).
//!
//! Every event is hashed over its canonical form, so the same event always has
//! the same hash no matter which program wrote it: object keys sorted by UTF-16
//! code units, numbers formatted as ECMAScript does, minimal string escaping.

use serde_json::Value;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum JcsError {
    #[error("number {0} is not representable as an IEEE-754 double without loss")]
    UnsafeInteger(String),
    #[error("non-finite number")]
    NonFinite,
}

pub fn canonicalize(v: &Value) -> Result<String, JcsError> {
    let mut out = String::new();
    write_value(v, &mut out)?;
    Ok(out)
}

/// Canonical bytes, for hashing.
pub fn canonical_bytes(v: &Value) -> Result<Vec<u8>, JcsError> {
    canonicalize(v).map(String::into_bytes)
}

fn write_value(v: &Value, out: &mut String) -> Result<(), JcsError> {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => write_number(n, out)?,
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(k, out);
                out.push(':');
                write_value(&map[*k], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn write_number(n: &serde_json::Number, out: &mut String) -> Result<(), JcsError> {
    const MAX_SAFE: u64 = 1 << 53;
    let f = if let Some(u) = n.as_u64() {
        if u > MAX_SAFE {
            return Err(JcsError::UnsafeInteger(u.to_string()));
        }
        u as f64
    } else if let Some(i) = n.as_i64() {
        if i.unsigned_abs() > MAX_SAFE {
            return Err(JcsError::UnsafeInteger(i.to_string()));
        }
        i as f64
    } else {
        n.as_f64().ok_or(JcsError::NonFinite)?
    };
    if !f.is_finite() {
        return Err(JcsError::NonFinite);
    }
    if f == 0.0 {
        out.push('0'); // ECMAScript prints −0 as "0"
        return Ok(());
    }
    let mut buf = ryu_js::Buffer::new();
    out.push_str(buf.format_finite(f));
    Ok(())
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{09}' => out.push_str("\\t"),
            '\u{0A}' => out.push_str("\\n"),
            '\u{0C}' => out.push_str("\\f"),
            '\u{0D}' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn num(s: &str) -> String {
        let v: Value = serde_json::from_str(s).unwrap();
        canonicalize(&v).unwrap()
    }

    /// Number examples from RFC 8785 appendix B and section 3.2.2.3.
    #[test]
    fn numbers_follow_ecmascript() {
        assert_eq!(num("333333333.33333329"), "333333333.3333333");
        assert_eq!(num("1E30"), "1e+30");
        assert_eq!(num("4.50"), "4.5");
        assert_eq!(num("2e-3"), "0.002");
        assert_eq!(num("0.000000000000000000000000001"), "1e-27");
        assert_eq!(num("-0.0"), "0");
        assert_eq!(num("100"), "100");
        assert_eq!(num("3.0"), "3");
        assert_eq!(num("1e21"), "1e+21");
        assert_eq!(num("123456789012345680000"), "123456789012345680000");
        assert_eq!(num("-1.5e-7"), "-1.5e-7");
    }

    #[test]
    fn keys_sort_by_utf16_and_strings_escape_minimally() {
        let v = json!({"b": 1, "a": [true, null, "x\ny\"z\u{1}"], "\u{20ac}": 2, "\u{1d11e}": 3, "A": {}});
        assert_eq!(
            canonicalize(&v).unwrap(),
            "{\"A\":{},\"a\":[true,null,\"x\\ny\\\"z\\u0001\"],\"b\":1,\"\u{20ac}\":2,\"\u{1d11e}\":3}"
        );
    }

    /// RFC 8785 section 3.2.3 sorting example.
    #[test]
    fn rfc_sorting_example() {
        let v: Value = serde_json::from_str(
            r#"{"€":"Euro Sign","\r":"Carriage Return","דּ":"Hebrew Letter Dalet With Dagesh","1":"One","😀":"Emoji: Grinning Face","\u0080":"Control","ö":"Latin Small Letter O With Diaeresis"}"#,
        )
        .unwrap();
        let c = canonicalize(&v).unwrap();
        let order: Vec<&str> = [
            "Carriage Return",
            "One",
            "Control",
            "Latin Small",
            "Euro Sign",
            "Emoji",
            "Hebrew",
        ]
        .to_vec();
        let mut last = 0;
        for o in order {
            let at = c.find(o).unwrap();
            assert!(at > last, "{o} out of order in {c}");
            last = at;
        }
    }

    #[test]
    fn rejects_unsafe_integers() {
        assert!(canonicalize(&json!(9007199254740993u64)).is_err());
    }
}
