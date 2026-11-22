//! Network probing.
//!
//! Monitoring is layered, the way Android's connectivity stack does it:
//!
//! 1. Cheap local signals (sysfs link state, the routing table, the source address) are read
//!    every [`LOCAL_POLL`]. That is a few file reads and one UDP `connect`, with no network
//!    traffic and no child processes.
//! 2. Real probes (gateway, internet, public IP) run only when those local signals change, or
//!    on a timer: slow while online, backing off while offline.
//! 3. A tier must fail twice in a row before it flips from up to down, so one dropped packet
//!    does not flicker the UI.
//!
//! Everything that can block (DNS, ICMP, TCP and HTTP) runs on the monitor thread. The UI only
//! ever reads a finished [`Snapshot`], which is why the window opens immediately even when the
//! machine has no network at all.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpStream, ToSocketAddrs, UdpSocket};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often the cheap local signals are read. File reads only, no network traffic.
const LOCAL_POLL: Duration = Duration::from_millis(500);
/// Probe cadence while the internet is reachable.
const ONLINE_INTERVAL: Duration = Duration::from_secs(10);
/// Quick follow-up probe while a failure is still being confirmed.
const RECHECK_DELAY: Duration = Duration::from_secs(1);
/// Offline probes back off 1s, 2s, 4s and then stay at this cap.
const OFFLINE_MAX_DELAY: Duration = Duration::from_secs(5);
/// How long a fetched public IP is trusted before it is looked up again.
const PUBLIC_IP_REFRESH: Duration = Duration::from_secs(600);
/// How long to wait before retrying a failed public IP lookup.
const PUBLIC_IP_RETRY: Duration = Duration::from_secs(60);

const HTTP_TIMEOUT: Duration = Duration::from_secs(3);
const TCP_PROBE_TIMEOUT: Duration = Duration::from_millis(1200);
const ROUTER_PORT_TIMEOUT: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Ethernet,
    Wifi,
    Other,
}

impl LinkKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ethernet => "Ethernet",
            Self::Wifi => "Wi-Fi",
            Self::Other => "Other",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub name: String,
    pub kind: LinkKind,
    pub up: bool,
    /// The kernel's `LOWER_UP` flag: the cable has link, or the Wi-Fi radio is associated.
    pub lower_up: bool,
    pub addresses: Vec<String>,
}

impl Interface {
    pub fn has_ipv4(&self) -> bool {
        !self.addresses.is_empty()
    }

    /// Mirrors the original app: Ethernet follows the physical link, Wi-Fi needs an address.
    pub fn connected(&self) -> bool {
        match self.kind {
            LinkKind::Ethernet => self.lower_up,
            LinkKind::Wifi | LinkKind::Other => self.up && self.has_ipv4(),
        }
    }

    pub fn state_text(&self) -> &'static str {
        match self.kind {
            LinkKind::Ethernet if self.lower_up => "Plugged in",
            LinkKind::Ethernet => "Plugged out",
            LinkKind::Wifi | LinkKind::Other if self.up && self.has_ipv4() => "Connected",
            LinkKind::Wifi | LinkKind::Other => "Disconnected",
        }
    }
}

/// One finished look at the network. Cheap to clone; the UI keeps the newest one.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub local_ip: Option<String>,
    pub gateway_ip: Option<String>,
    pub provider_gateway_ip: Option<String>,
    pub public_ip: Option<String>,
    pub desktop_up: bool,
    pub router_up: bool,
    pub provider_gateway_up: bool,
    pub internet_up: bool,
    pub interfaces: Vec<Interface>,
    #[allow(dead_code)]
    pub finished_at: Instant,
}

// ---------------------------------------------------------------------------
// Monitor loop
// ---------------------------------------------------------------------------

/// Start the single background probe thread.
pub fn spawn_monitor(state: Arc<Mutex<Option<Snapshot>>>, ctx: egui::Context) {
    let _ = std::thread::Builder::new()
        .name("network-probe".to_owned())
        .spawn(move || {
            // Returns false when the UI side has gone away (poisoned lock) and we should stop.
            let publish = |snapshot: Snapshot| -> bool {
                let Ok(mut guard) = state.lock() else {
                    return false;
                };
                *guard = Some(snapshot);
                drop(guard);
                ctx.request_repaint();
                true
            };

            let mut last_fingerprint: Option<LinkFingerprint> = None;
            let mut next_probe = Instant::now();
            let mut router_fails = 0u32;
            let mut provider_gateway_fails = 0u32;
            let mut internet_fails = 0u32;
            let mut offline_streak = 0u32;
            // (router_up, provider_gateway_up, internet_up) as last published, used for debouncing.
            let mut previous: Option<(bool, bool, bool)> = None;
            let mut public_ip: Option<String> = None;
            let mut ip_checked_at: Option<Instant> = None;

            loop {
                // Layer 1: cheap local signals. The first pass always counts as a change.
                let fingerprint = LinkFingerprint::read();
                let local_changed = last_fingerprint.as_ref() != Some(&fingerprint);
                last_fingerprint = Some(fingerprint);

                // Layer 2: real probes, only on a local change or when the timer is due.
                if local_changed || Instant::now() >= next_probe {
                    if local_changed {
                        // New link, new address or new route: restart from a clean slate.
                        offline_streak = 0;
                        public_ip = None;
                        ip_checked_at = None;
                    }

                    let mut snapshot = probe();
                    let raw_router = snapshot.router_up;
                    let raw_provider_gateway = snapshot.provider_gateway_up;
                    let raw_internet = snapshot.internet_up;

                    // Layer 3: debounce. A fresh local change is trusted immediately.
                    if local_changed {
                        router_fails = 0;
                        provider_gateway_fails = 0;
                        internet_fails = 0;
                    } else if let Some((was_router, was_provider, was_internet)) = previous {
                        snapshot.router_up = debounce(was_router, raw_router, &mut router_fails);
                        snapshot.provider_gateway_up = debounce(was_provider, raw_provider_gateway, &mut provider_gateway_fails);
                        snapshot.internet_up = debounce(was_internet, raw_internet, &mut internet_fails);
                    }
                    let held = (snapshot.router_up && !raw_router)
                        || (snapshot.provider_gateway_up && !raw_provider_gateway)
                        || (snapshot.internet_up && !raw_internet);
                    previous = Some((snapshot.router_up, snapshot.provider_gateway_up, snapshot.internet_up));

                    if snapshot.internet_up {
                        snapshot.public_ip = public_ip.clone();
                    } else {
                        public_ip = None;
                        ip_checked_at = None;
                    }

                    let delay = if !snapshot.internet_up {
                        let delay = offline_delay(offline_streak);
                        offline_streak += 1;
                        delay
                    } else {
                        offline_streak = 0;
                        if held { RECHECK_DELAY } else { ONLINE_INTERVAL }
                    };
                    next_probe = Instant::now() + delay;

                    // Only look up the public IP when it is missing or stale, never every cycle.
                    let ip_due = ip_checked_at.is_none_or(|checked| {
                        let max_age = if public_ip.is_some() {
                            PUBLIC_IP_REFRESH
                        } else {
                            PUBLIC_IP_RETRY
                        };
                        checked.elapsed() > max_age
                    });
                    let want_ip = snapshot.internet_up && !held && ip_due;

                    // Publish the status first so the UI never waits on the IP lookup.
                    if !publish(snapshot.clone()) {
                        break;
                    }

                    if want_ip {
                        public_ip = fetch_public_ip();
                        ip_checked_at = Some(Instant::now());
                        snapshot.public_ip = public_ip.clone();
                        if !publish(snapshot) {
                            break;
                        }
                    }
                }

                std::thread::sleep(LOCAL_POLL);
            }
        });
}

/// Report "down" only after two consecutive failed probes.
fn debounce(was_up: bool, now_up: bool, fails: &mut u32) -> bool {
    if now_up {
        *fails = 0;
        true
    } else if was_up && *fails < 1 {
        *fails += 1;
        true
    } else {
        false
    }
}

/// Offline probes back off 1s, 2s, 4s, then hold at [`OFFLINE_MAX_DELAY`].
fn offline_delay(failures: u32) -> Duration {
    Duration::from_secs(1u64 << failures.min(3)).min(OFFLINE_MAX_DELAY)
}

// ---------------------------------------------------------------------------
// Local fingerprint (layer 1)
// ---------------------------------------------------------------------------

/// Everything that can change without any network probe noticing: link state per physical
/// interface, the routing table and the source address. Comparing two of these is nearly free.
#[derive(Debug, PartialEq, Eq)]
struct LinkFingerprint {
    /// (interface, operstate, carrier) for Ethernet and Wi-Fi interfaces only, so virtual
    /// interfaces such as Docker's veth pairs coming and going do not trigger probes.
    links: Vec<(String, String, String)>,
    routes: String,
    source_ip: Option<String>,
}

impl LinkFingerprint {
    fn read() -> Self {
        let mut links = Vec::new();
        if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if classify(&name) == LinkKind::Other {
                    continue;
                }
                let read = |file: &str| {
                    std::fs::read_to_string(entry.path().join(file))
                        .map(|text| text.trim().to_owned())
                        .unwrap_or_default()
                };
                // `carrier` cannot be read while the interface is down; that is just "".
                let operstate = read("operstate");
                let carrier = read("carrier");
                links.push((name, operstate, carrier));
            }
        }
        links.sort();

        let routes = std::fs::read_to_string("/proc/net/route").unwrap_or_default();
        Self {
            links,
            routes: route_signature(&routes),
            source_ip: udp_local_ip(),
        }
    }
}

/// Reduce `/proc/net/route` to the columns that describe the topology. The `RefCnt` and `Use`
/// counters are dropped because they can move on their own and would look like a change.
fn route_signature(text: &str) -> String {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            // Iface Destination Gateway Flags RefCnt Use Metric Mask ...
            (fields.len() >= 8)
                .then(|| [fields[0], fields[1], fields[2], fields[3], fields[6], fields[7]].join(" "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Probe (layer 2)
// ---------------------------------------------------------------------------

/// One full look at the network. The public IP is filled in separately by the monitor loop.
fn probe() -> Snapshot {
    let interfaces = read_interfaces();
    let local_ip = primary_local_ip(&interfaces);
    let gateway_ip = gateway_ip();
    let provider_gateway_ip = provider_gateway_ip();
    let desktop_up = local_ip.is_some();
    // Ask about the internet first: if it answers, the router in between is working by
    // definition, so while everything is healthy the gateway is never probed at all.
    let internet_up = desktop_up && internet_reachable();
    let router_up = internet_up || gateway_ip.as_deref().is_some_and(router_reachable);
    let provider_gateway_up = (router_up || internet_up) && provider_gateway_ip.as_deref().is_some_and(|ip| ping(ip) || [80u16, 443, 53].into_iter().any(|port| tcp_open(ip, port, ROUTER_PORT_TIMEOUT)));

    Snapshot {
        local_ip,
        gateway_ip,
        provider_gateway_ip,
        public_ip: None,
        desktop_up,
        router_up,
        provider_gateway_up,
        internet_up,
        interfaces,
        finished_at: Instant::now(),
    }
}

// ---------------------------------------------------------------------------
// Local interfaces
// ---------------------------------------------------------------------------

fn read_interfaces() -> Vec<Interface> {
    let Some(links) = run("ip", &["-o", "link"]) else {
        return Vec::new();
    };
    let addresses = run("ip", &["-o", "-f", "inet", "addr", "show"]).unwrap_or_default();
    interfaces_from_ip(&links, &addresses)
}

fn interfaces_from_ip(links: &str, addresses: &str) -> Vec<Interface> {
    let addresses = parse_addresses(addresses);
    parse_links(links)
        .into_iter()
        .filter(|row| row.name != "lo")
        .map(|row| Interface {
            kind: classify(&row.name),
            addresses: addresses
                .iter()
                .filter(|(name, _)| name == &row.name)
                .map(|(_, ip)| ip.clone())
                .collect(),
            name: row.name,
            up: row.up,
            lower_up: row.lower_up,
        })
        .collect()
}

#[derive(Debug)]
struct LinkRow {
    name: String,
    up: bool,
    lower_up: bool,
}

fn parse_links(text: &str) -> Vec<LinkRow> {
    text.lines().filter_map(parse_link_line).collect()
}

fn parse_link_line(line: &str) -> Option<LinkRow> {
    // `2: enp3s0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 ...`
    let (_, rest) = line.split_once(": ")?;
    let (name, rest) = rest.split_once(": ")?;
    let open = rest.find('<')?;
    let close = rest[open..].find('>')? + open;
    let flags = &rest[open + 1..close];
    Some(LinkRow {
        name: name.split('@').next().unwrap_or(name).to_owned(),
        up: has_flag(flags, "UP"),
        lower_up: has_flag(flags, "LOWER_UP"),
    })
}

fn has_flag(flags: &str, wanted: &str) -> bool {
    flags.split(',').any(|flag| flag == wanted)
}

fn parse_addresses(text: &str) -> Vec<(String, String)> {
    // `2: enp3s0    inet 192.168.1.5/24 brd 192.168.1.255 scope global ...`
    let mut parsed = Vec::new();
    for line in text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(name) = tokens.get(1) else { continue };
        let Some(index) = tokens.iter().position(|token| *token == "inet") else {
            continue;
        };
        let Some(cidr) = tokens.get(index + 1) else {
            continue;
        };
        let address = cidr.split('/').next().unwrap_or(cidr);
        if address.parse::<Ipv4Addr>().is_ok() {
            parsed.push((name.trim_end_matches(':').to_owned(), address.to_owned()));
        }
    }
    parsed
}

/// Classify by interface name prefix. Prefixes only: a substring match would mistake Docker's
/// `veth...` pairs for Ethernet.
pub fn classify(name: &str) -> LinkKind {
    if name.starts_with("wl") {
        LinkKind::Wifi
    } else if name.starts_with("en") || name.starts_with("eth") {
        LinkKind::Ethernet
    } else {
        LinkKind::Other
    }
}

fn primary_local_ip(interfaces: &[Interface]) -> Option<String> {
    udp_local_ip().or_else(|| {
        interfaces
            .iter()
            .find(|interface| interface.has_ipv4())
            .and_then(|interface| interface.addresses.first().cloned())
    })
}

/// Ask the routing table which source address it would use. No packet is sent.
fn udp_local_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then(|| ip.to_string())
}

// ---------------------------------------------------------------------------
// Gateway and reachability
// ---------------------------------------------------------------------------

fn gateway_ip() -> Option<String> {
    if let Some(route) = run("ip", &["route", "show", "default"])
        && let Some(gateway) = parse_default_gateway(&route)
    {
        return Some(gateway);
    }
    // Offline-friendly fallback that needs no external program.
    let route = std::fs::read_to_string("/proc/net/route").ok()?;
    parse_proc_net_route(&route)
}

fn provider_gateway_ip() -> Option<String> {
    // Use traceroute to find the first hop beyond the local gateway
    // -n: numeric output, -m 3: max 3 hops (in case hop 1 times out), -q 1: 1 probe, -w 1: 1s timeout
    let output = run("traceroute", &["-n", "-m", "3", "-q", "1", "-w", "1", "8.8.8.8"])?;
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() < 2 {
        return None;
    }

    // Skip the header line (index 0), then find the first hop that's not "*" and not the local gateway
    let local_gw = gateway_ip().unwrap_or_default();

    for line in lines.iter().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let ip = parts[1];
            if ip != "*" && !ip.starts_with('?') && ip.parse::<IpAddr>().is_ok() {
                if ip != local_gw {
                    return Some(ip.to_owned());
                }
            }
        }
    }
    None
}

pub fn parse_default_gateway(text: &str) -> Option<String> {
    for line in text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.first() != Some(&"default") {
            continue;
        }
        let Some(index) = tokens.iter().position(|token| *token == "via") else {
            continue;
        };
        let Some(gateway) = tokens.get(index + 1) else {
            continue;
        };
        if gateway.parse::<IpAddr>().is_ok() {
            return Some((*gateway).to_owned());
        }
    }
    None
}

pub fn parse_proc_net_route(text: &str) -> Option<String> {
    // Iface Destination Gateway Flags ... ; the gateway is little-endian hex.
    for line in text.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 3 || fields[1] != "00000000" {
            continue;
        }
        let Ok(raw) = u32::from_str_radix(fields[2], 16) else {
            continue;
        };
        if raw != 0 {
            return Some(Ipv4Addr::from(raw.swap_bytes()).to_string());
        }
    }
    None
}

fn router_reachable(gateway: &str) -> bool {
    // A refused connection (RST) is an answer too: the router itself sent it. Only fall back
    // to spawning `ping` when every port stays silent.
    any_tcp_responds(
        &[(gateway, 80), (gateway, 443), (gateway, 53)],
        ROUTER_PORT_TIMEOUT,
        true,
    ) || ping(gateway)
}

fn internet_reachable() -> bool {
    // TCP first: many networks drop ICMP. IP literals avoid DNS, which can hang offline.
    any_tcp_responds(
        &[("1.1.1.1", 443), ("8.8.8.8", 443)],
        TCP_PROBE_TIMEOUT,
        false,
    ) || ping("8.8.8.8")
}

fn ping(host: &str) -> bool {
    Command::new("ping")
        .args(["-n", "-c", "1", "-W", "1", host])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Try several TCP targets at once and return as soon as one responds. Running them in
/// parallel keeps the offline worst case at a single timeout instead of the sum of all of them.
fn any_tcp_responds(targets: &[(&str, u16)], timeout: Duration, refused_counts: bool) -> bool {
    let (sender, receiver) = std::sync::mpsc::channel();
    for &(host, port) in targets {
        let sender = sender.clone();
        let host = host.to_owned();
        std::thread::spawn(move || {
            // The receiver may already be gone if another target answered first.
            let _ = sender.send(tcp_responds(&host, port, timeout, refused_counts));
        });
    }
    drop(sender);
    receiver.iter().any(|responded| responded)
}

fn tcp_responds(host: &str, port: u16, timeout: Duration, refused_counts: bool) -> bool {
    let Ok(addresses) = (host, port).to_socket_addrs() else {
        return false;
    };
    addresses
        .into_iter()
        .any(|address| match TcpStream::connect_timeout(&address, timeout) {
            Ok(_) => true,
            Err(error) => refused_counts && error.kind() == std::io::ErrorKind::ConnectionRefused,
        })
}

fn tcp_open(host: &str, port: u16, timeout: Duration) -> bool {
    let Ok(addresses) = (host, port).to_socket_addrs() else {
        return false;
    };
    addresses
        .into_iter()
        .any(|address| TcpStream::connect_timeout(&address, timeout).is_ok())
}

// ---------------------------------------------------------------------------
// Public IP
// ---------------------------------------------------------------------------

fn fetch_public_ip() -> Option<String> {
    const ENDPOINTS: &[(&str, &str)] = &[
        ("api.ipify.org", "/?format=text"),
        ("checkip.amazonaws.com", "/"),
        ("icanhazip.com", "/"),
    ];
    // Bound the whole fallback chain so a captive portal cannot keep the worker busy.
    let deadline = Instant::now() + Duration::from_secs(4);
    for (host, path) in ENDPOINTS {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if let Some(ip) = http_get_ip(host, path, remaining.min(HTTP_TIMEOUT)) {
            return Some(ip);
        }
    }
    None
}

fn http_get_ip(host: &str, path: &str, timeout: Duration) -> Option<String> {
    let addresses: Vec<_> = (host, 80u16).to_socket_addrs().ok()?.collect();
    for address in addresses {
        let Ok(mut stream) = TcpStream::connect_timeout(&address, timeout) else {
            continue;
        };
        let _ = stream.set_read_timeout(Some(timeout));
        let _ = stream.set_write_timeout(Some(timeout));
        let request = format!(
            "GET {path} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: connection-dashboard/{}\r\nAccept: text/plain\r\nConnection: close\r\n\r\n",
            env!("CARGO_PKG_VERSION")
        );
        if stream.write_all(request.as_bytes()).is_err() {
            continue;
        }
        let mut response = String::new();
        if stream
            .take(32 * 1024)
            .read_to_string(&mut response)
            .is_err()
        {
            continue;
        }
        let body = response
            .split_once("\r\n\r\n")
            .map_or(response.as_str(), |(_, body)| body);
        if let Some(ip) = extract_ip(body) {
            return Some(ip);
        }
    }
    None
}

/// Pull the first IP out of a plain or chunked response body.
pub fn extract_ip(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        line.parse::<IpAddr>()
            .ok()
            .or_else(|| {
                line.split_whitespace()
                    .find_map(|word| word.parse::<IpAddr>().ok())
            })
            .map(|ip| ip.to_string())
    })
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_default_route() {
        let route = "default via 192.168.1.1 dev wlp2s0 proto dhcp metric 600\n\
                     default via 10.0.0.1 dev enp3s0 metric 100\n";
        assert_eq!(parse_default_gateway(route).as_deref(), Some("192.168.1.1"));
        assert_eq!(parse_default_gateway("192.168.1.0/24 dev enp3s0\n"), None);
    }

    #[test]
    fn parses_the_proc_route_fallback() {
        let routes = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                      wlp2s0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n";
        assert_eq!(parse_proc_net_route(routes).as_deref(), Some("192.168.1.1"));
    }

    #[test]
    fn classifies_linux_interface_names() {
        assert_eq!(classify("enp3s0"), LinkKind::Ethernet);
        assert_eq!(classify("eno1"), LinkKind::Ethernet);
        assert_eq!(classify("eth0"), LinkKind::Ethernet);
        assert_eq!(classify("wlp2s0"), LinkKind::Wifi);
        assert_eq!(classify("wlan0"), LinkKind::Wifi);
        assert_eq!(classify("tun0"), LinkKind::Other);
        assert_eq!(classify("lo"), LinkKind::Other);
    }

    #[test]
    fn virtual_interfaces_are_not_ethernet() {
        assert_eq!(classify("veth3f2a1b"), LinkKind::Other);
        assert_eq!(classify("docker0"), LinkKind::Other);
        assert_eq!(classify("virbr0"), LinkKind::Other);
    }

    #[test]
    fn parses_link_and_address_output() {
        let links = "1: lo: <LOOPBACK,UP,LOWER_UP> mtu 65536 qdisc noqueue state UNKNOWN\n\
                     2: enp3s0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc fq_codel state UP\n\
                     3: wlp2s0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc noqueue state UP\n";
        let addresses = "1: lo    inet 127.0.0.1/8 scope host lo\n\
                         2: enp3s0    inet 192.168.1.5/24 brd 192.168.1.255 scope global enp3s0\n\
                         3: wlp2s0    inet 10.0.0.7/24 brd 10.0.0.255 scope global wlp2s0\n";
        let interfaces = interfaces_from_ip(links, addresses);
        assert_eq!(interfaces.len(), 2);
        assert_eq!(interfaces[0].name, "enp3s0");
        assert_eq!(interfaces[0].kind, LinkKind::Ethernet);
        assert_eq!(interfaces[0].addresses, vec!["192.168.1.5"]);
        assert_eq!(interfaces[1].state_text(), "Connected");
    }

    #[test]
    fn ethernet_follows_lower_up_and_wifi_needs_an_address() {
        let ethernet = Interface {
            name: "enp3s0".to_owned(),
            kind: LinkKind::Ethernet,
            up: true,
            lower_up: false,
            addresses: vec![],
        };
        assert!(!ethernet.connected());
        assert_eq!(ethernet.state_text(), "Plugged out");

        let wifi = Interface {
            name: "wlp2s0".to_owned(),
            kind: LinkKind::Wifi,
            up: true,
            lower_up: true,
            addresses: vec![],
        };
        assert!(!wifi.connected());

        let wifi = Interface {
            addresses: vec!["10.0.0.7".to_owned()],
            ..wifi
        };
        assert!(wifi.connected());
        assert_eq!(wifi.state_text(), "Connected");
    }

    #[test]
    fn extracts_ips_from_plain_and_chunked_bodies() {
        assert_eq!(extract_ip("203.0.113.5\n").as_deref(), Some("203.0.113.5"));
        assert_eq!(
            extract_ip("d\r\n203.0.113.5\r\n0\r\n\r\n").as_deref(),
            Some("203.0.113.5")
        );
        assert_eq!(extract_ip("no address here"), None);
    }

    #[test]
    fn route_signature_ignores_counters_but_sees_topology() {
        let header = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n";
        let quiet = format!("{header}wlp2s0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n");
        let busy = format!("{header}wlp2s0\t00000000\t0101A8C0\t0003\t7\t42\t600\t00000000\t0\t0\t0\n");
        let other_gateway =
            format!("{header}wlp2s0\t00000000\t0100000A\t0003\t0\t0\t600\t00000000\t0\t0\t0\n");
        assert_eq!(route_signature(&quiet), route_signature(&busy));
        assert_ne!(route_signature(&quiet), route_signature(&other_gateway));
    }

    #[test]
    fn debounce_needs_two_consecutive_failures() {
        let mut fails = 0;
        // First failure while up: hold the old state.
        assert!(debounce(true, false, &mut fails));
        // Second consecutive failure: now report down.
        assert!(!debounce(true, false, &mut fails));
        // Recovery resets the counter.
        assert!(debounce(false, true, &mut fails));
        assert_eq!(fails, 0);
        // A state that was already down stays down immediately.
        assert!(!debounce(false, false, &mut fails));
    }

    #[test]
    fn offline_probes_back_off_to_a_cap() {
        assert_eq!(offline_delay(0), Duration::from_secs(1));
        assert_eq!(offline_delay(1), Duration::from_secs(2));
        assert_eq!(offline_delay(2), Duration::from_secs(4));
        assert_eq!(offline_delay(3), Duration::from_secs(5));
        assert_eq!(offline_delay(50), Duration::from_secs(5));
    }
}