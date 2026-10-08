//! Name patterns: comma-separated regular expressions matched against joint
//! and other names. Case is ignored and a pattern matches anywhere in the
//! name, so a plain word still means "contains this word".

use regex::{Regex, RegexBuilder};

/// Split on commas that are not inside (), [] or {}, so `a{1,2}` stays whole.
pub fn parts(text: &str) -> Vec<&str> {
    let mut out = vec![];
    let (mut depth, mut start, mut escaped) = (0i32, 0usize, false);
    for (i, c) in text.char_indices() {
        if escaped { escaped = false; continue; }
        match c {
            '\\' => escaped = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            ',' if depth == 0 => { out.push(&text[start..i]); start = i + 1; }
            _ => {}
        }
    }
    out.push(&text[start..]);
    out.into_iter().map(str::trim).filter(|p| !p.is_empty()).collect()
}

fn build(part: &str) -> Result<Regex, regex::Error> {
    RegexBuilder::new(part).case_insensitive(true).build()
}

/// One regular expression. Text that is not a valid expression is matched
/// literally.
pub fn compile(part: &str) -> Regex {
    build(part).unwrap_or_else(|_| build(&regex::escape(part)).expect("escaped text is a valid expression"))
}

#[derive(Clone, Debug)]
pub struct NamePattern(Vec<Regex>);

impl NamePattern {
    pub fn new(text: &str) -> Self { NamePattern(parts(text).into_iter().map(compile).collect()) }
    pub fn is_empty(&self) -> bool { self.0.is_empty() }
    pub fn matches(&self, name: &str) -> bool { self.0.iter().any(|r| r.is_match(name)) }
}

/// The first part of `text` that is not a valid expression, with the reason.
pub fn error(text: &str) -> Option<String> {
    parts(text).into_iter().find_map(|p| build(p).err().map(|e| {
        let why = e.to_string();
        format!("\"{p}\" is matched as plain text: {}", why.lines().last().unwrap_or("").trim())
    }))
}

/// The part that matches exactly `name` and nothing else.
pub fn exact(name: &str) -> String { format!("^{}$", regex::escape(name)) }

/// True when `text` holds the exact part for `name`.
pub fn has_exact(text: &str, name: &str) -> bool {
    let e = exact(name);
    parts(text).iter().any(|p| *p == e)
}

/// Add the exact part for `name`, or remove it when it is there.
pub fn toggle(text: &mut String, name: &str) {
    let e = exact(name);
    let mut list: Vec<String> = parts(text).into_iter().map(String::from).collect();
    match list.iter().position(|p| *p == e) {
        Some(i) => { list.remove(i); }
        None    => list.push(e),
    }
    *text = list.join(", ");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_words_match_anywhere_ignoring_case() {
        let p = NamePattern::new("finger, Toe");
        assert!(p.matches("LeftHandFinger1") && p.matches("righttoebase"));
        assert!(!p.matches("Hips"));
    }

    #[test]
    fn regular_expressions() {
        let p = NamePattern::new("^Left(Arm|Leg)$, Finger[1-3]{1,2}$");
        assert!(p.matches("LeftArm") && p.matches("LeftLeg") && p.matches("RFinger12"));
        assert!(!p.matches("LeftArmRoll") && !p.matches("RFinger4"));
        assert_eq!(parts("a{1,2}, (b,c), d\\,e"), vec!["a{1,2}", "(b,c)", "d\\,e"]);
    }

    #[test]
    fn invalid_expression_is_literal() {
        let p = NamePattern::new("Arm(");
        assert!(p.matches("LeftArm(1)") && !p.matches("LeftArm"));
        assert!(error("Arm(").is_some() && error("Arm").is_none());
    }

    #[test]
    fn empty_matches_nothing() {
        assert!(NamePattern::new(" , ").is_empty());
        assert!(!NamePattern::new("").matches("Hips"));
    }

    #[test]
    fn toggle_adds_and_removes_exact_names() {
        let mut t = String::from("finger");
        toggle(&mut t, "Take01:Head.001");
        assert!(has_exact(&t, "Take01:Head.001"));
        let p = NamePattern::new(&t);
        assert!(p.matches("Take01:Head.001") && !p.matches("Take01:Head_001") && !p.matches("xTake01:Head.001"));
        toggle(&mut t, "Take01:Head.001");
        assert_eq!(t, "finger");
    }
}
