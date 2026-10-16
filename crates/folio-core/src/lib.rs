//! # folio-core
//!
//! The PDF engine under Folio: an object model, a lenient lexer and parser that
//! repairs damaged files instead of giving up, stream filters, and a writer that
//! always produces a clean, fully rewritten file.
//!
//! Everything here treats its input as hostile (see `AGENTS.md` §3): malformed
//! bytes produce an [`Error`] or a recorded repair, never a panic.

pub mod crashgen;
pub mod crypt;
pub mod document;
pub mod error;
pub mod filters;
pub mod glyphs;
pub mod lexer;
pub mod object;
pub mod parser;
pub mod text_string;
pub mod writer;
mod xref;

pub use document::Document;
pub use error::{Error, Result};
pub use object::{Dict, ObjRef, Object, Stream};
