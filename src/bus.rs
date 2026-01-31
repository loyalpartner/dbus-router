//! Bus selection for routing.

/// Target bus for routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bus {
    Host,
    Sandbox,
}
