/// A buffer, by the integer Lua holds, as Neovim's `bufnr`. Never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BufId(pub(crate) u32);
