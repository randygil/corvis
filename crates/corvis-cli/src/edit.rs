//! Small source editors used to register generated code: they add the new module, provider or
//! controller to the parent module.

/// Adds `mod name;` (or `pub mod name;`) to a file if missing.
pub fn add_mod(source: &str, decl: &str) -> String {
    if source.lines().any(|l| l.trim() == decl) {
        return source.to_string();
    }
    let lines: Vec<&str> = source.lines().collect();
    let last_mod = lines.iter().rposition(|l| {
        let t = l.trim_start();
        (t.starts_with("mod ") || t.starts_with("pub mod ")) && t.ends_with(';')
    });
    let mut out: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    match last_mod {
        Some(i) => out.insert(i + 1, decl.to_string()),
        None => {
            // after leading comments / attributes
            let i = lines.iter().position(|l| !l.trim_start().starts_with("//") && !l.trim_start().starts_with("#!")).unwrap_or(0);
            out.insert(i, decl.to_string());
            if out.get(i + 1).map(|l| !l.trim().is_empty()).unwrap_or(false) {
                out.insert(i + 1, String::new());
            }
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Adds a `use` line after the last top-level `use` (or at the top).
pub fn add_use(source: &str, line: &str) -> String {
    if source.lines().any(|l| l.trim() == line) {
        return source.to_string();
    }
    let lines: Vec<&str> = source.lines().collect();
    let mut out: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    match lines.iter().rposition(|l| l.starts_with("use ")) {
        Some(i) => out.insert(i + 1, line.to_string()),
        None => out.insert(0, line.to_string()),
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Inserts `item` into `key = [...]` of the first `#[module(...)]` attribute.
pub fn add_to_module(source: &str, key: &str, item: &str) -> Option<String> {
    let start = source.find("#[module")?;
    let after = start + "#[module".len();
    let rest = &source[after..];
    if !rest.starts_with('(') {
        // `#[module]` without arguments.
        let end = after + rest.find(']')?;
        return Some(format!("{}#[module({key} = [{item}])]{}", &source[..start], &source[end + 1..]));
    }
    // Find the matching `)` of `#[module(`.
    let open = after;
    let mut depth = 0i32;
    let mut close = None;
    for (i, c) in source[open..].char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let args = &source[open + 1..close];
    if args.contains(item) && args.contains(key) {
        // naive duplicate check inside the key list
        if let Some(k) = find_key(args, key) {
            let list = &args[k..];
            if let Some(end) = list.find(']')
                && list[..end].split([',', '[', ' ', '\n']).any(|s| s.trim() == item) {
                    return Some(source.to_string());
                }
        }
    }
    match find_key(args, key) {
        Some(k) => {
            let abs = open + 1 + k;
            let bracket = abs + source[abs..].find('[')?;
            let mut depth = 0i32;
            let mut end = None;
            for (i, c) in source[bracket..].char_indices() {
                match c {
                    '[' | '(' | '{' => depth += 1,
                    ']' | ')' | '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(bracket + i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let end = end?;
            let inner = source[bracket + 1..end].trim_end();
            let multiline = inner.contains('\n');
            let insertion = if inner.trim().is_empty() {
                item.to_string()
            } else if multiline {
                let trailing_comma = inner.ends_with(',');
                let indent = inner.lines().last().map(|l| l.len() - l.trim_start().len()).unwrap_or(8);
                format!("{}{}\n{}{item},", if trailing_comma { "" } else { "," }, "", " ".repeat(indent))
            } else {
                let sep = if inner.ends_with(',') { " " } else { ", " };
                format!("{sep}{item}")
            };
            let insert_at = bracket + 1 + inner.len();
            Some(format!("{}{}{}", &source[..insert_at], insertion, &source[insert_at..]))
        }
        None => {
            let trimmed = args.trim_end();
            let sep = if trimmed.trim().is_empty() {
                ""
            } else if trimmed.ends_with(',') {
                " "
            } else {
                ", "
            };
            let insert_at = open + 1 + trimmed.len();
            Some(format!("{}{sep}{key} = [{item}]{}", &source[..insert_at], &source[insert_at..]))
        }
    }
}

fn find_key(args: &str, key: &str) -> Option<usize> {
    let mut depth = 0i32;
    let bytes = args.as_bytes();
    for (i, c) in args.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && args[i..].starts_with(key) {
            let before_ok = i == 0 || !(bytes[i - 1] as char).is_alphanumeric() && bytes[i - 1] != b'_';
            let after = args[i + key.len()..].trim_start();
            if before_ok && after.starts_with('=') {
                return Some(i);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_editing() {
        let src = "#[module(controllers = [AppController], providers = [AppService])]\npub struct AppModule;";
        let out = add_to_module(src, "imports", "CatsModule").unwrap();
        assert!(out.contains("providers = [AppService], imports = [CatsModule])]"), "{out}");
        let out = add_to_module(&out, "imports", "DogsModule").unwrap();
        assert!(out.contains("imports = [CatsModule, DogsModule]"), "{out}");
        let out2 = add_to_module(&out, "imports", "DogsModule").unwrap();
        assert_eq!(out, out2);
        let empty = add_to_module("#[module]\npub struct M;", "providers", "S").unwrap();
        assert!(empty.contains("#[module(providers = [S])]"));
        let multi = "#[module(\n    imports = [\n        A,\n    ],\n)]\nstruct M;";
        let out = add_to_module(multi, "imports", "B").unwrap();
        assert!(out.contains("        A,\n        B,"), "{out}");
    }

    #[test]
    fn mods_and_uses() {
        let src = "mod a;\n\nfn main() {}\n";
        assert_eq!(add_mod(src, "mod b;"), "mod a;\nmod b;\n\nfn main() {}\n");
        let src = "use x::y;\n\nstruct A;\n";
        assert_eq!(add_use(src, "use z::w;"), "use x::y;\nuse z::w;\n\nstruct A;\n");
    }
}
