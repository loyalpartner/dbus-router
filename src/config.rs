//! Configuration file parsing for routing rules

use anyhow::Result;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Router configuration loaded from TOML file.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    /// Routes that should go to the host bus instead of sandbox.
    #[serde(default)]
    pub host_routes: Vec<RouteRule>,
    /// Processes allowed to register services on the host bus.
    #[serde(default)]
    pub hostpass: Vec<HostPass>,
}

/// A process allowed to register services on the host bus.
#[derive(Debug, Clone, Deserialize)]
pub struct HostPass {
    /// Path to the executable.
    pub process: PathBuf,
}

/// A routing rule that matches destinations.
#[derive(Debug, Clone, Deserialize)]
pub struct RouteRule {
    /// Destination to match. Supports exact match or "org.foo.*" wildcard.
    pub destination: String,
}

impl Config {
    /// Load configuration from a TOML file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        tracing::info!(routes = config.host_routes.len(), "Loaded configuration");
        for rule in &config.host_routes {
            tracing::debug!(destination = %rule.destination, "Host route");
        }
        Ok(config)
    }

    /// Check if a destination should be routed to the host bus.
    pub fn should_route_to_host(&self, destination: &str) -> bool {
        self.host_routes
            .iter()
            .any(|rule| rule.matches(destination))
    }

    /// Check if a process is allowed to register services on the host bus.
    pub fn has_hostpass(&self, exe_path: &Path) -> bool {
        self.hostpass.iter().any(|h| h.process == exe_path)
    }
}

impl RouteRule {
    /// Check if the destination matches this rule.
    /// Supports exact match and "org.foo.*" wildcard pattern.
    pub fn matches(&self, destination: &str) -> bool {
        if let Some(prefix) = self.destination.strip_suffix(".*") {
            // Wildcard: destination must start with prefix and either equal it
            // or have a dot after the prefix
            if destination == prefix {
                return true;
            }
            if let Some(rest) = destination.strip_prefix(prefix) {
                return rest.starts_with('.');
            }
            false
        } else {
            // Exact match
            self.destination == destination
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let rule = RouteRule {
            destination: "org.freedesktop.DBus".to_string(),
        };
        assert!(rule.matches("org.freedesktop.DBus"));
        assert!(!rule.matches("org.freedesktop.DBus.Peer"));
        assert!(!rule.matches("org.freedesktop"));
    }

    #[test]
    fn test_wildcard_match() {
        let rule = RouteRule {
            destination: "org.freedesktop.portal.*".to_string(),
        };
        assert!(rule.matches("org.freedesktop.portal.Desktop"));
        assert!(rule.matches("org.freedesktop.portal.FileChooser"));
        assert!(rule.matches("org.freedesktop.portal")); // prefix itself matches
        assert!(!rule.matches("org.freedesktop.portals")); // no dot separator
        assert!(!rule.matches("org.freedesktop.DBus"));
    }

    #[test]
    fn test_should_route_to_host() {
        let config = Config {
            host_routes: vec![
                RouteRule {
                    destination: "org.freedesktop.DBus".to_string(),
                },
                RouteRule {
                    destination: "org.freedesktop.portal.*".to_string(),
                },
            ],
            ..Default::default()
        };

        assert!(config.should_route_to_host("org.freedesktop.DBus"));
        assert!(config.should_route_to_host("org.freedesktop.portal.Desktop"));
        assert!(!config.should_route_to_host("org.example.Test"));
    }

    #[test]
    fn test_empty_config() {
        let config = Config::default();
        assert!(!config.should_route_to_host("org.freedesktop.DBus"));
    }

    #[test]
    fn test_parse_toml() {
        let toml_str = r#"
[[host_routes]]
destination = "org.freedesktop.DBus"

[[host_routes]]
destination = "org.freedesktop.portal.*"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.host_routes.len(), 2);
        assert_eq!(config.host_routes[0].destination, "org.freedesktop.DBus");
        assert_eq!(
            config.host_routes[1].destination,
            "org.freedesktop.portal.*"
        );
    }

    #[test]
    fn test_has_hostpass() {
        let config = Config {
            host_routes: vec![],
            hostpass: vec![
                HostPass {
                    process: PathBuf::from("/usr/bin/my-sandbox-app"),
                },
                HostPass {
                    process: PathBuf::from("/opt/app/bin/service"),
                },
            ],
        };

        assert!(config.has_hostpass(Path::new("/usr/bin/my-sandbox-app")));
        assert!(config.has_hostpass(Path::new("/opt/app/bin/service")));
        assert!(!config.has_hostpass(Path::new("/usr/bin/other-app")));
        assert!(!config.has_hostpass(Path::new("/usr/bin/my-sandbox-app-extra")));
    }

    #[test]
    fn test_parse_toml_with_hostpass() {
        let toml_str = r#"
[[host_routes]]
destination = "org.freedesktop.DBus"

[[hostpass]]
process = "/usr/bin/my-sandbox-app"

[[hostpass]]
process = "/opt/app/bin/service"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.host_routes.len(), 1);
        assert_eq!(config.hostpass.len(), 2);
        assert_eq!(
            config.hostpass[0].process,
            PathBuf::from("/usr/bin/my-sandbox-app")
        );
        assert_eq!(
            config.hostpass[1].process,
            PathBuf::from("/opt/app/bin/service")
        );
    }
}
