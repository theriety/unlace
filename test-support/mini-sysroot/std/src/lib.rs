//! The `std` of the test-only standard library (see `../../README.md`): facades
//! over the mini `core` and `alloc` with just the items unlace's fixtures name.
//! Its macros expand their arguments like the real ones, so references inside
//! them still resolve.
#![no_std]
pub use core::{any, cell, clone, cmp, convert, default, future, hash, hint, iter, marker, mem, ops, option, pin, ptr, result, slice, task, primitive, num, f32, f64, error, char, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize};
pub use alloc::{boxed, vec, string, borrow, rc, fmt, format};
pub mod str { pub use core::str::*; }
pub mod sync { pub use alloc::sync::Arc; pub struct Mutex<T>(T); pub struct OnceLock<T>(T); pub mod atomic { pub struct AtomicUsize(usize); pub struct AtomicBool(bool); pub enum Ordering { Relaxed, SeqCst } } }
pub mod collections {
    pub use alloc::collections::{BTreeMap, BTreeSet};
    pub struct HashMap<K, V>(K, V); impl<K, V> HashMap<K, V> { pub fn new() -> Self { loop {} } pub fn insert(&mut self, k: K, v: V) -> Option<V> { loop {} } pub fn get(&self, k: &K) -> Option<&V> { loop {} } }
    pub struct HashSet<T>(T); impl<T> HashSet<T> { pub fn new() -> Self { loop {} } pub fn insert(&mut self, t: T) -> bool { true } }
}
pub mod io { pub fn _print(args: core::fmt::Arguments<'_>) {} pub struct Error(u8); impl Error { pub fn other<E>(e: E) -> Error { loop {} } } pub type Result<T> = core::result::Result<T, Error>; pub trait Write {} pub trait Read {} }
pub mod path { pub struct Path(str); impl Path { pub fn new(s: &str) -> &Path { loop {} } pub fn join(&self, p: &str) -> PathBuf { loop {} } } pub struct PathBuf(alloc::string::String); }
pub mod fs { pub fn read_to_string(p: &crate::path::Path) -> crate::io::Result<alloc::string::String> { loop {} } pub fn write(p: &crate::path::Path, c: &[u8]) -> crate::io::Result<()> { loop {} } pub struct File; }
pub mod env { pub fn var(k: &str) -> Option<alloc::string::String> { loop {} } pub fn args() -> alloc::vec::Vec<alloc::string::String> { loop {} } pub fn temp_dir() -> crate::path::PathBuf { loop {} } }
pub mod process { pub fn exit(code: i32) -> ! { loop {} } pub fn abort() -> ! { loop {} } pub fn id() -> u32 { 0 } pub struct Command; }
pub mod thread { pub struct LocalKey<T>(fn() -> T); impl<T> LocalKey<T> { pub const fn new(f: fn() -> T) -> Self { LocalKey(f) } pub fn with<F: FnOnce(&T) -> R, R>(&'static self, f: F) -> R { loop {} } } pub fn spawn<F: FnOnce() -> T, T>(f: F) {} pub fn sleep(d: crate::time::Duration) {} }
pub mod time { pub struct Duration(u64); pub struct Instant(u64); pub struct SystemTime(u64); }
pub mod prelude {
    pub mod v1 { pub use core::prelude::v1::*; pub use core::extra_builtin::{test, bench, test_case, global_allocator, cfg_accessible, cfg_eval}; pub use alloc::{boxed::Box, vec::Vec, string::{String, ToString}, borrow::ToOwned}; }
    pub mod rust_2015 { pub use super::v1::*; }
    pub mod rust_2018 { pub use super::v1::*; }
    pub mod rust_2021 { pub use super::v1::*; }
    pub mod rust_2024 { pub use super::v1::*; }
}
#[macro_export] macro_rules! println { ($($arg:tt)*) => { $crate::io::_print(format_args!($($arg)*)) } }
#[macro_export] macro_rules! eprintln { ($($arg:tt)*) => { $crate::io::_print(format_args!($($arg)*)) } }
#[macro_export] macro_rules! print { ($($arg:tt)*) => { $crate::io::_print(format_args!($($arg)*)) } }
#[macro_export] macro_rules! eprint { ($($arg:tt)*) => { $crate::io::_print(format_args!($($arg)*)) } }
#[macro_export] macro_rules! dbg { () => {}; ($val:expr $(,)?) => { match $val { tmp => tmp } }; ($($val:expr),+ $(,)?) => { ($($crate::dbg!($val)),+,) } }
#[macro_export] macro_rules! thread_local { () => {}; ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; $($rest:tt)*) => { $vis const $name: $crate::thread::LocalKey<$t> = $crate::thread::LocalKey::new(|| $init); $crate::thread_local!($($rest)*); }; ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr) => { $vis const $name: $crate::thread::LocalKey<$t> = $crate::thread::LocalKey::new(|| $init); } }
pub use core::{assert, assert_eq, assert_ne, debug_assert, debug_assert_eq, panic, todo, unimplemented, unreachable, matches, write, writeln, format_args, concat, stringify, env, option_env, line, file, module_path, cfg, compile_error, include_str};
