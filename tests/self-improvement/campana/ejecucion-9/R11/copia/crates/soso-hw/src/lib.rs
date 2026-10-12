//! Lógica de hardware comprobable en host, compartida con el kernel.
//!
//! - [`ivrs`]: tabla ACPI IVRS (IOMMU AMD-Vi).
//! - [`fbrot`]: geometría de rotación de la consola framebuffer.

#![no_std]

pub mod fbrot;
pub mod ivrs;
