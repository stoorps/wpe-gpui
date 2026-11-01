#[derive(Clone, Copy, Debug, Default)]
pub struct FindOptions {
    pub case_sensitive: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindState {
    pub matches: u32,
    pub searching: bool,
}
