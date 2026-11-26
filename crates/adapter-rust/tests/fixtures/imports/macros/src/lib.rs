mod a;
mod child;
mod used;

macro_rules! my_dsl {
    ($($t:tt)*) => {};
}

pub fn printed() {
    println!("{}", crate::a::f());
}

pub fn asserted() {
    assert!(child::ok());
}

pub fn nested() {
    println!("{:?}", vec![crate::a::g()]);
}

pub fn quiet() {
    my_dsl! { a::f }
    my_dsl! { a::f => a::g }
}
