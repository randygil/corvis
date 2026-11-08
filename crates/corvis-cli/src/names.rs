//! Name conversions (`cats`, `user-profile`, `UserProfile` ...).

pub struct Names {
    /// `user_profile`
    pub snake: String,
    /// `UserProfile`
    pub pascal: String,
    /// `user-profile`
    pub kebab: String,
}

fn words(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in input.chars() {
        if c == '-' || c == '_' || c == ' ' || c == '.' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// A project or element name: `^[A-Za-z][A-Za-z0-9_-]*$`. Names become file names, Rust
/// identifiers and text inside the generated code, so separators (`/`, `\`, `:`), quotes and
/// other characters are rejected instead of being copied into paths or sources.
pub fn validate_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(format!("invalid name \"{}\": use letters, digits, '-' and '_', starting with a letter", name.escape_debug()))
    }
}

/// `--path <dir>`: the feature folder under `src/`, also used as a Rust module name
/// (`mod <dir>;`, `crate::<dir>::...`), so it must be a plain identifier (`^[A-Za-z_][A-Za-z0-9_]*$`).
pub fn validate_module_dir(dir: &str) -> Result<(), String> {
    let mut chars = dir.chars();
    let ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && dir != "_";
    if ok { Ok(()) } else { Err(format!("invalid --path \"{}\": use a module name such as users or user_profile", dir.escape_debug())) }
}

/// `--corvis-version <v>`: a Cargo version requirement (`0.1`, `^0.1.2`, `>=0.1, <0.3`,
/// `=1.0.0-beta.1`). It is written inside a TOML string, so quotes, backslashes and line
/// breaks are rejected.
pub fn validate_version_req(v: &str) -> Result<(), String> {
    let ok = !v.trim().is_empty()
        && v.chars().any(|c| c.is_ascii_digit() || c == '*')
        && v.chars().all(|c| c.is_ascii_alphanumeric() || " .,^~=<>*+-".contains(c));
    if ok { Ok(()) } else { Err(format!("invalid --corvis-version \"{}\"", v.escape_debug())) }
}

impl Names {
    pub fn new(input: &str) -> Names {
        let w = words(input);
        let pascal = w
            .iter()
            .map(|s| {
                let mut c = s.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    None => String::new(),
                }
            })
            .collect();
        Names { snake: w.join("_"), pascal, kebab: w.join("-") }
    }

    /// Singular form, naive (`cats` -> `cat`, `categories` -> `category`).
    pub fn singular(&self) -> Names {
        let s = &self.snake;
        let single = if let Some(stem) = s.strip_suffix("ies") {
            format!("{stem}y")
        } else if s.ends_with("ses") || s.ends_with("xes") {
            s[..s.len() - 2].to_string()
        } else if let Some(stem) = s.strip_suffix('s') {
            stem.to_string()
        } else {
            s.clone()
        };
        Names::new(&single)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        let n = Names::new("user-profile");
        assert_eq!(n.snake, "user_profile");
        assert_eq!(n.pascal, "UserProfile");
        assert_eq!(n.kebab, "user-profile");
        assert_eq!(Names::new("UserProfile").snake, "user_profile");
        assert_eq!(Names::new("cats").singular().pascal, "Cat");
        assert_eq!(Names::new("categories").singular().snake, "category");
    }

    #[test]
    fn name_validation() {
        for ok in ["cats", "user-profile", "UserProfile", "user_profile2", "a"] {
            assert!(validate_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "1cats", "-x", "../x", "a/b", r"a\b", "C:", "a\"b", "a b", "a.b", "a\nb", "gató"] {
            assert!(validate_name(bad).is_err(), "{bad}");
        }
        for ok in ["users", "user_profile", "_x", "v2"] {
            assert!(validate_module_dir(ok).is_ok(), "{ok}");
        }
        for bad in ["", "_", "../src", "a/b", "/abs", r"C:\x", "a-b", "1a", "a;b"] {
            assert!(validate_module_dir(bad).is_err(), "{bad}");
        }
        for ok in ["0.1", "^0.1.2", ">=0.1, <0.3", "=1.0.0-beta.1", "*", "~1"] {
            assert!(validate_version_req(ok).is_ok(), "{ok}");
        }
        for bad in ["", " ", "1\"\nevil = \"x", r"1\x", "latest", "1;2"] {
            assert!(validate_version_req(bad).is_err(), "{bad}");
        }
    }
}
