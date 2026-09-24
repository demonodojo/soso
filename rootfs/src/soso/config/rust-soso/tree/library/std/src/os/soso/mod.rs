#![stable(feature = "rust1", since = "1.0.0")]
#![deny(unsafe_op_in_unsafe_fn)]

#[allow(unused_extern_crates)]
#[stable(feature = "rust1", since = "1.0.0")]
pub extern crate soso_rt as soso_abi;

pub mod ffi;
pub mod io;

#[stable(feature = "rust1", since = "1.0.0")]
pub mod prelude {
    #[doc(no_inline)]
    #[stable(feature = "rust1", since = "1.0.0")]
    pub use super::ffi::{OsStrExt, OsStringExt};
}
