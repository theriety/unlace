//! Edge-input fixture: `outer` is an inline module a macro defines, holding the
//! file module `outer/inner.rs`; `docs.rs` on disk is older than the source the
//! test parses for it.

macro_rules! nest {
    () => {
        pub mod outer {
            pub mod inner;
        }
    };
}

nest!();

pub mod docs;

pub fn spooky() {}
