//! Project-defined SV2 extensions: `0x0003` coinbase payouts and `0x5350`
//! delivery capabilities. See docs/SPEC.md §3.5 and §11.4.

#![deny(unsafe_code)]

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
