use crate::a::f;

pub fn twice() {
    println!("{} {}", f(), crate::a::f()); crate::a::g();
}
