use crate::Language;
use std::path::Path;

trait Analyzer<P: AsRef<Path>>: Fn(P, &[u8]) -> &Language + Clone + Send {}
trait BoolAttr<P: AsRef<Path>>: Fn(P, &[u8]) -> bool {}

pub mod git;
