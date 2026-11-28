//! The `alloc` of the test-only standard library (see `../../README.md`): the
//! owned types and macros unlace's fixtures name, with bodies left out.
#![no_std]
pub mod boxed { pub struct Box<T: ?Sized>(*mut T); impl<T> Box<T> { pub fn new(x: T) -> Self { loop {} } } }
pub mod vec {
    pub struct Vec<T> { ptr: *mut T, len: usize }
    impl<T> Vec<T> {
        pub const fn new() -> Self { loop {} }
        pub fn with_capacity(n: usize) -> Self { loop {} }
        pub fn push(&mut self, x: T) {}
        pub fn pop(&mut self) -> Option<T> { loop {} }
        pub fn len(&self) -> usize { 0 }
        pub fn is_empty(&self) -> bool { true }
        pub fn iter(&self) -> core::slice::Iter<'_, T> { loop {} }
    }
    impl<T> core::ops::Deref for Vec<T> { type Target = [T]; fn deref(&self) -> &[T] { loop {} } }
    impl<T> core::iter::IntoIterator for Vec<T> { type Item = T; type IntoIter = IntoIter<T>; fn into_iter(self) -> IntoIter<T> { loop {} } }
    pub struct IntoIter<T>(Vec<T>);
    impl<T> core::iter::Iterator for IntoIter<T> { type Item = T; fn next(&mut self) -> Option<T> { loop {} } }
    impl<T> core::iter::FromIterator<T> for Vec<T> { fn from_iter<I: core::iter::IntoIterator<Item = T>>(i: I) -> Self { loop {} } }
}
pub mod string {
    pub struct String { v: crate::vec::Vec<u8> }
    impl String {
        pub const fn new() -> String { loop {} }
        pub fn push_str(&mut self, s: &str) {}
        pub fn push(&mut self, c: char) {}
        pub fn len(&self) -> usize { 0 }
        pub fn is_empty(&self) -> bool { true }
        pub fn as_str(&self) -> &str { loop {} }
    }
    impl core::ops::Deref for String { type Target = str; fn deref(&self) -> &str { loop {} } }
    impl core::convert::From<&str> for String { fn from(s: &str) -> String { loop {} } }
    impl core::fmt::Display for String { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { loop {} } }
    impl core::fmt::Debug for String { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { loop {} } }
    impl core::clone::Clone for String { fn clone(&self) -> String { loop {} } }
    impl core::cmp::PartialEq for String { fn eq(&self, o: &String) -> bool { true } }
    impl core::cmp::Eq for String {}
    pub trait ToString { fn to_string(&self) -> String; }
    impl<T: core::fmt::Display + ?Sized> ToString for T { fn to_string(&self) -> String { loop {} } }
}
pub mod borrow { pub use core::borrow::*; pub trait ToOwned { type Owned; fn to_owned(&self) -> Self::Owned; } }
pub mod rc { pub struct Rc<T: ?Sized>(*mut T); }
pub mod sync { pub struct Arc<T: ?Sized>(*mut T); }
pub mod fmt { pub use core::fmt::*; pub fn format(args: Arguments<'_>) -> crate::string::String { loop {} } }
pub mod collections {
    pub struct BTreeMap<K, V>(K, V); impl<K, V> BTreeMap<K, V> { pub fn new() -> Self { loop {} } pub fn insert(&mut self, k: K, v: V) -> Option<V> { loop {} } pub fn get(&self, k: &K) -> Option<&V> { loop {} } }
    pub struct BTreeSet<T>(T); impl<T> BTreeSet<T> { pub fn new() -> Self { loop {} } pub fn insert(&mut self, t: T) -> bool { true } pub fn contains(&self, t: &T) -> bool { true } }
}
pub fn from_elem<T>(elem: T, n: usize) -> vec::Vec<T> { loop {} }
pub fn into_vec<T, const N: usize>(b: boxed::Box<[T; N]>) -> vec::Vec<T> { loop {} }
#[macro_export] macro_rules! vec { () => { $crate::vec::Vec::new() }; ($elem:expr; $n:expr) => { $crate::from_elem($elem, $n) }; ($($x:expr),+ $(,)?) => { $crate::into_vec($crate::boxed::Box::new([$($x),+])) } }
#[macro_export] macro_rules! format { ($($arg:tt)*) => { $crate::fmt::format(format_args!($($arg)*)) } }
