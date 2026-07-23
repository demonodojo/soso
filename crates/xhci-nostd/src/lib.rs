//! # xhci-nostd — Bare-metal xHCI (USB 3.0) Host Controller Driver
//!
//! This crate implements an xHCI host controller driver targeting USB keyboard
//! support via the HID boot protocol. It operates directly on memory-mapped PCI
//! BAR0 registers with no OS abstractions. Designed for `no_std` bare-metal
//! environments.
//!
//! ## Architecture
//!
//! ```text
//! +----------------------------------+
//! |         XhciController           |  driver.rs - top-level API
//! +----------------------------------+
//! |  Device Manager  |  HID/Keyboard |  device.rs, hid.rs
//! +------------------+---------------+
//! |  Command Ring | Event Ring | TR  |  ring.rs - TRB ring management
//! +----------------------------------+
//! |  Device/Endpoint/Input Contexts  |  context.rs - xHCI data structures
//! +----------------------------------+
//! |  Capability | Operational | RT   |  registers.rs - MMIO register access
//! +----------------------------------+
//! ```
//!
//! ## Usage
//!
//! ```rust,no_run
//! use xhci_nostd::XhciController;
//!
//! // In your bare-metal kernel, after PCI enumeration:
//! // let bar0 = pci_device.bar0_address();
//! // let mut controller = unsafe { XhciController::init(bar0) };
//! // controller.enumerate_ports();
//! //
//! // // In your main loop or interrupt handler:
//! // if let Some(key_event) = controller.poll_keyboard() {
//! //     // Handle key_event.scancode, key_event.pressed, etc.
//! // }
//! ```

#![no_std]
#![allow(dead_code)]

extern crate alloc;

pub mod dma;
pub mod registers;
pub mod ring;
pub mod context;
pub mod device;
pub mod hid;
pub mod driver;
pub mod mass_storage;

pub use driver::XhciController;
pub use hid::KeyEvent;
pub use mass_storage::MassStorage;
pub use dma::set_allocator;
