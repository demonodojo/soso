# xhci-nostd

[![Crates.io](https://img.shields.io/crates/v/xhci-nostd.svg)](https://crates.io/crates/xhci-nostd)
[![docs.rs](https://docs.rs/xhci-nostd/badge.svg)](https://docs.rs/xhci-nostd)
[![License](https://img.shields.io/crates/l/xhci-nostd.svg)](https://github.com/suhteevah/xhci-nostd)

A `no_std` xHCI (USB 3.0) host controller driver with HID keyboard support, written in Rust.

Designed for bare-metal OS kernels and unikernels that need USB keyboard input without any OS abstractions or standard library dependencies.

## Features

- **Full xHCI initialization**: Reset, configure, and start the host controller per xHCI 1.2 spec
- **Device enumeration**: Port scanning, slot allocation, address assignment, descriptor parsing
- **USB descriptor parsing**: Device, Configuration, Interface, Endpoint, and HID descriptors
- **HID boot protocol keyboard**: Automatic detection and setup of USB keyboards
- **Key event generation**: Press/release events with modifier tracking, USB HID to PS/2 scancode mapping
- **Transfer ring management**: Command, Event, and Transfer rings with proper cycle bit handling
- **Context management**: Device Context Base Address Array, Input/Slot/Endpoint contexts
- **Scratchpad buffer support**: Automatic allocation when required by the controller
- **`#![no_std]`**: Only depends on `alloc` and `log`

## Architecture

```
+----------------------------------+
|         XhciController           |  Top-level API
+----------------------------------+
|  Device Manager  |  HID/Keyboard |  USB descriptors, device lifecycle
+------------------+---------------+
|  Command Ring | Event Ring | TR  |  TRB ring management
+----------------------------------+
|  Device/Endpoint/Input Contexts  |  xHCI data structures
+----------------------------------+
|  Capability | Operational | RT   |  MMIO register access
+----------------------------------+
```

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
xhci-nostd = "0.1"
```

In your bare-metal kernel:

```rust,no_run
use xhci_nostd::XhciController;

// After PCI enumeration, get the xHCI controller's BAR0 address:
let bar0: usize = 0xFEB0_0000; // example

// Initialize the controller (identity-mapped memory required for DMA)
let mut controller = unsafe { XhciController::init(bar0) };

// Scan ports and enumerate connected devices
controller.enumerate_ports();

// Check if a keyboard was found
if controller.has_keyboard() {
    log::info!("USB keyboard detected!");
}

// In your main loop or interrupt handler:
loop {
    if let Some(key_event) = controller.poll_keyboard() {
        log::info!(
            "Key {}: scancode={:#x}, usage_id={:#x}",
            if key_event.pressed { "pressed" } else { "released" },
            key_event.scancode,
            key_event.usage_id,
        );
    }
    // ...
}
```

## Requirements

- A global allocator (`#[global_allocator]`) must be available
- Memory used for DMA buffers must be identity-mapped (virtual address == physical address)
- The xHCI controller's MMIO space (PCI BAR0) must be mapped and accessible

## Supported Hardware

- Any xHCI-compliant USB 3.0/3.1/3.2 host controller
- USB keyboards using the HID boot protocol (virtually all USB keyboards)
- USB 1.1 (Low/Full Speed), USB 2.0 (High Speed), and USB 3.x (SuperSpeed) devices

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contributing

Contributions are welcome! Please open an issue or pull request on [GitHub](https://github.com/suhteevah/xhci-nostd).

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

## Support This Project

If you find this project useful, consider buying me a coffee! Your support helps me keep building and sharing open-source tools.

[![Donate via PayPal](https://img.shields.io/badge/Donate-PayPal-blue.svg?logo=paypal)](https://www.paypal.me/baal_hosting)

**PayPal:** [baal_hosting@live.com](https://paypal.me/baal_hosting)

Every donation, no matter how small, is greatly appreciated and motivates continued development. Thank you!
