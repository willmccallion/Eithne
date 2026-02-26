pub mod block;
pub mod net;
pub mod pci;
pub mod serial;
pub mod traits;

pub use traits::{BlockDevice, CharDevice, Driver, NetworkDevice};
