//! Representation of the JSON format used by CNI. See the [CNI Specification](https://github.com/containernetworking/cni/blob/master/SPEC.md).

use std::net::IpAddr;
use std::{collections::HashMap, fmt};

use serde::{de, Deserialize, Serialize};
use serde_json::Value;

/// A versioned CNI object. Many objects in the CNI specification are reused, but only the top-level object generally specifies a version. This wrapper allows
/// reusing the corresponding type definitions.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Versioned<T> {
    /// Semantic Version 2.0 of the CNI specification to which this object conforms.
    #[serde(rename = "cniVersion")]
    cni_version: String,

    #[serde(flatten)]
    payload: T,
}

/// CNI network configuration
///
/// [Specification](https://github.com/containernetworking/cni/blob/master/SPEC.md#network-configuration).
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkConfiguration {
    /// Network name. This should be unique across all containers on the host (or other administrative domain).
    /// Must start with a alphanumeric character, optionally followed by any combination of one or more alphanumeric
    /// characters, underscore (_), dot (.) or hyphen (-).
    name: String,

    #[serde(flatten)]
    plugin: PluginConfiguration,
}

/// CNI network configuration list.
///
/// [Specification](https://github.com/containernetworking/cni/blob/master/SPEC.md#network-configuration-lists)
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkConfigurationList {
    /// Network name. This should be unique across all containers on the host (or other administrative domain).
    /// Must start with a alphanumeric character, optionally followed by any combination of one or more alphanumeric
    /// characters, underscore (_), dot (.) or hyphen (-).
    name: String,

    /// f disableCheck is true, runtimes must not call CHECK for this network configuration list. This allows an administrator to prevent CHECKing where a combination of plugins is known to return spurious errors.
    #[serde(skip_serializing_if = "is_false")]
    #[serde(rename = "disableCheck")]
    #[serde(default)]
    disable_check: bool,

    /// A list of standard CNI network plugin configurations.
    plugins: Vec<PluginConfiguration>,
}

/// Configuration for a single CNI plugin. This may be included in either a single-plugin [`NetworkConfiguration`] or a multi-plugin
/// [`NetworkConfigurationList`].
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginConfiguration {
    /// Refers to the filename of the CNI plugin executable.
    #[serde(rename = "type")]
    plugin_type: String,

    /// Additional arguments provided by the container runtime. For example a dictionary of labels could be passed to CNI
    /// plugins by adding them to a labels field under args.
    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    args: HashMap<String, Value>,

    /// If supported by the plugin, sets up an IP masquerade on the host for this network.
    /// This is necessary if the host will act as a gateway to subnets that are not able to route to the IP assigned to the container.
    #[serde(rename = "ipMasq")]
    #[serde(default)]
    #[serde(skip_serializing_if = "is_false")]
    ip_masq: bool,

    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    ipam: Option<IpamConfiguration>,

    /// DNS-specific configuration
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    dns: Option<DnsConfiguration>,

    /// Additional plugin-specific fields. Plugins may define additional fields that they accept and may generate an error if called with unknown fields.
    /// However, plugins should ignore fields in [`args`] if they are not understood.
    #[serde(flatten)]
    other: HashMap<String, Value>,
}

/// IPAM (IP Address Management) plugin configuration.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IpamConfiguration {
    /// Refers to the filename of the IPAM plugin executable.
    #[serde(rename = "type")]
    plugin_type: String,

    /// Additional plugin-specific fields. Plugins may define additional fields that they accept and may generate an error if called with unknown fields.
    #[serde(flatten)]
    other: HashMap<String, Value>,
}

/// Common DNS information.
///
/// [DNS well-known type](https://github.com/containernetworking/cni/blob/master/SPEC.md#dns).
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DnsConfiguration {
    /// A priority-ordered list of DNS nameservers that this network is aware of
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    nameservers: Vec<IpAddr>,

    /// The local domain used for short hostname lookups
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    domain: Option<String>,

    /// List of priority-ordered search domains for short hostname lookups. Will be preferred over [`domain`]
    /// by most resolvers.
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    search: Vec<String>,

    /// List of options that can be passed to the resolver.
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    options: Vec<String>,
}

/// Result of a CNI plugin invocation.
///
/// [Result specification](https://github.com/containernetworking/cni/blob/master/SPEC.md#result).
#[derive(Debug, Deserialize)]
pub struct PluginResult {
    /// Specific network interfaces the plugin created. If the `CNI_IFNAME` variable exists the plugin must use that name for the sandbox/hypervisor
    /// interface or return an error if it cannot.
    #[serde(default)]
    interfaces: Vec<Interface>,

    #[serde(default)]
    ips: Vec<IpConfiguration>,

    #[serde(default)]
    routes: Vec<RouteConfiguration>,

    #[serde(default)]
    dns: Option<DnsConfiguration>,
}

/// A network interface created by a CNI plugin.
#[derive(Debug, Deserialize)]
pub struct Interface {
    /// Network interface name.
    name: String,

    /// The hardware address of the interface. If L2 addresses are not meaningful for the plugin then this field is optional.
    #[serde(default)]
    mac: Option<String>,

    /// Container/namespace-based environments should return the full filesystem path to the network namespace of that sandbox.
    /// Hypervisor/VM-based plugins should return an ID unique to the virtualized sandbox the interface was created in. This
    /// item must be provided for interfaces created or moved into a sandbox like a network namespace or a hypervisor/VM.
    sandbox: String,
}

/// IP configuration information provided by a CNI plugin.
///
/// [IP well-known structure](https://github.com/containernetworking/cni/blob/master/SPEC.md#ips).
#[derive(Debug, Deserialize)]
pub struct IpConfiguration {
    /// IP address range in CIDR notation
    address: String,

    /// The default gateway for this subnet, if one exists. It does not instruct the CNI plugin to add any routes with this gateway:
    /// routes to add are specified separately via the routes field. An example use of this value is for the CNI bridge plugin to add
    /// this IP address to the Linux bridge to make it a gateway.
    #[serde(default)]
    gateway: Option<String>,

    /// Index into the [`Result::interfaces`] list of a CNI plugin result indicating which interface this IP configuration should be applied
    /// to.
    interface: usize,
}

/// IP routing configuration. Each `RouteConfiguration` must be relevant to the sandbox interface specified by `CNI_IFNAME`.
/// Routes are expected to be added with a 0 metric. A default route may be specified via "0.0.0.0/0". Since another network
/// might have already configured the default route, the CNI plugin should be prepared to skip over its default route definition.
#[derive(Debug, Deserialize)]
pub struct RouteConfiguration {
    /// Destination subnet specified in CIDR notation.
    #[serde(rename = "dst")]
    destination: String,

    /// IP of the gateway. If omitted, a default gateway is assumed (as determined by the CNI plugin).
    #[serde(rename = "gw")]
    #[serde(default)]
    gateway: Option<String>,
}

/// Abbreviated form of [`Result`] returned by IPAM plugins.
///
/// [IP Allocation specification](https://github.com/containernetworking/cni/blob/master/SPEC.md#ip-allocation).
#[derive(Debug, Deserialize)]
pub struct IpamResult {
    /// IP configuration
    ips: Vec<IpamIpConfiguration>,

    /// Route configuration.
    #[serde(default)]
    routes: Vec<RouteConfiguration>,

    /// Common DNS information.
    dns: Option<DnsConfiguration>,
}

/// Version of [`IpConfiguration`] that omits fields that should not be returned by IPAM plugins.
#[derive(Debug, Deserialize)]
pub struct IpamIpConfiguration {
    /// IP address range in CIDR notation
    address: String,

    /// The default gateway for this subnet, if one exists. It does not instruct the CNI plugin to add any routes with this gateway:
    /// routes to add are specified separately via the routes field. An example use of this value is for the CNI bridge plugin to add
    /// this IP address to the Linux bridge to make it a gateway.
    #[serde(default)]
    gateway: Option<String>,
}

/// A CNI plugin error. Note that plugins may also log unstructured information to stderr.
#[derive(Debug, Deserialize)]
pub struct Error {
    code: ErrorCode,

    #[serde(rename = "msg")]
    message: String,

    #[serde(default)]
    details: Option<String>,
}

/// A CNI error code. See the [Well-known Error Codes](https://github.com/containernetworking/cni/blob/master/SPEC.md#well-known-error-codes).
#[derive(Debug)]
pub enum ErrorCode {
    IncompatibleCniVersion,
    UnsupportedConfigurationField,
    ContainerUnknown,
    InvalidEnvironmentVariable,
    Io,
    Decode,
    InvalidNetworkConfiguration,
    Transient,
    Reserved(u32),
    Plugin(u32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "CNI error ({}): {}", self.code, self.message)?;
        if let Some(details) = &self.details {
            write!(f, " ({})", details)?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D>(deserializer: D) -> Result<ErrorCode, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        struct ErrorCodeVisitor;

        impl<'de> de::Visitor<'de> for ErrorCodeVisitor {
            type Value = ErrorCode;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a CNI error code")
            }

            fn visit_u32<E>(self, value: u32) -> Result<Self::Value, E> {
                match value {
                    1 => Ok(ErrorCode::IncompatibleCniVersion),
                    2 => Ok(ErrorCode::UnsupportedConfigurationField),
                    3 => Ok(ErrorCode::ContainerUnknown),
                    4 => Ok(ErrorCode::InvalidEnvironmentVariable),
                    5 => Ok(ErrorCode::Io),
                    6 => Ok(ErrorCode::Decode),
                    7 => Ok(ErrorCode::InvalidNetworkConfiguration),
                    11 => Ok(ErrorCode::Transient),
                    8 | 9 | 12..=99 => Ok(ErrorCode::Reserved(value)),
                    _ => Ok(ErrorCode::Plugin(1)),
                }
            }
        }

        deserializer.deserialize_u32(ErrorCodeVisitor)
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ErrorCode::IncompatibleCniVersion => f.write_str("Incompatible CNI version"),
            ErrorCode::UnsupportedConfigurationField => {
                f.write_str("Unsupported field in network configuration")
            }
            ErrorCode::ContainerUnknown => f.write_str("Container unknown or does not exist"),
            ErrorCode::InvalidEnvironmentVariable => {
                f.write_str("Invalid necessary environment variables")
            }
            ErrorCode::Io => f.write_str("I/O failure"),
            ErrorCode::Decode => f.write_str("Failed to decode content"),
            ErrorCode::InvalidNetworkConfiguration => f.write_str("Invalid network config"),
            ErrorCode::Transient => f.write_str("Try again later"),
            ErrorCode::Reserved(code) => write!(f, "reserved error {}", code),
            ErrorCode::Plugin(code) => write!(f, "plugin-specific error {}", code),
        }
    }
}

/// Helper for Serde's `skip_serializing_if` attribute.
fn is_false(v: &bool) -> bool {
    !*v
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;

    use serde::{Serialize, de::DeserializeOwned};
    use serde_json::{self, Value, json};

    use super::*;

    /// Helper to assert that a value deserializes from / serializes to the expected JSON.
    fn assert_roundtrip<T>(value: T, expected: Value) where T: DeserializeOwned + Serialize + Eq + Debug {
        let encoded = serde_json::to_value(&value).expect("encode failed");
        if encoded != expected {
            panic!(r"Value did not serialize as expected!
  Value:
{:?}
  Expected JSON:
{:#}
  Actual JSON:
{:#}
", value, expected, encoded);
        }

        let decoded: T = serde_json::from_value(expected.clone()).expect("decode failed");
        if decoded != value {
            panic!(r"JSON did not deserialize as expected!
  JSON:
{:#}
  Expected value:
{:?}
  Actual value:
{:?}", expected, value, decoded);
        }
    }

    #[test]
    fn test_single_plugin() {
        // Taken from https://github.com/containernetworking/cni/blob/master/SPEC.md#example-bridge-configuration
        let config = Versioned {
            cni_version: "1.0.0".into(),
            payload: NetworkConfiguration {
                name: "dbnet".into(),
                plugin: PluginConfiguration {
                    plugin_type: "bridge".into(),
                    other: {
                        let mut map = HashMap::new();
                        map.insert("bridge".into(), json!("cni0"));
                        map
                    },
                    ipam: Some(IpamConfiguration {
                        plugin_type: "host-local".into(),
                        other: {
                            let mut map = HashMap::new();
                            map.insert("subnet".into(), json!("10.1.0.0/16"));
                            map.insert("gateway".into(), json!("10.1.0.1"));
                            map
                        },
                    }),
                    ip_masq: false,
                    dns: Some(DnsConfiguration {
                        nameservers: vec!["10.1.0.1".parse().unwrap()],
                        domain: None,
                        search: Vec::new(),
                        options: Vec::new(),
                    }),
                    args: HashMap::new(),
                },
            },
        };

        let json = json!({
            "cniVersion": "1.0.0",
            "name": "dbnet",
            "type": "bridge",
            "bridge": "cni0",
            "ipam": {
                "type": "host-local",
                "subnet": "10.1.0.0/16",
                "gateway": "10.1.0.1"
            },
            "dns": {
                "nameservers": [ "10.1.0.1" ]
            }
        });

        assert_roundtrip(config, json);
    }

    #[test]
    fn test_plugin_list() {
        // Taken from https://github.com/containernetworking/cni/blob/master/SPEC.md#example-network-configuration-lists
        let config = Versioned {
            cni_version: "1.0.0".into(),
            payload: NetworkConfigurationList {
                name: "dbnet".into(),
                disable_check: false,
                plugins: vec![
                    PluginConfiguration {
                        plugin_type: "bridge".into(),
                        other: {
                            let mut map = HashMap::new();
                            map.insert("bridge".into(), json!("cni0"));
                            map
                        },
                        args: {
                            let mut map = HashMap::new();
                            map.insert("labels".into(), json!({
                                "appVersion": "1.0"
                            }));
                            map
                        },
                        ipam: Some(IpamConfiguration {
                            plugin_type: "host-local".into(),
                            other: {
                                let mut map = HashMap::new();
                                map.insert("subnet".into(), json!("10.1.0.0/16"));
                                map.insert("gateway".into(), json!("10.1.0.1"));
                                map
                            },    
                        }),
                        dns: Some(DnsConfiguration {
                            nameservers: vec!["10.1.0.1".parse().unwrap()],
                            domain: None,
                            search: Vec::new(),
                            options: Vec::new(),
                        }),
                        ip_masq: false,
                    },
                    PluginConfiguration {
                        plugin_type: "tuning".into(),
                        other: {
                            let mut map = HashMap::new();
                            map.insert("sysctl".into(), json!({
                                "net.core.somaxconn": "500"
                            }));
                            map
                        },
                        args: HashMap::new(),
                        ipam: None,
                        ip_masq: false,
                        dns: None,
                    }
                ]
            }
        };

        let json = json!({
            "cniVersion": "1.0.0",
            "name": "dbnet",
            "plugins": [
                {
                    "type": "bridge",
                    "bridge": "cni0",
                    "args": {
                        "labels": {
                            "appVersion": "1.0"
                        }
                    },
                    "ipam": {
                        "type": "host-local",
                        "subnet": "10.1.0.0/16",
                        "gateway": "10.1.0.1"
                    },
                    "dns": {
                        "nameservers": [ "10.1.0.1" ]
                    }
                },
                {
                    "type": "tuning",
                    "sysctl": {
                        "net.core.somaxconn": "500"
                    }
                }
            ]
        });

        assert_roundtrip(config, json);
    }
}
