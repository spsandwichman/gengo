use crate::Language;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

trait Analyzer<P: AsRef<Path>>: Fn(P, &[u8]) -> &Language + Clone + Send {}

#[derive(Eq, Hash, PartialEq)]
pub struct ResponseEntry {
    pub language: Language,
    pub documentation_override: Option<bool>,
    pub generated_override: Option<bool>,
    pub vendored_override: Option<bool>,
}

pub type Response = HashMap<PathBuf, ResponseEntry>;

pub mod git;
