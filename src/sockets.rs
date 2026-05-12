//! W7: WASI P2 Sockets — `wasi:sockets/tcp`, `wasi:sockets/udp`, `wasi:sockets/ip-name-lookup`.
//!
//! Implements WASI Preview 2 socket interfaces (simulated in-memory for testing):
//! - TCP types: TcpSocket, IpAddress, SocketAddr, Network (W7.1)
//! - TCP connect: `start_connect()` / `finish_connect()` state machine (W7.2)
//! - TCP listen & accept: `start_listen()` / `accept()` (W7.3)
//! - TCP streams: input/output streams from connected sockets (W7.4)
//! - UDP: `UdpSocket` with `send_to()` / `receive_from()` datagram API (W7.5)
//! - DNS lookup: `resolve()` hostname to IP addresses (W7.6)
//! - Socket options: `SO_REUSEADDR`, `TCP_NODELAY`, timeouts (W7.7)
//! - Non-blocking I/O: `is_readable()` / `is_writable()` for pollable sockets (W7.8)
//! - Socket errors: `SocketError` enum with typed error variants (W7.9)

use std::collections::{HashMap, VecDeque};
use std::fmt;

// ═══════════════════════════════════════════════════════════════════════
// W7.1: TCP Types
// ═══════════════════════════════════════════════════════════════════════

/// Handle for a TCP socket in the socket table.
pub type TcpSocketHandle = u32;

/// Handle for a UDP socket in the socket table.
pub type UdpSocketHandle = u32;

/// Handle for a network resource.
pub type NetworkHandle = u32;

/// An IP address (V4 or V6).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IpAddress {
    /// IPv4 address as 4 octets.
    V4(u8, u8, u8, u8),
    /// IPv6 address as 8 groups of 16-bit values.
    V6(u16, u16, u16, u16, u16, u16, u16, u16),
}

impl IpAddress {
    /// Creates the IPv4 loopback address (127.0.0.1).
    pub fn localhost_v4() -> Self {
        IpAddress::V4(127, 0, 0, 1)
    }

    /// Creates the IPv6 loopback address (::1).
    pub fn localhost_v6() -> Self {
        IpAddress::V6(0, 0, 0, 0, 0, 0, 0, 1)
    }

    /// Creates the IPv4 unspecified address (0.0.0.0).
    pub fn unspecified_v4() -> Self {
        IpAddress::V4(0, 0, 0, 0)
    }

    /// Returns true if this is an IPv4 address.
    pub fn is_v4(&self) -> bool {
        matches!(self, IpAddress::V4(..))
    }

    /// Returns true if this is an IPv6 address.
    pub fn is_v6(&self) -> bool {
        matches!(self, IpAddress::V6(..))
    }
}

impl fmt::Display for IpAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IpAddress::V4(a, b, c, d) => write!(f, "{a}.{b}.{c}.{d}"),
            IpAddress::V6(a, b, c, d, e, g, h, i) => {
                write!(f, "{a:x}:{b:x}:{c:x}:{d:x}:{e:x}:{g:x}:{h:x}:{i:x}")
            }
        }
    }
}

/// A socket address: IP + port.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SocketAddr {
    /// The IP address.
    pub ip: IpAddress,
    /// The port number.
    pub port: u16,
}

impl SocketAddr {
    /// Creates a new socket address.
    pub fn new(ip: IpAddress, port: u16) -> Self {
        Self { ip, port }
    }
}

impl fmt::Display for SocketAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.ip {
            IpAddress::V4(..) => write!(f, "{}:{}", self.ip, self.port),
            IpAddress::V6(..) => write!(f, "[{}]:{}", self.ip, self.port),
        }
    }
}

/// A network resource (WASI `wasi:sockets/network`).
///
/// Represents the network capability granted to a WASI component.
#[derive(Debug)]
pub struct Network {
    /// Unique network handle.
    pub handle: NetworkHandle,
    /// Human-readable name for this network.
    pub name: String,
}

impl Network {
    /// Creates a new network resource with the given handle and name.
    pub fn new(handle: NetworkHandle, name: &str) -> Self {
        Self {
            handle,
            name: name.to_string(),
        }
    }
}

/// TCP socket state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpState {
    /// Socket created but not connected or listening.
    Closed,
    /// `start_connect()` called, awaiting `finish_connect()`.
    Connecting,
    /// Connection established (either client or accepted server socket).
    Connected,
    /// `start_listen()` called, socket is accepting connections.
    Listening,
}

impl fmt::Display for TcpState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => write!(f, "closed"),
            Self::Connecting => write!(f, "connecting"),
            Self::Connected => write!(f, "connected"),
            Self::Listening => write!(f, "listening"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W7.7: Socket Options
// ═══════════════════════════════════════════════════════════════════════

/// Configuration options for a socket.
#[derive(Debug, Clone, Default)]
pub struct SocketOptions {
    /// `SO_REUSEADDR`: allow binding to an address already in use.
    pub reuse_address: bool,
    /// `TCP_NODELAY`: disable Nagle's algorithm.
    pub tcp_nodelay: bool,
    /// Send timeout in milliseconds (0 = no timeout).
    pub send_timeout_ms: u64,
    /// Receive timeout in milliseconds (0 = no timeout).
    pub recv_timeout_ms: u64,
    /// Keep-alive interval in seconds (0 = disabled).
    pub keep_alive_secs: u64,
}

// ═══════════════════════════════════════════════════════════════════════
// W7.9: Socket Errors
// ═══════════════════════════════════════════════════════════════════════

/// Socket error type covering all networking failure modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketError {
    /// The remote host refused the connection.
    ConnectionRefused,
    /// The operation timed out.
    Timeout,
    /// The connection was reset by the remote host.
    ConnectionReset,
    /// The address is already in use.
    AddressInUse,
    /// The address is not available on this host.
    AddressNotAvailable,
    /// The socket is not connected.
    NotConnected,
    /// The socket is already connected.
    AlreadyConnected,
    /// The socket is already listening.
    AlreadyListening,
    /// The operation is invalid for the current socket state.
    InvalidState(String),
    /// DNS resolution failed.
    NameResolutionFailed(String),
    /// The socket handle is invalid.
    BadHandle,
    /// A generic I/O error.
    Io(String),
    /// The operation would block on a non-blocking socket.
    WouldBlock,
    /// Connection was aborted.
    ConnectionAborted,
    /// Network is unreachable.
    NetworkUnreachable,
}

impl fmt::Display for SocketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConnectionRefused => write!(f, "connection refused"),
            Self::Timeout => write!(f, "operation timed out"),
            Self::ConnectionReset => write!(f, "connection reset"),
            Self::AddressInUse => write!(f, "address already in use"),
            Self::AddressNotAvailable => write!(f, "address not available"),
            Self::NotConnected => write!(f, "socket not connected"),
            Self::AlreadyConnected => write!(f, "socket already connected"),
            Self::AlreadyListening => write!(f, "socket already listening"),
            Self::InvalidState(msg) => write!(f, "invalid state: {msg}"),
            Self::NameResolutionFailed(host) => {
                write!(f, "name resolution failed for: {host}")
            }
            Self::BadHandle => write!(f, "bad socket handle"),
            Self::Io(msg) => write!(f, "I/O error: {msg}"),
            Self::WouldBlock => write!(f, "operation would block"),
            Self::ConnectionAborted => write!(f, "connection aborted"),
            Self::NetworkUnreachable => write!(f, "network unreachable"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W7.1–W7.4: TCP Socket
// ═══════════════════════════════════════════════════════════════════════

/// A simulated TCP socket with full state machine.
#[derive(Debug)]
pub struct TcpSocket {
    /// Current state of the socket.
    state: TcpState,
    /// Local address this socket is bound to.
    local_addr: Option<SocketAddr>,
    /// Remote address this socket is connected to.
    remote_addr: Option<SocketAddr>,
    /// Socket options.
    options: SocketOptions,
    /// Receive buffer (data received from the remote end).
    recv_buffer: VecDeque<u8>,
    /// Send buffer (data waiting to be sent).
    send_buffer: VecDeque<u8>,
    /// Pending connections for a listening socket.
    accept_queue: VecDeque<TcpSocketHandle>,
    /// Total bytes received.
    bytes_received: u64,
    /// Total bytes sent.
    bytes_sent: u64,
}

impl TcpSocket {
    /// Creates a new TCP socket in the Closed state.
    pub fn new() -> Self {
        Self {
            state: TcpState::Closed,
            local_addr: None,
            remote_addr: None,
            options: SocketOptions::default(),
            recv_buffer: VecDeque::new(),
            send_buffer: VecDeque::new(),
            accept_queue: VecDeque::new(),
            bytes_received: 0,
            bytes_sent: 0,
        }
    }

    /// Returns the current socket state.
    pub fn state(&self) -> TcpState {
        self.state
    }

    /// Returns the local address, if bound.
    pub fn local_addr(&self) -> Option<&SocketAddr> {
        self.local_addr.as_ref()
    }

    /// Returns the remote address, if connected.
    pub fn remote_addr(&self) -> Option<&SocketAddr> {
        self.remote_addr.as_ref()
    }

    /// Returns a reference to the socket options.
    pub fn options(&self) -> &SocketOptions {
        &self.options
    }

    /// Returns a mutable reference to the socket options.
    pub fn options_mut(&mut self) -> &mut SocketOptions {
        &mut self.options
    }

    /// Returns total bytes received over this socket's lifetime.
    pub fn bytes_received(&self) -> u64 {
        self.bytes_received
    }

    /// Returns total bytes sent over this socket's lifetime.
    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent
    }
}

impl Default for TcpSocket {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W7.5: UDP Socket
// ═══════════════════════════════════════════════════════════════════════

/// A received UDP datagram.
#[derive(Debug, Clone, PartialEq)]
pub struct Datagram {
    /// The payload bytes.
    pub data: Vec<u8>,
    /// The source address of the datagram.
    pub remote_addr: SocketAddr,
}

/// A simulated UDP socket.
#[derive(Debug)]
pub struct UdpSocket {
    /// Local address this socket is bound to.
    local_addr: Option<SocketAddr>,
    /// Socket options.
    options: SocketOptions,
    /// Incoming datagram queue.
    recv_queue: VecDeque<Datagram>,
    /// Outgoing datagram log (for testing / inspection).
    sent_datagrams: Vec<Datagram>,
    /// Total bytes received.
    bytes_received: u64,
    /// Total bytes sent.
    bytes_sent: u64,
}

impl UdpSocket {
    /// Creates a new UDP socket.
    pub fn new() -> Self {
        Self {
            local_addr: None,
            options: SocketOptions::default(),
            recv_queue: VecDeque::new(),
            sent_datagrams: Vec::new(),
            bytes_received: 0,
            bytes_sent: 0,
        }
    }

    /// Returns the local address, if bound.
    pub fn local_addr(&self) -> Option<&SocketAddr> {
        self.local_addr.as_ref()
    }

    /// Returns a reference to the socket options.
    pub fn options(&self) -> &SocketOptions {
        &self.options
    }

    /// Returns a mutable reference to the socket options.
    pub fn options_mut(&mut self) -> &mut SocketOptions {
        &mut self.options
    }

    /// Returns all sent datagrams (for test inspection).
    pub fn sent_datagrams(&self) -> &[Datagram] {
        &self.sent_datagrams
    }

    /// Returns total bytes received.
    pub fn bytes_received(&self) -> u64 {
        self.bytes_received
    }

    /// Returns total bytes sent.
    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent
    }
}

impl Default for UdpSocket {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W7.6: DNS Lookup Table
// ═══════════════════════════════════════════════════════════════════════

/// A simulated DNS resolver.
#[derive(Debug, Default)]
pub struct DnsResolver {
    /// Hostname -> list of IP addresses.
    records: HashMap<String, Vec<IpAddress>>,
}

impl DnsResolver {
    /// Creates a new empty DNS resolver.
    pub fn new() -> Self {
        let mut resolver = Self {
            records: HashMap::new(),
        };
        // Seed with common loopback entries.
        resolver.add_record("localhost", IpAddress::localhost_v4());
        resolver.add_record("localhost", IpAddress::localhost_v6());
        resolver
    }

    /// Adds a DNS record mapping a hostname to an IP address.
    pub fn add_record(&mut self, hostname: &str, ip: IpAddress) {
        self.records
            .entry(hostname.to_string())
            .or_default()
            .push(ip);
    }

    /// Resolves a hostname to a list of IP addresses.
    pub fn resolve(&self, hostname: &str) -> Result<Vec<IpAddress>, SocketError> {
        self.records
            .get(hostname)
            .cloned()
            .filter(|addrs| !addrs.is_empty())
            .ok_or_else(|| SocketError::NameResolutionFailed(hostname.to_string()))
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Socket Table — Central Resource Manager
// ═══════════════════════════════════════════════════════════════════════

/// Central table tracking all open sockets (TCP and UDP).
///
/// Analogous to `StreamTable` in `wasi_p2::streams` and the descriptor table
/// in `wasi_p2::filesystem`. Manages socket handles, connection state, and
/// provides the simulated networking fabric for in-memory testing.
#[derive(Debug)]
pub struct SocketTable {
    /// Open TCP sockets.
    pub tcp_sockets: HashMap<TcpSocketHandle, TcpSocket>,
    /// Open UDP sockets.
    pub udp_sockets: HashMap<UdpSocketHandle, UdpSocket>,
    /// Addresses currently bound by listening TCP sockets.
    bound_tcp: HashMap<SocketAddr, TcpSocketHandle>,
    /// Addresses currently bound by UDP sockets.
    bound_udp: HashMap<SocketAddr, UdpSocketHandle>,
    /// DNS resolver for hostname lookups.
    pub dns: DnsResolver,
    /// Next handle ID for allocation.
    next_handle: u32,
    /// Next ephemeral port for auto-binding.
    next_ephemeral_port: u16,
}

impl SocketTable {
    /// Creates a new socket table with default DNS entries.
    pub fn new() -> Self {
        Self {
            tcp_sockets: HashMap::new(),
            udp_sockets: HashMap::new(),
            bound_tcp: HashMap::new(),
            bound_udp: HashMap::new(),
            dns: DnsResolver::new(),
            next_handle: 1,
            next_ephemeral_port: 49152,
        }
    }

    fn alloc_handle(&mut self) -> u32 {
        let h = self.next_handle;
        self.next_handle += 1;
        h
    }

    // ── TCP operations ──

    /// Creates a new TCP socket and returns its handle.
    pub fn tcp_create(&mut self) -> TcpSocketHandle {
        let handle = self.alloc_handle();
        self.tcp_sockets.insert(handle, TcpSocket::new());
        handle
    }

    /// Binds a TCP socket to a local address.
    pub fn tcp_bind(
        &mut self,
        handle: TcpSocketHandle,
        addr: SocketAddr,
    ) -> Result<(), SocketError> {
        let sock = self
            .tcp_sockets
            .get_mut(&handle)
            .ok_or(SocketError::BadHandle)?;

        if sock.state != TcpState::Closed {
            return Err(SocketError::InvalidState(format!(
                "cannot bind in state: {}",
                sock.state
            )));
        }

        // Check for address reuse.
        if self.bound_tcp.contains_key(&addr) && !sock.options.reuse_address {
            return Err(SocketError::AddressInUse);
        }

        sock.local_addr = Some(addr);
        Ok(())
    }

    // ── W7.2: TCP connect ──

    /// Begins a TCP connection to the given remote address.
    ///
    /// Transitions the socket from Closed to Connecting. Call `tcp_finish_connect()`
    /// to complete the handshake.
    pub fn tcp_start_connect(
        &mut self,
        handle: TcpSocketHandle,
        remote: SocketAddr,
    ) -> Result<(), SocketError> {
        let sock = self
            .tcp_sockets
            .get_mut(&handle)
            .ok_or(SocketError::BadHandle)?;

        if sock.state != TcpState::Closed {
            return Err(SocketError::InvalidState(format!(
                "cannot connect in state: {}",
                sock.state
            )));
        }

        // Auto-bind to an ephemeral port if not already bound.
        if sock.local_addr.is_none() {
            let port = self.next_ephemeral_port;
            self.next_ephemeral_port += 1;
            sock.local_addr = Some(SocketAddr::new(IpAddress::localhost_v4(), port));
        }

        sock.remote_addr = Some(remote);
        sock.state = TcpState::Connecting;
        Ok(())
    }

    /// Completes a TCP connection initiated by `tcp_start_connect()`.
    ///
    /// In this simulation, the connection succeeds if there is a listening socket
    /// bound to the target address. If no listener exists, returns `ConnectionRefused`.
    pub fn tcp_finish_connect(&mut self, handle: TcpSocketHandle) -> Result<(), SocketError> {
        // Validate state first.
        let remote = {
            let sock = self
                .tcp_sockets
                .get(&handle)
                .ok_or(SocketError::BadHandle)?;

            if sock.state != TcpState::Connecting {
                return Err(SocketError::InvalidState(format!(
                    "cannot finish connect in state: {}",
                    sock.state
                )));
            }
            sock.remote_addr.clone()
        };

        let remote = remote.ok_or(SocketError::NotConnected)?;

        // Check if a listener exists on the remote address.
        let listener_handle = self.bound_tcp.get(&remote).copied();

        match listener_handle {
            Some(listener_h) => {
                // Transition client to Connected.
                if let Some(sock) = self.tcp_sockets.get_mut(&handle) {
                    sock.state = TcpState::Connected;
                }
                // Enqueue this client handle in the listener's accept queue.
                if let Some(listener) = self.tcp_sockets.get_mut(&listener_h) {
                    listener.accept_queue.push_back(handle);
                }
                Ok(())
            }
            None => {
                // No listener — reset back to Closed.
                if let Some(sock) = self.tcp_sockets.get_mut(&handle) {
                    sock.state = TcpState::Closed;
                    sock.remote_addr = None;
                }
                Err(SocketError::ConnectionRefused)
            }
        }
    }

    // ── W7.3: TCP listen & accept ──

    /// Begins listening for incoming TCP connections on the bound address.
    pub fn tcp_start_listen(&mut self, handle: TcpSocketHandle) -> Result<(), SocketError> {
        let sock = self
            .tcp_sockets
            .get_mut(&handle)
            .ok_or(SocketError::BadHandle)?;

        if sock.state != TcpState::Closed {
            return Err(SocketError::InvalidState(format!(
                "cannot listen in state: {}",
                sock.state
            )));
        }

        let addr = sock.local_addr.clone().ok_or(SocketError::InvalidState(
            "socket must be bound before listening".to_string(),
        ))?;

        // Check if the address is already in use by another listener.
        if let Some(&existing) = self.bound_tcp.get(&addr) {
            if existing != handle {
                return Err(SocketError::AddressInUse);
            }
        }

        sock.state = TcpState::Listening;
        self.bound_tcp.insert(addr, handle);
        Ok(())
    }

    /// Accepts an incoming connection from a listening socket.
    ///
    /// Returns a new handle for the server-side peer socket. The original
    /// client socket (in the accept queue) and the returned peer socket form
    /// a connected pair.
    pub fn tcp_accept(
        &mut self,
        handle: TcpSocketHandle,
    ) -> Result<(TcpSocketHandle, TcpSocketHandle), SocketError> {
        let sock = self
            .tcp_sockets
            .get_mut(&handle)
            .ok_or(SocketError::BadHandle)?;

        if sock.state != TcpState::Listening {
            return Err(SocketError::InvalidState(format!(
                "cannot accept in state: {}",
                sock.state
            )));
        }

        let client_handle = sock
            .accept_queue
            .pop_front()
            .ok_or(SocketError::WouldBlock)?;

        // Determine the addresses for the server-side peer.
        let client_local = self
            .tcp_sockets
            .get(&client_handle)
            .and_then(|s| s.local_addr.clone());
        let server_local = self
            .tcp_sockets
            .get(&handle)
            .and_then(|s| s.local_addr.clone());

        // Create a new server-side peer socket in the Connected state.
        let peer_handle = self.alloc_handle();
        let mut peer = TcpSocket::new();
        peer.state = TcpState::Connected;
        peer.local_addr = server_local;
        peer.remote_addr = client_local;
        self.tcp_sockets.insert(peer_handle, peer);

        Ok((client_handle, peer_handle))
    }

    // ── W7.4: TCP streams (send/receive) ──

    /// Sends data over a connected TCP socket.
    pub fn tcp_send(&mut self, handle: TcpSocketHandle, data: &[u8]) -> Result<u64, SocketError> {
        let sock = self
            .tcp_sockets
            .get_mut(&handle)
            .ok_or(SocketError::BadHandle)?;

        if sock.state != TcpState::Connected {
            return Err(SocketError::NotConnected);
        }

        sock.send_buffer.extend(data);
        sock.bytes_sent += data.len() as u64;
        Ok(data.len() as u64)
    }

    /// Receives data from a connected TCP socket.
    ///
    /// Reads up to `max_len` bytes from the receive buffer.
    pub fn tcp_recv(
        &mut self,
        handle: TcpSocketHandle,
        max_len: usize,
    ) -> Result<Vec<u8>, SocketError> {
        let sock = self
            .tcp_sockets
            .get_mut(&handle)
            .ok_or(SocketError::BadHandle)?;

        if sock.state != TcpState::Connected {
            return Err(SocketError::NotConnected);
        }

        if sock.recv_buffer.is_empty() {
            return Err(SocketError::WouldBlock);
        }

        let n = max_len.min(sock.recv_buffer.len());
        let data: Vec<u8> = sock.recv_buffer.drain(..n).collect();
        sock.bytes_received += data.len() as u64;
        Ok(data)
    }

    /// Delivers data into a socket's receive buffer (simulates network arrival).
    ///
    /// This is used by the test harness to simulate data arriving over the network.
    pub fn tcp_deliver(&mut self, handle: TcpSocketHandle, data: &[u8]) -> Result<(), SocketError> {
        let sock = self
            .tcp_sockets
            .get_mut(&handle)
            .ok_or(SocketError::BadHandle)?;

        sock.recv_buffer.extend(data);
        Ok(())
    }

    /// Closes a TCP socket and releases its bound address.
    pub fn tcp_close(&mut self, handle: TcpSocketHandle) -> Result<(), SocketError> {
        let sock = self
            .tcp_sockets
            .remove(&handle)
            .ok_or(SocketError::BadHandle)?;

        // Release the bound address if this was a listener.
        if let Some(addr) = &sock.local_addr {
            if let Some(&bound_handle) = self.bound_tcp.get(addr) {
                if bound_handle == handle {
                    self.bound_tcp.remove(addr);
                }
            }
        }

        Ok(())
    }

    // ── W7.5: UDP ──

    /// Creates a new UDP socket and returns its handle.
    pub fn udp_create(&mut self) -> UdpSocketHandle {
        let handle = self.alloc_handle();
        self.udp_sockets.insert(handle, UdpSocket::new());
        handle
    }

    /// Binds a UDP socket to a local address.
    pub fn udp_bind(
        &mut self,
        handle: UdpSocketHandle,
        addr: SocketAddr,
    ) -> Result<(), SocketError> {
        let sock = self
            .udp_sockets
            .get_mut(&handle)
            .ok_or(SocketError::BadHandle)?;

        if sock.local_addr.is_some() {
            return Err(SocketError::InvalidState(
                "UDP socket already bound".to_string(),
            ));
        }

        if self.bound_udp.contains_key(&addr) && !sock.options.reuse_address {
            return Err(SocketError::AddressInUse);
        }

        sock.local_addr = Some(addr.clone());
        self.bound_udp.insert(addr, handle);
        Ok(())
    }

    /// Sends a datagram from a UDP socket to a remote address.
    pub fn udp_send_to(
        &mut self,
        handle: UdpSocketHandle,
        data: &[u8],
        remote: SocketAddr,
    ) -> Result<u64, SocketError> {
        // Auto-bind if not yet bound.
        {
            let sock = self
                .udp_sockets
                .get_mut(&handle)
                .ok_or(SocketError::BadHandle)?;

            if sock.local_addr.is_none() {
                let port = self.next_ephemeral_port;
                self.next_ephemeral_port += 1;
                sock.local_addr = Some(SocketAddr::new(IpAddress::localhost_v4(), port));
            }
        }

        let local_addr = {
            let sock = self
                .udp_sockets
                .get(&handle)
                .ok_or(SocketError::BadHandle)?;
            sock.local_addr.clone()
        };

        let datagram = Datagram {
            data: data.to_vec(),
            remote_addr: remote.clone(),
        };

        // Record the sent datagram.
        let len = data.len() as u64;
        {
            let sock = self
                .udp_sockets
                .get_mut(&handle)
                .ok_or(SocketError::BadHandle)?;
            sock.sent_datagrams.push(datagram);
            sock.bytes_sent += len;
        }

        // Deliver to the remote socket if it exists in the table.
        if let Some(&remote_handle) = self.bound_udp.get(&remote) {
            if let Some(remote_sock) = self.udp_sockets.get_mut(&remote_handle) {
                let source =
                    local_addr.ok_or(SocketError::InvalidState("sender not bound".to_string()))?;
                remote_sock.recv_queue.push_back(Datagram {
                    data: data.to_vec(),
                    remote_addr: source,
                });
            }
        }

        Ok(len)
    }

    /// Receives a datagram from a UDP socket.
    pub fn udp_receive_from(&mut self, handle: UdpSocketHandle) -> Result<Datagram, SocketError> {
        let sock = self
            .udp_sockets
            .get_mut(&handle)
            .ok_or(SocketError::BadHandle)?;

        match sock.recv_queue.pop_front() {
            Some(dgram) => {
                sock.bytes_received += dgram.data.len() as u64;
                Ok(dgram)
            }
            None => Err(SocketError::WouldBlock),
        }
    }

    /// Delivers a datagram into a UDP socket's receive queue (test harness).
    pub fn udp_deliver(
        &mut self,
        handle: UdpSocketHandle,
        datagram: Datagram,
    ) -> Result<(), SocketError> {
        let sock = self
            .udp_sockets
            .get_mut(&handle)
            .ok_or(SocketError::BadHandle)?;
        sock.recv_queue.push_back(datagram);
        Ok(())
    }

    /// Closes a UDP socket and releases its bound address.
    pub fn udp_close(&mut self, handle: UdpSocketHandle) -> Result<(), SocketError> {
        let sock = self
            .udp_sockets
            .remove(&handle)
            .ok_or(SocketError::BadHandle)?;

        if let Some(addr) = &sock.local_addr {
            self.bound_udp.remove(addr);
        }

        Ok(())
    }

    // ── W7.6: DNS ──

    /// Resolves a hostname to a list of IP addresses using the built-in DNS table.
    pub fn resolve(&self, hostname: &str) -> Result<Vec<IpAddress>, SocketError> {
        self.dns.resolve(hostname)
    }

    // ── W7.8: Non-blocking I/O ──

    /// Returns true if a TCP socket has data available to read.
    pub fn tcp_is_readable(&self, handle: TcpSocketHandle) -> Result<bool, SocketError> {
        let sock = self
            .tcp_sockets
            .get(&handle)
            .ok_or(SocketError::BadHandle)?;

        match sock.state {
            TcpState::Connected => Ok(!sock.recv_buffer.is_empty()),
            TcpState::Listening => Ok(!sock.accept_queue.is_empty()),
            _ => Ok(false),
        }
    }

    /// Returns true if a TCP socket can accept write data.
    pub fn tcp_is_writable(&self, handle: TcpSocketHandle) -> Result<bool, SocketError> {
        let sock = self
            .tcp_sockets
            .get(&handle)
            .ok_or(SocketError::BadHandle)?;

        Ok(sock.state == TcpState::Connected)
    }

    /// Returns true if a UDP socket has datagrams available to read.
    pub fn udp_is_readable(&self, handle: UdpSocketHandle) -> Result<bool, SocketError> {
        let sock = self
            .udp_sockets
            .get(&handle)
            .ok_or(SocketError::BadHandle)?;

        Ok(!sock.recv_queue.is_empty())
    }

    /// Returns true if a UDP socket can accept outgoing datagrams.
    pub fn udp_is_writable(&self, handle: UdpSocketHandle) -> Result<bool, SocketError> {
        // UDP sockets are always writable once created.
        if !self.udp_sockets.contains_key(&handle) {
            return Err(SocketError::BadHandle);
        }
        Ok(true)
    }
}

impl Default for SocketTable {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── W7.1: TCP types ──

    #[test]
    fn w7_1_ip_address_v4_display_and_properties() {
        let ip = IpAddress::V4(192, 168, 1, 100);
        assert_eq!(ip.to_string(), "192.168.1.100");
        assert!(ip.is_v4());
        assert!(!ip.is_v6());

        let lo = IpAddress::localhost_v4();
        assert_eq!(lo.to_string(), "127.0.0.1");
    }

    #[test]
    fn w7_1_ip_address_v6_display_and_properties() {
        let ip = IpAddress::V6(0x2001, 0x0db8, 0, 0, 0, 0, 0, 1);
        assert_eq!(ip.to_string(), "2001:db8:0:0:0:0:0:1");
        assert!(ip.is_v6());
        assert!(!ip.is_v4());

        let lo = IpAddress::localhost_v6();
        assert_eq!(lo.to_string(), "0:0:0:0:0:0:0:1");
    }

    #[test]
    fn w7_1_socket_addr_display() {
        let v4 = SocketAddr::new(IpAddress::localhost_v4(), 8080);
        assert_eq!(v4.to_string(), "127.0.0.1:8080");

        let v6 = SocketAddr::new(IpAddress::localhost_v6(), 443);
        assert_eq!(v6.to_string(), "[0:0:0:0:0:0:0:1]:443");
    }

    #[test]
    fn w7_1_network_resource() {
        let net = Network::new(1, "default");
        assert_eq!(net.handle, 1);
        assert_eq!(net.name, "default");
    }

    #[test]
    fn w7_1_tcp_state_display() {
        assert_eq!(TcpState::Closed.to_string(), "closed");
        assert_eq!(TcpState::Connecting.to_string(), "connecting");
        assert_eq!(TcpState::Connected.to_string(), "connected");
        assert_eq!(TcpState::Listening.to_string(), "listening");
    }

    // ── W7.2: TCP connect ──

    #[test]
    fn w7_2_tcp_connect_to_listener() {
        let mut table = SocketTable::new();

        // Set up a listener on port 8080.
        let server = table.tcp_create();
        let server_addr = SocketAddr::new(IpAddress::localhost_v4(), 8080);
        table.tcp_bind(server, server_addr.clone()).unwrap();
        table.tcp_start_listen(server).unwrap();

        // Client connects.
        let client = table.tcp_create();
        table.tcp_start_connect(client, server_addr).unwrap();
        assert_eq!(table.tcp_sockets[&client].state(), TcpState::Connecting,);

        table.tcp_finish_connect(client).unwrap();
        assert_eq!(table.tcp_sockets[&client].state(), TcpState::Connected,);
    }

    #[test]
    fn w7_2_tcp_connect_refused_no_listener() {
        let mut table = SocketTable::new();
        let client = table.tcp_create();
        let remote = SocketAddr::new(IpAddress::localhost_v4(), 9999);

        table.tcp_start_connect(client, remote).unwrap();
        let err = table.tcp_finish_connect(client).unwrap_err();
        assert_eq!(err, SocketError::ConnectionRefused);

        // Socket should be reset to Closed.
        assert_eq!(table.tcp_sockets[&client].state(), TcpState::Closed,);
    }

    // ── W7.3: TCP listen & accept ──

    #[test]
    fn w7_3_tcp_listen_and_accept() {
        let mut table = SocketTable::new();

        let server = table.tcp_create();
        let addr = SocketAddr::new(IpAddress::localhost_v4(), 3000);
        table.tcp_bind(server, addr.clone()).unwrap();
        table.tcp_start_listen(server).unwrap();
        assert_eq!(table.tcp_sockets[&server].state(), TcpState::Listening,);

        // No connections yet — should return WouldBlock.
        let err = table.tcp_accept(server).unwrap_err();
        assert_eq!(err, SocketError::WouldBlock);

        // Client connects.
        let client = table.tcp_create();
        table.tcp_start_connect(client, addr).unwrap();
        table.tcp_finish_connect(client).unwrap();

        // Accept the connection — returns (client_handle, peer_handle).
        let (accepted_client, _peer) = table.tcp_accept(server).unwrap();
        assert_eq!(accepted_client, client);
    }

    #[test]
    fn w7_3_tcp_listen_without_bind_fails() {
        let mut table = SocketTable::new();
        let server = table.tcp_create();
        let err = table.tcp_start_listen(server).unwrap_err();
        assert!(matches!(err, SocketError::InvalidState(_)));
    }

    // ── W7.4: TCP streams (echo server pattern) ──

    #[test]
    fn w7_4_tcp_echo_server() {
        let mut table = SocketTable::new();

        // Server setup.
        let server = table.tcp_create();
        let addr = SocketAddr::new(IpAddress::localhost_v4(), 7000);
        table.tcp_bind(server, addr.clone()).unwrap();
        table.tcp_start_listen(server).unwrap();

        // Client connects and sends data.
        let client = table.tcp_create();
        table.tcp_start_connect(client, addr).unwrap();
        table.tcp_finish_connect(client).unwrap();

        table.tcp_send(client, b"Hello, server!").unwrap();

        // Accept the client on server side — get the server-side peer.
        let (_accepted_client, peer) = table.tcp_accept(server).unwrap();

        // Simulate the network delivering client's send_buffer to peer's recv_buffer.
        let sent_data: Vec<u8> = table.tcp_sockets[&client]
            .send_buffer
            .iter()
            .copied()
            .collect();
        table.tcp_deliver(peer, &sent_data).unwrap();

        // Server-side peer reads the data.
        let received = table.tcp_recv(peer, 1024).unwrap();
        assert_eq!(received, b"Hello, server!");

        // Server echoes back: send from peer, deliver to client.
        table.tcp_send(peer, &received).unwrap();
        let echo_data: Vec<u8> = table.tcp_sockets[&peer]
            .send_buffer
            .iter()
            .copied()
            .collect();
        table.tcp_deliver(client, &echo_data).unwrap();

        // Client reads the echo.
        let echo = table.tcp_recv(client, 1024).unwrap();
        assert_eq!(echo, b"Hello, server!");
    }

    #[test]
    fn w7_4_tcp_send_on_closed_fails() {
        let mut table = SocketTable::new();
        let sock = table.tcp_create();
        let err = table.tcp_send(sock, b"data").unwrap_err();
        assert_eq!(err, SocketError::NotConnected);
    }

    // ── W7.5: UDP ──

    #[test]
    fn w7_5_udp_send_and_receive() {
        let mut table = SocketTable::new();

        let sender = table.udp_create();
        let sender_addr = SocketAddr::new(IpAddress::localhost_v4(), 5000);
        table.udp_bind(sender, sender_addr).unwrap();

        let receiver = table.udp_create();
        let receiver_addr = SocketAddr::new(IpAddress::localhost_v4(), 5001);
        table.udp_bind(receiver, receiver_addr.clone()).unwrap();

        // Send from sender to receiver.
        let sent = table
            .udp_send_to(sender, b"UDP payload", receiver_addr)
            .unwrap();
        assert_eq!(sent, 11);

        // Receiver gets the datagram.
        let dgram = table.udp_receive_from(receiver).unwrap();
        assert_eq!(dgram.data, b"UDP payload");
        assert_eq!(dgram.remote_addr.port, 5000);
    }

    #[test]
    fn w7_5_udp_receive_empty_returns_would_block() {
        let mut table = SocketTable::new();
        let sock = table.udp_create();
        let err = table.udp_receive_from(sock).unwrap_err();
        assert_eq!(err, SocketError::WouldBlock);
    }

    // ── W7.6: DNS lookup ──

    #[test]
    fn w7_6_dns_resolve_localhost() {
        let table = SocketTable::new();
        let addrs = table.resolve("localhost").unwrap();
        assert!(!addrs.is_empty());
        assert!(addrs.contains(&IpAddress::localhost_v4()));
    }

    #[test]
    fn w7_6_dns_resolve_custom_host() {
        let mut table = SocketTable::new();
        table
            .dns
            .add_record("api.example.com", IpAddress::V4(93, 184, 216, 34));
        let addrs = table.resolve("api.example.com").unwrap();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0], IpAddress::V4(93, 184, 216, 34));
    }

    #[test]
    fn w7_6_dns_resolve_unknown_fails() {
        let table = SocketTable::new();
        let err = table.resolve("nonexistent.invalid").unwrap_err();
        assert!(matches!(err, SocketError::NameResolutionFailed(_)));
    }

    // ── W7.7: Socket options ──

    #[test]
    fn w7_7_socket_options_defaults_and_set() {
        let mut table = SocketTable::new();
        let sock = table.tcp_create();

        {
            let opts = table.tcp_sockets.get(&sock).unwrap().options();
            assert!(!opts.reuse_address);
            assert!(!opts.tcp_nodelay);
            assert_eq!(opts.send_timeout_ms, 0);
        }

        {
            let opts = table.tcp_sockets.get_mut(&sock).unwrap().options_mut();
            opts.reuse_address = true;
            opts.tcp_nodelay = true;
            opts.send_timeout_ms = 5000;
            opts.recv_timeout_ms = 3000;
            opts.keep_alive_secs = 60;
        }

        let opts = table.tcp_sockets.get(&sock).unwrap().options();
        assert!(opts.reuse_address);
        assert!(opts.tcp_nodelay);
        assert_eq!(opts.send_timeout_ms, 5000);
        assert_eq!(opts.recv_timeout_ms, 3000);
        assert_eq!(opts.keep_alive_secs, 60);
    }

    #[test]
    fn w7_7_bind_reuse_address() {
        let mut table = SocketTable::new();
        let addr = SocketAddr::new(IpAddress::localhost_v4(), 4000);

        let s1 = table.tcp_create();
        table.tcp_bind(s1, addr.clone()).unwrap();
        table.tcp_start_listen(s1).unwrap();

        // Without reuse_address, binding the same address fails.
        let s2 = table.tcp_create();
        let err = table.tcp_bind(s2, addr.clone()).unwrap_err();
        assert_eq!(err, SocketError::AddressInUse);

        // Close the first socket to release the address.
        table.tcp_close(s1).unwrap();

        // Now binding succeeds.
        let s3 = table.tcp_create();
        table.tcp_bind(s3, addr).unwrap();
    }

    // ── W7.8: Non-blocking I/O ──

    #[test]
    fn w7_8_tcp_is_readable_writable() {
        let mut table = SocketTable::new();

        let server = table.tcp_create();
        let addr = SocketAddr::new(IpAddress::localhost_v4(), 6000);
        table.tcp_bind(server, addr.clone()).unwrap();
        table.tcp_start_listen(server).unwrap();

        let client = table.tcp_create();
        table.tcp_start_connect(client, addr).unwrap();
        table.tcp_finish_connect(client).unwrap();

        // Listener is readable because accept queue is non-empty (before accept).
        assert!(table.tcp_is_readable(server).unwrap());

        // Accept to get the peer.
        let (_c, peer) = table.tcp_accept(server).unwrap();

        // Client is connected but no data yet.
        assert!(!table.tcp_is_readable(client).unwrap());
        assert!(table.tcp_is_writable(client).unwrap());

        // Deliver data to client.
        table.tcp_deliver(client, b"incoming").unwrap();
        assert!(table.tcp_is_readable(client).unwrap());

        // Peer is also writable.
        assert!(table.tcp_is_writable(peer).unwrap());
    }

    #[test]
    fn w7_8_udp_is_readable_writable() {
        let mut table = SocketTable::new();
        let sock = table.udp_create();
        let addr = SocketAddr::new(IpAddress::localhost_v4(), 7777);
        table.udp_bind(sock, addr).unwrap();

        // No datagrams yet.
        assert!(!table.udp_is_readable(sock).unwrap());
        assert!(table.udp_is_writable(sock).unwrap());

        // Deliver a datagram.
        table
            .udp_deliver(
                sock,
                Datagram {
                    data: b"ping".to_vec(),
                    remote_addr: SocketAddr::new(IpAddress::localhost_v4(), 9999),
                },
            )
            .unwrap();
        assert!(table.udp_is_readable(sock).unwrap());
    }

    // ── W7.9: Socket errors ──

    #[test]
    fn w7_9_socket_error_display() {
        assert_eq!(
            SocketError::ConnectionRefused.to_string(),
            "connection refused",
        );
        assert_eq!(SocketError::Timeout.to_string(), "operation timed out",);
        assert_eq!(SocketError::ConnectionReset.to_string(), "connection reset",);
        assert_eq!(
            SocketError::AddressInUse.to_string(),
            "address already in use",
        );
        assert_eq!(SocketError::WouldBlock.to_string(), "operation would block",);
        assert_eq!(SocketError::BadHandle.to_string(), "bad socket handle",);
        assert_eq!(
            SocketError::NameResolutionFailed("host".into()).to_string(),
            "name resolution failed for: host",
        );
        assert_eq!(
            SocketError::NetworkUnreachable.to_string(),
            "network unreachable",
        );
    }

    #[test]
    fn w7_9_bad_handle_errors() {
        let mut table = SocketTable::new();
        assert_eq!(
            table.tcp_send(999, b"data").unwrap_err(),
            SocketError::BadHandle,
        );
        assert_eq!(
            table.tcp_recv(999, 100).unwrap_err(),
            SocketError::BadHandle,
        );
        assert_eq!(table.tcp_close(999).unwrap_err(), SocketError::BadHandle,);
        assert_eq!(
            table.udp_receive_from(999).unwrap_err(),
            SocketError::BadHandle,
        );
        assert!(table.tcp_is_readable(999).is_err());
        assert!(table.udp_is_writable(999).is_err());
    }

    // ── W7.10: Comprehensive tests ──

    #[test]
    fn w7_10_full_tcp_lifecycle() {
        let mut table = SocketTable::new();

        // Server: create, bind, listen.
        let server = table.tcp_create();
        let addr = SocketAddr::new(IpAddress::localhost_v4(), 8888);
        table.tcp_bind(server, addr.clone()).unwrap();
        table.tcp_start_listen(server).unwrap();

        // Client: create, connect.
        let client = table.tcp_create();
        table.tcp_start_connect(client, addr).unwrap();
        table.tcp_finish_connect(client).unwrap();

        // Accept — returns (client_handle, peer_handle).
        let (accepted_client, peer) = table.tcp_accept(server).unwrap();
        assert_eq!(accepted_client, client);

        // Client sends "request" to server.
        table.tcp_send(client, b"request").unwrap();
        let sent: Vec<u8> = table.tcp_sockets[&client]
            .send_buffer
            .iter()
            .copied()
            .collect();
        table.tcp_deliver(peer, &sent).unwrap();

        let req = table.tcp_recv(peer, 1024).unwrap();
        assert_eq!(req, b"request");

        // Server-side peer sends "response" back to client.
        table.tcp_send(peer, b"response").unwrap();
        let resp_data: Vec<u8> = table.tcp_sockets[&peer]
            .send_buffer
            .iter()
            .copied()
            .collect();
        table.tcp_deliver(client, &resp_data).unwrap();

        let resp = table.tcp_recv(client, 1024).unwrap();
        assert_eq!(resp, b"response");

        // Byte counters.
        assert_eq!(table.tcp_sockets[&client].bytes_sent(), 7);
        assert_eq!(table.tcp_sockets[&client].bytes_received(), 8);
        assert_eq!(table.tcp_sockets[&peer].bytes_sent(), 8);
        assert_eq!(table.tcp_sockets[&peer].bytes_received(), 7);

        // Close everything.
        table.tcp_close(client).unwrap();
        table.tcp_close(peer).unwrap();
        table.tcp_close(server).unwrap();

        assert!(!table.tcp_sockets.contains_key(&client));
        assert!(!table.tcp_sockets.contains_key(&peer));
        assert!(!table.tcp_sockets.contains_key(&server));
    }

    #[test]
    fn w7_10_full_udp_bidirectional() {
        let mut table = SocketTable::new();

        let alice = table.udp_create();
        let alice_addr = SocketAddr::new(IpAddress::localhost_v4(), 10000);
        table.udp_bind(alice, alice_addr.clone()).unwrap();

        let bob = table.udp_create();
        let bob_addr = SocketAddr::new(IpAddress::localhost_v4(), 10001);
        table.udp_bind(bob, bob_addr.clone()).unwrap();

        // Alice -> Bob.
        table
            .udp_send_to(alice, b"hello bob", bob_addr.clone())
            .unwrap();
        let dgram = table.udp_receive_from(bob).unwrap();
        assert_eq!(dgram.data, b"hello bob");
        assert_eq!(dgram.remote_addr, alice_addr);

        // Bob -> Alice.
        table
            .udp_send_to(bob, b"hello alice", alice_addr.clone())
            .unwrap();
        let dgram = table.udp_receive_from(alice).unwrap();
        assert_eq!(dgram.data, b"hello alice");
        assert_eq!(dgram.remote_addr, bob_addr);

        // Verify byte counters.
        assert_eq!(table.udp_sockets[&alice].bytes_sent(), 9,);
        assert_eq!(table.udp_sockets[&alice].bytes_received(), 11,);

        // Verify sent datagram log.
        assert_eq!(table.udp_sockets[&alice].sent_datagrams().len(), 1,);

        // Cleanup.
        table.udp_close(alice).unwrap();
        table.udp_close(bob).unwrap();
        assert!(table.udp_sockets.is_empty());
    }

    #[test]
    fn w7_10_dns_to_tcp_connect_workflow() {
        let mut table = SocketTable::new();

        // Register a DNS entry for our simulated server.
        table
            .dns
            .add_record("myserver.local", IpAddress::V4(10, 0, 0, 1));

        // Resolve the hostname.
        let addrs = table.resolve("myserver.local").unwrap();
        assert_eq!(addrs[0], IpAddress::V4(10, 0, 0, 1));

        // Set up a listener on the resolved address.
        let server = table.tcp_create();
        let server_addr = SocketAddr::new(addrs[0].clone(), 443);
        table.tcp_bind(server, server_addr.clone()).unwrap();
        table.tcp_start_listen(server).unwrap();

        // Client resolves and connects.
        let client = table.tcp_create();
        table.tcp_start_connect(client, server_addr).unwrap();
        table.tcp_finish_connect(client).unwrap();

        assert_eq!(table.tcp_sockets[&client].state(), TcpState::Connected,);

        let (accepted_client, peer) = table.tcp_accept(server).unwrap();
        assert_eq!(accepted_client, client);
        assert_eq!(table.tcp_sockets[&peer].state(), TcpState::Connected,);
    }

    #[test]
    fn w7_10_multiple_clients_to_one_server() {
        let mut table = SocketTable::new();

        let server = table.tcp_create();
        let addr = SocketAddr::new(IpAddress::localhost_v4(), 2222);
        table.tcp_bind(server, addr.clone()).unwrap();
        table.tcp_start_listen(server).unwrap();

        // Connect 3 clients.
        let mut clients = Vec::new();
        for _ in 0..3 {
            let c = table.tcp_create();
            table.tcp_start_connect(c, addr.clone()).unwrap();
            table.tcp_finish_connect(c).unwrap();
            clients.push(c);
        }

        // Accept all 3 — each returns (client_handle, peer_handle).
        let mut peers = Vec::new();
        for (i, expected) in clients.iter().enumerate() {
            let (accepted_client, peer) = table.tcp_accept(server).unwrap();
            assert_eq!(accepted_client, *expected, "client {i} mismatch",);
            peers.push(peer);
        }
        assert_eq!(peers.len(), 3);

        // Accept queue now empty.
        let err = table.tcp_accept(server).unwrap_err();
        assert_eq!(err, SocketError::WouldBlock);
    }
}
