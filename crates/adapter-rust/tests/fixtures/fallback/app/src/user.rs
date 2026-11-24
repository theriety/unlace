use crate::inner;

pub fn via_workspace_root() {
    inner::Ghost::make();
}

pub fn via_external_root() {
    std::Ghost::make();
}

pub fn via_module_file() {
    orphan::Ghost::make();
}

pub fn via_unknown_root() {
    nowhere::Ghost::make();
}

pub fn via_global_path() {
    ::nowhere::Ghost::make();
}

pub fn via_macro() {
    undefined!(Ghost::make());
}

pub fn spaced_call() {
    spooky ();
}

pub fn imports() {
    use crate::inner::present;
    present();
}
