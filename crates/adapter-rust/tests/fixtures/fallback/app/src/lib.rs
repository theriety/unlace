//! Name-fallback fixture: `orphan.rs` and `loose.rs` are never declared.

pub mod inner;
pub mod user;
mod stale_cut;
mod stale_eof;
mod stale_far;
mod stale_pad;

pub use core::primitive::u8 as Byte;
pub use crate::missing::Nothing;
pub use inner as alias;
pub use std::collections;
pub use std::fmt::Display;
pub use std::vec::Vec as StdVec;
