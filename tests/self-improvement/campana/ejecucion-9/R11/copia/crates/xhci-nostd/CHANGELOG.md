# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-04-02

### Added

- Full xHCI host controller initialization per xHCI 1.2 spec section 4.2
- Capability, Operational, Runtime, and Doorbell register access
- Command Ring, Event Ring, and Transfer Ring management with cycle bit handling
- Device Context Base Address Array (DCBAA) with scratchpad buffer support
- Input Context, Slot Context, and Endpoint Context builders
- USB device enumeration: port scanning, slot allocation, address assignment
- USB descriptor parsing: Device, Configuration, Interface, Endpoint, HID
- HID boot protocol keyboard support with automatic detection
- Key event generation with press/release tracking and modifier state
- USB HID Usage ID to PS/2 scancode mapping (128 entries)
- Interrupt transfer scheduling for keyboard polling
- Controller status diagnostics via `dump_status()`
