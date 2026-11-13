#![deny(unsafe_op_in_unsafe_fn)]
pub mod arib;
pub mod caption;
pub mod caption_layout;
pub mod cdt;
mod drcs;
#[cfg(feature = "drcs-collect")]
#[path = "../tools/drcs/collector.rs"]
mod drcs_dump;
pub mod dsm;
pub mod dtvclient;
mod dtvclient_ffi;
pub mod engine;
pub mod events;
mod ffi;
pub mod json;
pub mod si;
mod tables;
pub mod time;
pub mod ts;
mod windows;

pub mod srt;
pub mod stream;
mod stream_ffi;
mod stream_filter;

#[cfg(test)]
#[path = "../tests/unit/stream.rs"]
mod stream_tests;
