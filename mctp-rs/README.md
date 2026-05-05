# mctp-rs

A `no_std` Rust implementation of the Management Component Transport Protocol (MCTP) as defined in the [DMTF DSP0236 specification](https://www.dmtf.org/sites/default/files/standards/documents/DSP0236_1.3.3.pdf).

## Overview

MCTP is a communication protocol designed for platform management subsystems in computer systems. It facilitates communication between management controllers (like BMCs) and managed devices across various bus types. This library provides:

- **Protocol Implementation**: Complete MCTP transport layer with packet assembly/disassembly
- **Medium Abstraction**: Support for different physical transport layers (SMBus/eSPI included)
- **No-std Compatible**: Suitable for embedded and resource-constrained environments

## Features

- `espi` - Enables eSPI device support via the `espi-device` crate

## Documentation & Usage

See the crate documentation for up-to-date usage and examples: [Rendered Docs](https://dymk.github.io/mctp-rs/)

## Architecture

The library is structured around:

- **`MctpPacketContext`**: Main entry point for handling MCTP packets
- **`MctpMedium`**: Trait for implementing transport-specific packet handling
- **`MctpMessage`**: Represents a complete MCTP message with reply context
- **Control Commands**: Type-safe implementation of MCTP control protocol


## License

MIT License - see [LICENSE.md](LICENSE.md) for details.

## Contributing

1. Ensure `cargo check` and `cargo test` pass
2. Test with all feature combinations using `cargo hack --feature-powerset check`
3. Maintain `no_std` compatibility
4. Follow the existing code patterns for protocol message handling
