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
}
