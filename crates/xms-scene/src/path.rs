//! Scene paths: absolute, `/`-separated, USD-style (`/World/Lion/body`).

use std::fmt;
use std::sync::Arc;

/// An absolute prim path. Cheap to clone (shared string).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Path(Arc<str>);

impl Path {
    /// The root path `/`.
    pub fn root() -> Path {
        Path(Arc::from("/"))
    }

    /// Parse an absolute path. Trailing slashes are dropped; empty segments,
    /// relative paths and property paths (`.attr`) are rejected.
    pub fn new(s: &str) -> Result<Path, String> {
        if !s.starts_with('/') {
            return Err(format!("path must be absolute: `{s}`"));
        }
        let trimmed = s.trim_end_matches('/');
        if trimmed.is_empty() {
            return Ok(Path::root());
        }
        for seg in trimmed[1..].split('/') {
            if seg.is_empty() || seg.contains('.') {
                return Err(format!("invalid prim path: `{s}`"));
            }
        }
        Ok(Path(Arc::from(trimmed)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        &*self.0 == "/"
    }

    /// Last segment (`body` for `/World/Lion/body`); empty for the root.
    pub fn name(&self) -> &str {
        if self.is_root() {
            ""
        } else {
            self.0.rsplit('/').next().unwrap_or("")
        }
    }

    /// Parent path; `None` for the root.
    pub fn parent(&self) -> Option<Path> {
        if self.is_root() {
            return None;
        }
        match self.0.rfind('/') {
            Some(0) => Some(Path::root()),
            Some(i) => Some(Path(Arc::from(&self.0[..i]))),
            None => None,
        }
    }

    /// Child path `self/name`.
    pub fn child(&self, name: &str) -> Result<Path, String> {
        if name.is_empty() || name.contains('/') || name.contains('.') {
            return Err(format!("invalid prim name: `{name}`"));
        }
        Ok(if self.is_root() { Path(Arc::from(format!("/{name}"))) } else { Path(Arc::from(format!("{}/{name}", self.0))) })
    }

    /// Whether `self` is `other` or below it.
    pub fn has_prefix(&self, other: &Path) -> bool {
        other.is_root() || &*self.0 == &*other.0 || (self.0.starts_with(&*other.0) && self.0.as_bytes().get(other.0.len()) == Some(&b'/'))
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Path({})", self.0)
    }
}
