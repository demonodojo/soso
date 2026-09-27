// tidy-alphabetical-start
#![allow(internal_features)]
#![cfg_attr(bootstrap, feature(never_type))]
// C-005: `Error::sources` (feature `error_iter`) sólo lo usa host_dylib en unix/Windows.
#![cfg_attr(any(unix, windows), feature(error_iter))]
#![feature(file_buffered)]
#![feature(gen_blocks)]
#![feature(macro_metavar_expr)]
#![feature(min_specialization)]
#![feature(option_into_flat_iter)]
#![feature(proc_macro_internals)]
#![feature(trusted_len)]
// tidy-alphabetical-end

// C-005: en soso host_dylib no llama a libloading, pero la dependencia sigue.
#[cfg(not(any(unix, windows)))]
use libloading as _;

pub use rmeta::provide;

mod dependency_format;
mod eii;
mod foreign_modules;
mod host_dylib;
mod native_libs;
mod rmeta;

pub mod creader;
pub mod diagnostics;
pub mod fs;
pub mod locator;

pub use fs::{METADATA_FILENAME, emit_wrapper_file};
pub use host_dylib::{DylibError, load_symbol_from_dylib};
pub use rmeta::{EncodedMetadata, METADATA_HEADER, ProcMacroKind, encode_metadata, rendered_const};
