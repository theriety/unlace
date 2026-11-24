macro_rules! make {
    () => {
        pub mod made {
            pub fn built() {}
        }
    };
}

make!();

pub(crate) fn beside() {}
