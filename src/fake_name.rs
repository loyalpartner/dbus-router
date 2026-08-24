//! Fake Unique Name transformation for D-Bus routing
//!
//! This module implements a "Fake IP" style technique for D-Bus unique names.
//! By adding prefixes to unique names, the router can determine which bus
//! a unique name belongs to.
//!
//! # Format
//!
//! - Host bus unique names: `:h.1.123` (original: `:1.123`)
//! - Sandbox bus unique names: `:s.1.123` (original: `:1.123`)
//!
//! Well-known names (e.g., `org.fcitx.Fcitx5`) are not transformed.

use crate::bus::Bus;

/// Prefix for host bus unique names
pub const HOST_PREFIX: &str = ":h.";

/// Prefix for sandbox bus unique names
pub const SANDBOX_PREFIX: &str = ":s.";

/// Convert a real unique name to a fake name with bus prefix.
///
/// Well-known names (not starting with `:`) are returned unchanged.
///
/// # Examples
///
/// ```
/// use dbus_router::fake_name::to_fake_name;
/// use dbus_router::Bus;
///
/// assert_eq!(to_fake_name(":1.45", Bus::Host), ":h.1.45");
/// assert_eq!(to_fake_name(":1.23", Bus::Sandbox), ":s.1.23");
/// assert_eq!(to_fake_name("org.fcitx.Fcitx5", Bus::Host), "org.fcitx.Fcitx5");
/// ```
pub fn to_fake_name(real: &str, source: Bus) -> String {
    if !is_unique_name(real) {
        return real.to_string();
    }
    format!("{}{}", fake_prefix(source), &real[1..]) // drop the leading ':'
}

/// Convert a fake unique name back to the real name and determine target bus.
///
/// Returns `None` if the name is not a fake unique name (well-known name or
/// unique name without our prefix).
///
/// # Examples
///
/// ```
/// use dbus_router::fake_name::from_fake_name;
/// use dbus_router::Bus;
///
/// assert_eq!(from_fake_name(":h.1.45"), Some((":1.45".to_string(), Bus::Host)));
/// assert_eq!(from_fake_name(":s.1.23"), Some((":1.23".to_string(), Bus::Sandbox)));
/// assert_eq!(from_fake_name("org.fcitx.Fcitx5"), None);
/// assert_eq!(from_fake_name(":1.45"), None); // No prefix
/// ```
pub fn from_fake_name(fake: &str) -> Option<(String, Bus)> {
    let bus = get_bus_from_fake_name(fake)?;
    let suffix = &fake[fake_prefix(bus).len()..];
    Some((format!(":{}", suffix), bus))
}

/// Check if a name is a unique name (starts with ':').
pub fn is_unique_name(name: &str) -> bool {
    name.starts_with(':')
}

/// Check if a name is a fake unique name (has our prefix).
pub fn is_fake_unique_name(name: &str) -> bool {
    get_bus_from_fake_name(name).is_some()
}

/// The prefix a bus's unique names carry once rewritten for the client.
/// Sole owner of the bus-to-prefix mapping; everything else derives from
/// this and from `get_bus_from_fake_name`.
fn fake_prefix(bus: Bus) -> &'static str {
    match bus {
        Bus::Host => HOST_PREFIX,
        Bus::Sandbox => SANDBOX_PREFIX,
    }
}

/// Determine the target bus from a fake unique name.
///
/// Returns `None` if the name is not a fake unique name. This is the single
/// place that inspects prefixes; the predicates above are phrased in terms
/// of it so a third bus would only need the two matches in this file.
pub fn get_bus_from_fake_name(name: &str) -> Option<Bus> {
    if name.starts_with(HOST_PREFIX) {
        Some(Bus::Host)
    } else if name.starts_with(SANDBOX_PREFIX) {
        Some(Bus::Sandbox)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_fake_name_unique() {
        assert_eq!(to_fake_name(":1.45", Bus::Host), ":h.1.45");
        assert_eq!(to_fake_name(":1.45", Bus::Sandbox), ":s.1.45");
        assert_eq!(to_fake_name(":1.123", Bus::Host), ":h.1.123");
        assert_eq!(to_fake_name(":10.999", Bus::Sandbox), ":s.10.999");
    }

    #[test]
    fn test_to_fake_name_wellknown() {
        // Well-known names should not be transformed
        assert_eq!(
            to_fake_name("org.fcitx.Fcitx5", Bus::Host),
            "org.fcitx.Fcitx5"
        );
        assert_eq!(
            to_fake_name("org.freedesktop.DBus", Bus::Sandbox),
            "org.freedesktop.DBus"
        );
    }

    #[test]
    fn test_from_fake_name_host() {
        let result = from_fake_name(":h.1.45");
        assert_eq!(result, Some((":1.45".to_string(), Bus::Host)));

        let result = from_fake_name(":h.10.999");
        assert_eq!(result, Some((":10.999".to_string(), Bus::Host)));
    }

    #[test]
    fn test_from_fake_name_sandbox() {
        let result = from_fake_name(":s.1.23");
        assert_eq!(result, Some((":1.23".to_string(), Bus::Sandbox)));

        let result = from_fake_name(":s.5.678");
        assert_eq!(result, Some((":5.678".to_string(), Bus::Sandbox)));
    }

    #[test]
    fn test_from_fake_name_not_fake() {
        // Well-known names
        assert_eq!(from_fake_name("org.fcitx.Fcitx5"), None);
        assert_eq!(from_fake_name("org.freedesktop.DBus"), None);

        // Unique names without our prefix
        assert_eq!(from_fake_name(":1.45"), None);
        assert_eq!(from_fake_name(":10.999"), None);
    }

    #[test]
    fn test_roundtrip() {
        // Host bus roundtrip
        let original = ":1.45";
        let fake = to_fake_name(original, Bus::Host);
        let (restored, bus) = from_fake_name(&fake).unwrap();
        assert_eq!(restored, original);
        assert_eq!(bus, Bus::Host);

        // Sandbox bus roundtrip
        let original = ":1.23";
        let fake = to_fake_name(original, Bus::Sandbox);
        let (restored, bus) = from_fake_name(&fake).unwrap();
        assert_eq!(restored, original);
        assert_eq!(bus, Bus::Sandbox);
    }

    #[test]
    fn test_is_unique_name() {
        assert!(is_unique_name(":1.45"));
        assert!(is_unique_name(":h.1.45"));
        assert!(is_unique_name(":s.1.23"));
        assert!(!is_unique_name("org.fcitx.Fcitx5"));
        assert!(!is_unique_name(""));
    }

    #[test]
    fn test_is_fake_unique_name() {
        assert!(is_fake_unique_name(":h.1.45"));
        assert!(is_fake_unique_name(":s.1.23"));
        assert!(!is_fake_unique_name(":1.45"));
        assert!(!is_fake_unique_name("org.fcitx.Fcitx5"));
    }

    #[test]
    fn test_get_bus_from_fake_name() {
        assert_eq!(get_bus_from_fake_name(":h.1.45"), Some(Bus::Host));
        assert_eq!(get_bus_from_fake_name(":s.1.23"), Some(Bus::Sandbox));
        assert_eq!(get_bus_from_fake_name(":1.45"), None);
        assert_eq!(get_bus_from_fake_name("org.fcitx.Fcitx5"), None);
    }
}
