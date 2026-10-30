//! NWL style sheets: CSS-like text files (`MaschineMK3Styles.txt` and what it
//! `@import`s). `@define $name value;` defines text that `$name` stands for later;
//! rules are `selector, selector { property: value; ... }`.
//!
//! A selector is a chain of compounds, each an ancestor of the next (whitespace or
//! `::`): `Type#name.class[Attr=value]:state:!state`. `Type` may be `Vector<Bar>`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    /// RGBA.
    Color([u8; 4]),
    /// `no`: nothing (no background, no image).
    None,
    Url(String),
    Bool(bool),
    Ident(String),
}

impl Value {
    pub fn num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Compound {
    pub kind: Option<String>,
    pub name: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<(String, String)>,
    /// `:selected` (true) or `:!selected` (false).
    pub states: Vec<(String, bool)>,
    /// Joined to the compound before by `::`: a child of it, not any descendant.
    pub child: bool,
}

#[derive(Debug, Clone)]
pub struct Rule {
    /// Ancestors first; the last compound is the element styled.
    pub chain: Vec<Compound>,
    pub decls: Vec<(String, Value)>,
    /// (names, classes + attributes + states, types), as in CSS.
    pub specificity: (u32, u32, u32),
}

/// What a selector compound is matched against.
pub struct Node<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    /// Style classes (`style("A B")`).
    pub classes: &'a str,
    pub attrs: &'a [(String, String)],
    pub states: &'a [(&'static str, bool)],
}

#[derive(Debug, Default)]
pub struct Sheet {
    pub rules: Vec<Rule>,
    defines: HashMap<String, String>,
    /// `paletteN-colorI` declarations: (N, I) → colour.
    palettes: HashMap<(u32, u32), [u8; 4]>,
    /// Rules by the type, name or first class of their last compound (`*` for none).
    index: HashMap<String, Vec<usize>>,
}

impl Sheet {
    /// Load `entry` (a path relative to one of `roots`) and everything it imports.
    pub fn load(roots: &[PathBuf], entry: &str) -> Result<Sheet, String> {
        let mut s = Sheet::default();
        s.import(roots, entry, 0)?;
        s.reindex();
        Ok(s)
    }

    /// Parse sheet text directly (imports are not followed).
    pub fn parse(text: &str) -> Sheet {
        let mut s = Sheet::default();
        s.add(&[], text, 0).ok();
        s.reindex();
        s
    }

    fn import(&mut self, roots: &[PathBuf], path: &str, depth: usize) -> Result<(), String> {
        if depth > 32 {
            return Err(format!("{path}: imports nest too deep"));
        }
        let file = find(roots, path).ok_or_else(|| format!("style sheet not found: {path}"))?;
        let text = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        self.add(roots, &String::from_utf8_lossy(&text), depth)
    }

    fn add(&mut self, roots: &[PathBuf], text: &str, depth: usize) -> Result<(), String> {
        let text = strip_comments(text);
        let mut rest = text.as_str();
        loop {
            rest = rest.trim_start();
            if rest.is_empty() {
                return Ok(());
            }
            if let Some(d) = rest.strip_prefix('@') {
                let end = d.find(';').unwrap_or(d.len());
                let directive = d[..end].trim();
                rest = d.get(end + 1..).unwrap_or("");
                if let Some(def) = directive.strip_prefix("define") {
                    let def = def.trim();
                    let (name, value) = def.split_once(char::is_whitespace).unwrap_or((def, ""));
                    let value = self.substitute(value.trim());
                    self.defines.insert(name.trim_start_matches('$').to_string(), value);
                } else if let Some(imp) = directive.strip_prefix("import") {
                    if let Some(Value::Url(path)) = parse_value(imp.trim()) {
                        // A missing import is skipped (sheets name a few files that aren't shipped).
                        self.import(roots, &path, depth + 1).ok();
                    }
                }
                continue;
            }
            let Some(open) = rest.find('{') else { return Ok(()) };
            let selectors = &rest[..open];
            let after = &rest[open + 1..];
            let close = after.find('}').unwrap_or(after.len());
            let body = &after[..close];
            rest = after.get(close + 1..).unwrap_or("");
            let mut decls = Vec::new();
            for d in body.split(';') {
                let Some((k, v)) = d.split_once(':') else { continue };
                let (k, v) = (k.trim(), self.substitute(v.trim()));
                if k.is_empty() {
                    continue;
                }
                let Some(v) = parse_value(&v) else { continue };
                if let (Some((p, c)), Value::Color(rgba)) = (palette_key(k), &v) {
                    self.palettes.insert((p, c), *rgba);
                }
                decls.push((k.to_string(), v));
            }
            for sel in selectors.split(',') {
                let sel = sel.trim();
                if sel.is_empty() {
                    continue;
                }
                let chain = parse_selector(sel);
                if chain.is_empty() {
                    continue;
                }
                let specificity = chain.iter().fold((0, 0, 0), |(a, b, c), p| {
                    (
                        a + p.name.is_some() as u32,
                        b + (p.classes.len() + p.attrs.len() + p.states.len()) as u32,
                        c + p.kind.is_some() as u32,
                    )
                });
                self.rules.push(Rule { chain, decls: decls.clone(), specificity });
            }
        }
    }

    /// Replace `$name`s with their definitions.
    fn substitute(&self, v: &str) -> String {
        if !v.contains('$') {
            return v.to_string();
        }
        let mut out = String::new();
        let mut chars = v.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c != '$' {
                out.push(c);
                continue;
            }
            let start = i + 1;
            let mut end = start;
            while let Some(&(j, c)) = chars.peek() {
                if c.is_alphanumeric() || c == '_' || c == '-' {
                    end = j + c.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            match self.defines.get(&v[start..end]) {
                Some(d) => out.push_str(d),
                None => out.push_str(&v[i..end]),
            }
        }
        out
    }

    fn reindex(&mut self) {
        self.index.clear();
        for (i, r) in self.rules.iter().enumerate() {
            self.index.entry(index_key(r.chain.last().unwrap())).or_default().push(i);
        }
    }

    /// The declarations that apply to the last node of `path` (root first), in the
    /// order they apply: least specific first, later rules after earlier ones.
    pub fn matching(&self, path: &[Node]) -> Vec<&Rule> {
        let Some(node) = path.last() else { return Vec::new() };
        let mut keys = vec!["*".to_string(), format!("t:{}", node.kind)];
        if !node.name.is_empty() {
            keys.push(format!("n:{}", node.name));
        }
        keys.extend(node.classes.split_whitespace().map(|c| format!("c:{c}")));
        let mut found: Vec<usize> =
            keys.iter().filter_map(|k| self.index.get(k)).flatten().copied().filter(|&i| matches(&self.rules[i].chain, path)).collect();
        found.sort_unstable();
        found.dedup();
        found.sort_by_key(|&i| (self.rules[i].specificity, i));
        found.into_iter().map(|i| &self.rules[i]).collect()
    }

    /// Colour `index` of palette `palette` (`paletteN-colorI`).
    pub fn palette(&self, palette: u32, index: u32) -> Option<[u8; 4]> {
        self.palettes.get(&(palette, index)).copied()
    }

    pub fn define(&self, name: &str) -> Option<&str> {
        self.defines.get(name).map(String::as_str)
    }
}

fn index_key(c: &Compound) -> String {
    if let Some(n) = &c.name {
        format!("n:{n}")
    } else if let Some(cl) = c.classes.first() {
        format!("c:{cl}")
    } else if let Some(k) = &c.kind {
        format!("t:{k}")
    } else {
        "*".into()
    }
}

fn find(roots: &[PathBuf], path: &str) -> Option<PathBuf> {
    roots.iter().map(|r| r.join(path)).find(|p| p.is_file()).or_else(|| Some(PathBuf::from(path)).filter(|p| Path::new(p).is_file()))
}

fn palette_key(k: &str) -> Option<(u32, u32)> {
    let (p, c) = k.strip_prefix("palette")?.split_once("-color")?;
    Some((p.parse().ok()?, c.parse().ok()?))
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let b = text.as_bytes();
    let mut i = 0;
    let mut quoted = false;
    while i < b.len() {
        if b[i] == b'"' {
            quoted = !quoted;
        } else if !quoted && b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        } else if !quoted && b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        } else if b[i] == b'\n' {
            quoted = false;
        }
        let ch_len = text[i..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    out
}

pub fn parse_value(v: &str) -> Option<Value> {
    let v = v.trim();
    if v.is_empty() {
        return None;
    }
    if v == "no" || v == "none" {
        return Some(Value::None);
    }
    if v == "true" || v == "false" {
        return Some(Value::Bool(v == "true"));
    }
    if let Some(u) = v.strip_prefix("url(") {
        return Some(Value::Url(u.trim_end_matches(')').trim().trim_matches('"').trim_matches('\'').to_string()));
    }
    for (prefix, n) in [("rgba(", 4), ("rgb(", 3)] {
        if let Some(args) = v.strip_prefix(prefix) {
            let nums: Vec<u8> = args
                .trim_end_matches(')')
                .split(',')
                .filter_map(|a| a.trim().parse::<f64>().ok())
                .map(|x| x.clamp(0.0, 255.0) as u8)
                .collect();
            if nums.len() < n {
                return None;
            }
            return Some(Value::Color([nums[0], nums[1], nums[2], if n == 4 { nums[3] } else { 255 }]));
        }
    }
    if let Ok(n) = v.parse::<f64>() {
        return Some(Value::Num(n));
    }
    Some(Value::Ident(v.to_string()))
}

pub fn parse_selector(sel: &str) -> Vec<Compound> {
    let mut out = Vec::new();
    for word in sel.split_whitespace() {
        for (i, part) in word.split("::").enumerate() {
            if part.is_empty() {
                continue;
            }
            let mut c = parse_compound(part);
            c.child = i > 0;
            out.push(c);
        }
    }
    out
}

fn parse_compound(s: &str) -> Compound {
    let mut c = Compound::default();
    let b = s.as_bytes();
    // The type runs to the first `#`, `.`, `[` or `:` outside `<...>`.
    let mut depth = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'<' => depth += 1,
            b'>' => depth -= 1,
            b'#' | b'.' | b'[' | b':' if depth == 0 => break,
            _ => {}
        }
        i += 1;
    }
    if i > 0 && &s[..i] != "*" {
        c.kind = Some(s[..i].to_string());
    }
    let ident_end = |from: usize| s[from..].find(['#', '.', '[', ':']).map_or(s.len(), |e| from + e);
    while i < b.len() {
        match b[i] {
            b'#' => {
                let e = ident_end(i + 1);
                c.name = Some(s[i + 1..e].to_string());
                i = e;
            }
            b'.' => {
                let e = ident_end(i + 1);
                c.classes.push(s[i + 1..e].to_string());
                i = e;
            }
            b'[' => {
                let e = s[i..].find(']').map_or(s.len(), |e| i + e);
                let inner = &s[i + 1..e];
                let (k, v) = inner.split_once('=').unwrap_or((inner, ""));
                c.attrs.push((k.trim().to_string(), v.trim().trim_matches('"').to_string()));
                i = e + 1;
            }
            b':' => {
                let (on, from) = if b.get(i + 1) == Some(&b'!') { (false, i + 2) } else { (true, i + 1) };
                let e = ident_end(from);
                c.states.push((s[from..e].to_string(), on));
                i = e;
            }
            _ => i += 1,
        }
    }
    c
}

fn compound_matches(c: &Compound, n: &Node) -> bool {
    if c.kind.as_deref().is_some_and(|k| k != n.kind) {
        return false;
    }
    if c.name.as_deref().is_some_and(|k| k != n.name) {
        return false;
    }
    if !c.classes.iter().all(|cl| n.classes.split_whitespace().any(|x| x == cl)) {
        return false;
    }
    if !c.attrs.iter().all(|(k, v)| n.attrs.iter().any(|(a, b)| a == k && b == v)) {
        return false;
    }
    c.states.iter().all(|(s, on)| n.states.iter().find(|(k, _)| k == s).map_or(!on, |(_, v)| v == on))
}

fn matches(chain: &[Compound], path: &[Node]) -> bool {
    let (Some(last), Some(node)) = (chain.last(), path.last()) else { return false };
    if !compound_matches(last, node) {
        return false;
    }
    // The rest, innermost first: each the parent of the one after it (`::`) or
    // somewhere further up. Tried every way a descendant step can match.
    fn up_from(chain: &[Compound], path: &[Node], at: usize, child: bool) -> bool {
        let Some((c, rest)) = chain.split_last() else { return true };
        let mut up = at;
        while up > 0 {
            up -= 1;
            if compound_matches(c, &path[up]) && up_from(rest, path, up, c.child) {
                return true;
            }
            if child {
                return false;
            }
        }
        false
    }
    up_from(&chain[..chain.len() - 1], path, path.len() - 1, last.child)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node<'a>(kind: &'a str, name: &'a str, classes: &'a str, attrs: &'a [(String, String)]) -> Node<'a> {
        Node { kind, name, classes, attrs, states: &[("selected", false), ("enabled", true)] }
    }

    #[test]
    fn defines_rules_and_specificity() {
        let s = Sheet::parse(
            r#"
            @define $w 480; // half
            @define $white rgb(255, 255, 255);
            Bar { width: 1; }
            Bar.Page { width: $w; background-color: $white; image: url("A/b.png"); }
            Stack#S Bar#C[HasFocus=true] Label#Icon:!selected { animation-index: 1; }
            /* gone */ Label#root { palette2-color3: rgba(1,2,3,4); }
            "#,
        );
        let a = Vec::new();
        let page = s.matching(&[node("Bar", "", "Page", &a)]);
        assert_eq!(page.len(), 2);
        assert_eq!(page[1].decls[0], ("width".into(), Value::Num(480.0)));
        assert_eq!(page[1].decls[1].1, Value::Color([255, 255, 255, 255]));
        assert_eq!(page[1].decls[2].1, Value::Url("A/b.png".into()));
        let focus = vec![("HasFocus".to_string(), "true".to_string())];
        let path = [node("Stack", "S", "", &a), node("Bar", "X", "", &a), node("Bar", "C", "", &focus), node("Label", "Icon", "", &a)];
        assert_eq!(s.matching(&path).len(), 1);
        let path = [node("Bar", "C", "", &focus), node("Label", "Icon", "", &a)];
        assert!(s.matching(&path).is_empty(), "needs Stack#S above");
        assert_eq!(s.palette(2, 3), Some([1, 2, 3, 4]));
        let s = Sheet::parse("Bar#P::Bar { width: 1; }");
        assert_eq!(s.matching(&[node("Bar", "P", "", &a), node("Bar", "", "", &a)]).len(), 1);
        assert!(s.matching(&[node("Bar", "P", "", &a), node("Bar", "", "", &a), node("Bar", "", "", &a)]).is_empty(), "children only");
    }

    #[test]
    fn templated_types() {
        let c = parse_selector("Bar#IdeaSpace Vector<Bar>#m_groupList::Bar#widgets:selected");
        assert_eq!(c.len(), 3);
        assert!(c[2].child && !c[1].child);
        assert_eq!(c[1].kind.as_deref(), Some("Vector<Bar>"));
        assert_eq!(c[1].name.as_deref(), Some("m_groupList"));
        assert_eq!(c[2].states, vec![("selected".to_string(), true)]);
    }

    /// NI's MK3 sheets, when they've been extracted (`out/ni-res`).
    #[test]
    fn loads_the_mk3_sheets() {
        let res = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/ni-res");
        if !res.is_dir() {
            return;
        }
        let roots = [res.join("maschine_nwl_resources"), res.join("shared_nwl_resources")];
        let s = Sheet::load(&roots, "Maschine/HW/MaschineMK3/MaschineMK3Styles.txt").unwrap();
        assert!(s.rules.len() > 1000, "{} rules", s.rules.len());
        assert_eq!(s.define("halfWidth"), Some("480"));
        let a = Vec::new();
        let page = s.matching(&[node("Bar", "", "Page", &a)]);
        assert!(page.iter().flat_map(|r| &r.decls).any(|(k, v)| k == "width" && *v == Value::Num(960.0)));
    }
}
