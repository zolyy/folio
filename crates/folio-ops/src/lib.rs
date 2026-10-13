//! # folio-ops
//!
//! Operations on whole documents, built to match Acrobat Pro's options
//! (see `docs/acrobat/organize.md`): organize pages, combine, split, page
//! labels, document properties, reduce file size and text export.
//!
//! Every operation takes a [`Document`], returns a [`Result`], and never
//! panics on damaged input (`AGENTS.md` §3).

pub mod compress;
pub mod info;
pub mod json;
pub mod labels;
pub mod merge;
pub mod meta;
pub mod outline;
pub mod pages;
pub mod ranges;
pub mod samples;
pub mod split;
pub mod text;

pub use folio_core::{Document, Error, Result};
