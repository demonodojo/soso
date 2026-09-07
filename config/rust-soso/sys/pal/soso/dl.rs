//! Cargador dinámico mínimo para proc-macros (Hito 3b).
//!
//! Contingencia: pre-expansión en host de `serde_derive`, `thiserror-impl`, etc.

use core::ffi::c_void;

pub struct Library(*mut c_void);

impl Library {
    pub unsafe fn open(_path: &str) -> Result<Self, String> {
        Err("dlopen: no implementado; usar pre-expansión en host".into())
    }

    pub unsafe fn symbol<T>(&self, _name: &str) -> Result<T, String> {
        Err("dlopen: biblioteca no cargada".into())
    }
}

pub unsafe fn close(_lib: Library) {}
