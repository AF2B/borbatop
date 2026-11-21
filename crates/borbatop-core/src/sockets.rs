//! Open sockets: parsers for `/proc/net/{tcp,tcp6,udp,udp6}` and for the `socket:[inode]`
//! links in `/proc/[pid]/fd`, which together say which process owns which connection.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::identity::Uid;

const HEADER_LINES: usize = 1;
const LOCAL_INDEX: usize = 1;
const REMOTE_INDEX: usize = 2;
const STATE_INDEX: usize = 3;
const UID_INDEX: usize = 7;
const INODE_INDEX: usize = 9;
const MIN_FIELDS: usize = 10;
const ADDRESS_SEPARATOR: char = ':';
const HEX_RADIX: u32 = 16;
const IPV4_HEX_DIGITS: usize = 8;
const IPV6_HEX_DIGITS: usize = 32;
const WORD_HEX_DIGITS: usize = 8;
const SOCKET_LINK_PREFIX: &str = "socket:[";
const SOCKET_LINK_SUFFIX: char = ']';

/// Transport protocol and address family of a socket table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SocketProtocol {
    /// TCP over IPv4 (`/proc/net/tcp`).
    Tcp4,
    /// TCP over IPv6 (`/proc/net/tcp6`).
    Tcp6,
    /// UDP over IPv4 (`/proc/net/udp`).
    Udp4,
    /// UDP over IPv6 (`/proc/net/udp6`).
    Udp6,
}

impl SocketProtocol {
    /// Every table, in the order they are listed.
    pub const ALL: [Self; 4] = [Self::Tcp4, Self::Tcp6, Self::Udp4, Self::Udp6];

    /// Short name for tables on screen.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Tcp4 => "tcp",
            Self::Tcp6 => "tcp6",
            Self::Udp4 => "udp",
            Self::Udp6 => "udp6",
        }
    }

    /// File under `/proc/net` that holds the table.
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Tcp4 => "tcp",
            Self::Tcp6 => "tcp6",
            Self::Udp4 => "udp",
            Self::Udp6 => "udp6",
        }
    }

    const fn is_ipv6(self) -> bool {
        match self {
            Self::Tcp6 | Self::Udp6 => true,
            Self::Tcp4 | Self::Udp4 => false,
        }
    }
}

/// Connection state, the `st` column (the kernel's `TCP_*` states; UDP reuses two of them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketState {
    /// Connection established.
    Established,
    /// Connection request sent.
    SynSent,
    /// Connection request received.
    SynReceived,
    /// First half of an active close.
    FinWait1,
    /// Second half of an active close.
    FinWait2,
    /// Waiting out late packets after closing.
    TimeWait,
    /// Closed; for UDP, a socket with no peer.
    Close,
    /// The peer closed; the local side has not yet.
    CloseWait,
    /// Waiting for the last acknowledgement.
    LastAck,
    /// Waiting for connections.
    Listen,
    /// Both sides closed at once.
    Closing,
    /// Request received and tracked in the SYN queue.
    NewSynReceived,
}

/// A state code that no [`SocketState`] recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownSocketState(pub u8);

impl SocketState {
    /// Parses the two-digit hexadecimal state code.
    ///
    /// # Arguments
    ///
    /// * `code` - the kernel's state number
    ///
    /// # Errors
    ///
    /// Returns [`UnknownSocketState`] when the code is not one of the kernel's states.
    pub const fn from_code(code: u8) -> Result<Self, UnknownSocketState> {
        match code {
            0x01 => Ok(Self::Established),
            0x02 => Ok(Self::SynSent),
            0x03 => Ok(Self::SynReceived),
            0x04 => Ok(Self::FinWait1),
            0x05 => Ok(Self::FinWait2),
            0x06 => Ok(Self::TimeWait),
            0x07 => Ok(Self::Close),
            0x08 => Ok(Self::CloseWait),
            0x09 => Ok(Self::LastAck),
            0x0A => Ok(Self::Listen),
            0x0B => Ok(Self::Closing),
            0x0C => Ok(Self::NewSynReceived),
            unknown => Err(UnknownSocketState(unknown)),
        }
    }

    /// Name as `ss` prints it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Established => "ESTAB",
            Self::SynSent => "SYN-SENT",
            Self::SynReceived => "SYN-RECV",
            Self::FinWait1 => "FIN-WAIT-1",
            Self::FinWait2 => "FIN-WAIT-2",
            Self::TimeWait => "TIME-WAIT",
            Self::Close => "UNCONN",
            Self::CloseWait => "CLOSE-WAIT",
            Self::LastAck => "LAST-ACK",
            Self::Listen => "LISTEN",
            Self::Closing => "CLOSING",
            Self::NewSynReceived => "NEW-SYN-RECV",
        }
    }
}

/// One line of a socket table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketEntry {
    protocol: SocketProtocol,
    local: SocketAddr,
    remote: SocketAddr,
    state: SocketState,
    uid: Uid,
    inode: u64,
}

impl SocketEntry {
    /// Protocol and address family.
    #[must_use]
    pub const fn protocol(&self) -> SocketProtocol {
        self.protocol
    }

    /// Local address and port.
    #[must_use]
    pub const fn local(&self) -> SocketAddr {
        self.local
    }

    /// Remote address and port; all zeros when there is no peer.
    #[must_use]
    pub const fn remote(&self) -> SocketAddr {
        self.remote
    }

    /// Connection state.
    #[must_use]
    pub const fn state(&self) -> SocketState {
        self.state
    }

    /// Owner of the socket, as the kernel records it.
    #[must_use]
    pub const fn uid(&self) -> Uid {
        self.uid
    }

    /// Inode that links the socket to the `socket:[inode]` entries in `/proc/[pid]/fd`.
    #[must_use]
    pub const fn inode(&self) -> u64 {
        self.inode
    }
}

/// The parsed contents of one socket table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketTable {
    entries: Vec<SocketEntry>,
    skipped: usize,
}

impl SocketTable {
    /// The sockets, in file order.
    #[must_use]
    pub fn entries(&self) -> &[SocketEntry] {
        &self.entries
    }

    /// Lines that were not understood, counted instead of dropped silently.
    #[must_use]
    pub const fn skipped(&self) -> usize {
        self.skipped
    }
}

/// Parses one of `/proc/net/{tcp,tcp6,udp,udp6}`.
///
/// # Arguments
///
/// * `text` - full contents of the file, header line included
/// * `protocol` - which table the text came from; decides the address width
///
/// # Examples
///
/// ```
/// use borbatop_core::{SocketProtocol, SocketState, parse_socket_table};
///
/// let text = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4242 1 0000000000000000 100 0 0 10 0\n";
/// let table = parse_socket_table(text, SocketProtocol::Tcp4);
/// let entry = table.entries().first().ok_or("no entry")?;
/// assert_eq!(entry.local().to_string(), "127.0.0.1:8080");
/// assert_eq!(entry.state(), SocketState::Listen);
/// assert_eq!(entry.inode(), 4242);
/// # Ok::<(), &str>(())
/// ```
#[must_use]
pub fn parse_socket_table(text: &str, protocol: SocketProtocol) -> SocketTable {
    let mut entries = Vec::new();
    let mut skipped = 0_usize;
    for line in text.lines().skip(HEADER_LINES) {
        if line.trim().is_empty() {
            continue;
        }
        match parse_line(line, protocol) {
            Some(entry) => entries.push(entry),
            None => skipped = skipped.saturating_add(1),
        }
    }
    SocketTable { entries, skipped }
}

fn parse_line(line: &str, protocol: SocketProtocol) -> Option<SocketEntry> {
    let fields: Vec<&str> = line.split_ascii_whitespace().collect();
    if fields.len() < MIN_FIELDS {
        return None;
    }
    let state_code = u8::from_str_radix(fields.get(STATE_INDEX)?, HEX_RADIX).ok()?;
    Some(SocketEntry {
        protocol,
        local: parse_address(fields.get(LOCAL_INDEX)?, protocol)?,
        remote: parse_address(fields.get(REMOTE_INDEX)?, protocol)?,
        state: SocketState::from_code(state_code).ok()?,
        uid: Uid::new(fields.get(UID_INDEX)?.parse().ok()?),
        inode: fields.get(INODE_INDEX)?.parse().ok()?,
    })
}

/// `ADDRESS:PORT` with the address as the kernel prints it: each 32-bit word in host byte order.
fn parse_address(token: &str, protocol: SocketProtocol) -> Option<SocketAddr> {
    let (address, port) = token.split_once(ADDRESS_SEPARATOR)?;
    let port = u16::from_str_radix(port, HEX_RADIX).ok()?;
    let ip = if protocol.is_ipv6() {
        IpAddr::V6(parse_ipv6(address)?)
    } else {
        IpAddr::V4(parse_ipv4(address)?)
    };
    Some(SocketAddr::new(ip, port))
}

fn parse_ipv4(hex: &str) -> Option<Ipv4Addr> {
    if hex.len() != IPV4_HEX_DIGITS {
        return None;
    }
    let word = u32::from_str_radix(hex, HEX_RADIX).ok()?;
    Some(Ipv4Addr::from(word.to_ne_bytes()))
}

fn parse_ipv6(hex: &str) -> Option<Ipv6Addr> {
    if hex.len() != IPV6_HEX_DIGITS {
        return None;
    }
    let mut bytes = Vec::with_capacity(IPV6_HEX_DIGITS / 2);
    for index in (0..IPV6_HEX_DIGITS).step_by(WORD_HEX_DIGITS) {
        let word = hex.get(index..index.checked_add(WORD_HEX_DIGITS)?)?;
        bytes.extend_from_slice(&u32::from_str_radix(word, HEX_RADIX).ok()?.to_ne_bytes());
    }
    let octets: [u8; 16] = bytes.try_into().ok()?;
    Some(Ipv6Addr::from(octets))
}

/// The inode in a `/proc/[pid]/fd` link such as `socket:[12345]`.
///
/// # Arguments
///
/// * `link` - target of the symbolic link
///
/// # Returns
///
/// `None` when the link is not a socket (a file, a pipe, an anonymous inode).
#[must_use]
pub fn parse_socket_link(link: &str) -> Option<u64> {
    link.strip_prefix(SOCKET_LINK_PREFIX)?
        .strip_suffix(SOCKET_LINK_SUFFIX)?
        .parse()
        .ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const HEADER: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n";

    fn table(protocol: SocketProtocol, rows: &str) -> SocketTable {
        parse_socket_table(&format!("{HEADER}{rows}"), protocol)
    }

    #[test]
    fn an_ipv4_listener_and_an_established_connection() {
        let parsed = table(
            SocketProtocol::Tcp4,
            "   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4242 1 0 100 0 0 10 0\n   1: 0A00A8C0:C350 0101A8C0:01BB 01 00000000:00000000 02:000A7214 00000000     0        0 5151 2 0 21 4 0 -1\n",
        );

        let [listener, established] = parsed.entries() else {
            return assert_eq!(parsed.entries().len(), 2);
        };
        assert_eq!(listener.local().to_string(), "127.0.0.1:8080");
        assert_eq!(listener.remote().to_string(), "0.0.0.0:0");
        assert_eq!(listener.state(), SocketState::Listen);
        assert_eq!(listener.uid().get(), 1000);
        assert_eq!(established.local().to_string(), "192.168.0.10:50000");
        assert_eq!(established.remote().to_string(), "192.168.1.1:443");
        assert_eq!(established.state(), SocketState::Established);
        assert_eq!(established.inode(), 5151);
        assert_eq!(parsed.skipped(), 0);
    }

    #[test]
    fn ipv6_words_keep_their_order() {
        let loopback = format!("{}:0050", "00000000000000000000000001000000");
        let row = format!(
            "   0: {loopback} {} 0A 00000000:00000000 00:00000000 00000000     0        0 77 1 0 100 0 0 10 0\n",
            "00000000000000000000000000000000:0000"
        );
        let parsed = table(SocketProtocol::Tcp6, &row);
        assert_eq!(
            parsed.entries().first().unwrap().local().to_string(),
            "[::1]:80"
        );
    }

    #[test]
    fn udp_sockets_without_a_peer_read_as_unconnected() {
        let parsed = table(
            SocketProtocol::Udp4,
            "  12: 00000000:0044 00000000:0000 07 00000000:00000000 00:00000000 00000000     0        0 900 2 0\n",
        );
        let entry = parsed.entries().first().unwrap();
        assert_eq!(entry.state().name(), "UNCONN");
        assert_eq!(entry.protocol().label(), "udp");
    }

    #[test]
    fn malformed_lines_are_counted_not_dropped_silently() {
        let parsed = table(
            SocketProtocol::Tcp4,
            "garbage\n   0: ZZZZZZZZ:1F90 00000000:0000 0A 0:0 0:0 0 0 0 4242\n   1: 0100007F:1F90 00000000:0000 FF 0:0 0:0 0 0 0 4242\n\n",
        );
        assert_eq!(parsed.entries(), []);
        assert_eq!(parsed.skipped(), 3);
    }

    #[test]
    fn state_codes_round_trip_through_names() {
        for code in 1..=0x0C {
            let state = SocketState::from_code(code).unwrap();
            assert_ne!(state.name(), "");
        }
        assert_eq!(SocketState::from_code(0), Err(UnknownSocketState(0)));
        assert_eq!(SocketState::from_code(0x0D), Err(UnknownSocketState(0x0D)));
    }

    #[test]
    fn socket_links_yield_inodes_and_other_links_do_not() {
        assert_eq!(parse_socket_link("socket:[12345]"), Some(12_345));
        assert_eq!(parse_socket_link("pipe:[12345]"), None);
        assert_eq!(parse_socket_link("/home/a/file"), None);
        assert_eq!(parse_socket_link("socket:[x]"), None);
        assert_eq!(parse_socket_link("socket:[12"), None);
        assert_eq!(parse_socket_link("anon_inode:[eventpoll]"), None);
    }

    #[test]
    fn every_protocol_has_a_table_file() {
        let names: Vec<&str> = SocketProtocol::ALL.iter().map(|p| p.file_name()).collect();
        assert_eq!(names, ["tcp", "tcp6", "udp", "udp6"]);
    }
}
