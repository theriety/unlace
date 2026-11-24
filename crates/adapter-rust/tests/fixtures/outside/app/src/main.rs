mod tool;
#[path = "../outer.rs"]
mod outer;

fn main() {
    tool::helper();
    outer::beyond();
}
