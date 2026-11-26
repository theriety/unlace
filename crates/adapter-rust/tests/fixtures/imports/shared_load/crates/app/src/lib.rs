mod other;
mod shared;
#[cfg(test)]
#[path = "shared.rs"]
mod shared_again;
#[path = "generated.in"]
mod generated;

pub fn run() -> other::Thing {
    ::other::Thing
}
