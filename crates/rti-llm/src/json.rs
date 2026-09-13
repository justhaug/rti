/// Extract the first JSON object or array from free-form model output
/// (handles ```json fences and leading prose).
pub fn extract_json(text: &str) -> Option<serde_json::Value> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text.trim()) {
        return Some(v);
    }
    // fenced block
    if let Some(start) = text.find("```") {
        let rest = &text[start + 3..];
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        if let Some(end) = rest.find("```") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(rest[..end].trim()) {
                return Some(v);
            }
        }
    }
    // first balanced {...} or [...]
    for (open, close) in [('{', '}'), ('[', ']')] {
        if let Some(s) = text.find(open) {
            let bytes = text.as_bytes();
            let mut depth = 0i32;
            let mut in_str = false;
            let mut esc = false;
            for (i, &b) in bytes.iter().enumerate().skip(s) {
                let c = b as char;
                if in_str {
                    if esc {
                        esc = false;
                    } else if c == '\\' {
                        esc = true;
                    } else if c == '"' {
                        in_str = false;
                    }
                    continue;
                }
                match c {
                    '"' => in_str = true,
                    c if c == open => depth += 1,
                    c if c == close => {
                        depth -= 1;
                        if depth == 0 {
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text[s..=i]) {
                                return Some(v);
                            }
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_from_prose_and_fences() {
        assert_eq!(extract_json("{\"a\":1}").unwrap()["a"], 1);
        assert_eq!(
            extract_json("Sure! ```json\n{\"a\": [1,2]}\n``` done").unwrap()["a"][1],
            2
        );
        assert_eq!(
            extract_json("text {\"s\":\"x}y\",\"n\":{\"k\":true}} tail").unwrap()["n"]["k"],
            true
        );
        assert!(extract_json("no json here").is_none());
    }
}
