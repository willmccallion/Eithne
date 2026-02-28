pub mod devfs;
pub mod ramfs;
pub mod traits;
pub mod vfs;

pub use traits::{DirEntry, File, FileSystem};
pub use vfs::{close, dup, init, mkdir, open, read, readdir, stat, unlink, write};
