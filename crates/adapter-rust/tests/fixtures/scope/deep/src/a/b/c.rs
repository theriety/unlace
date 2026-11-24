pub(in crate::a) fn deep() {}

pub(self) fn own() {}

pub(in crate::missing) fn lost() {}

pub(in super::crate) fn odd() {}
