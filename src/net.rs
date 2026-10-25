use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket},
    sync::OnceLock,
    time::Duration,
};

static BYPASS_IP: OnceLock<Option<IpAddr>> = OnceLock::new();

/// Returns a local physical interface IPv4 address to bind outbound sockets to,
/// bypassing Clash/Mihomo TUN (`198.18.0.0/15`) and TAP (`10.0.0.1`) virtual interfaces.
/// Can be overridden via `GDIRECT_BIND_IP` (set `GDIRECT_BIND_IP=none` to disable).
pub fn local_bind_ip() -> Option<IpAddr> {
    *BYPASS_IP.get_or_init(detect_local_bind_ip)
}

fn is_tun_or_virtual(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        // Clash/Mihomo TUN benchmark range 198.18.0.0/15
        || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
        // cfw-tap default address
        || (o == [10, 0, 0, 1])
}

fn score_candidate(ip: Ipv4Addr) -> u8 {
    let o = ip.octets();
    // Typical LAN physical gateway subnets (192.168.0.x - 192.168.15.x, 192.168.31.x, 192.168.50.x, etc.)
    // whereas VMware/VirtualBox often use 192.168.19.1, 192.168.56.1, 192.168.60.1 (host side is .1)
    if o[0] == 192 && o[1] == 168 {
        if o[3] != 1 {
            return 0; // Client DHCP/static IP on LAN (e.g., 192.168.1.126) - highest priority
        }
        return 2; // Host side of a virtual switch (.1)
    }
    if o[0] == 10 {
        return 1;
    }
    // WSL / Hyper-V default switch often uses 172.16.0.0/12 with .1 on host
    if o[0] == 172 && (16..=31).contains(&o[1]) {
        if o[3] != 1 {
            return 1;
        }
        return 3;
    }
    1
}

fn can_route_external(local: Ipv4Addr) -> bool {
    let bind_addr = SocketAddr::new(IpAddr::V4(local), 0);
    let Ok(udp) = UdpSocket::bind(bind_addr) else {
        return false;
    };
    if udp.connect("223.5.5.5:53").is_err() {
        return false;
    }
    // Verify real TCP egress on physical NIC (183.56.143.147:443 or 223.5.5.5:53)
    for target in ["183.56.143.147:443", "223.5.5.5:53"] {
        if let Ok(target_addr) = target.parse::<SocketAddr>()
            && let Ok(sock) = create_bound_tcp_v4(local, target_addr, Duration::from_millis(1200))
        {
            drop(sock);
            return true;
        }
    }
    false
}

pub fn create_bound_tcp_v4(
    local: Ipv4Addr,
    remote: SocketAddr,
    timeout: Duration,
) -> std::io::Result<TcpStream> {
    #[cfg(windows)]
    {
        use std::os::windows::io::{FromRawSocket, RawSocket};
        unsafe extern "system" {
            fn WSASocketW(
                af: i32,
                socket_type: i32,
                protocol: i32,
                lpProtocolInfo: *mut std::ffi::c_void,
                g: u32,
                dwFlags: u32,
            ) -> usize;
            fn bind(s: usize, name: *const SockAddrIn, namelen: i32) -> i32;
            fn closesocket(s: usize) -> i32;
        }
        #[repr(C)]
        struct SockAddrIn {
            sin_family: u16,
            sin_port: u16,
            sin_addr: [u8; 4],
            sin_zero: [u8; 8],
        }
        // Trigger Winsock init via dummy UdpSocket if not yet initialized
        let _ = UdpSocket::bind("127.0.0.1:0");
        let s = unsafe { WSASocketW(2, 1, 6, std::ptr::null_mut(), 0, 1) };
        if s == usize::MAX {
            return Err(std::io::Error::last_os_error());
        }
        let addr = SockAddrIn {
            sin_family: 2, // AF_INET
            sin_port: 0,
            sin_addr: local.octets(),
            sin_zero: [0; 8],
        };
        if unsafe { bind(s, &addr, std::mem::size_of::<SockAddrIn>() as i32) } != 0 {
            let err = std::io::Error::last_os_error();
            unsafe {
                closesocket(s);
            }
            return Err(err);
        }
        // Connect using non-blocking mode + select via std or connect_timeout:
        // Wait: std::net::TcpStream doesn't have connect_on_existing_socket.
        // Instead let's connect with WSAConnect / ioctlsocket + select!
        unsafe extern "system" {
            fn ioctlsocket(s: usize, cmd: i32, argp: *mut u32) -> i32;
            fn connect(s: usize, name: *const SockAddrIn, namelen: i32) -> i32;
            fn select(
                nfds: i32,
                readfds: *mut FdSet,
                writefds: *mut FdSet,
                exceptfds: *mut FdSet,
                timeout: *const TimeVal,
            ) -> i32;
        }
        #[repr(C)]
        struct FdSet {
            fd_count: u32,
            fd_array: [usize; 1],
        }
        #[repr(C)]
        struct TimeVal {
            tv_sec: i32,
            tv_usec: i32,
        }
        let SocketAddr::V4(remote_v4) = remote else {
            unsafe {
                closesocket(s);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "IPv4 required",
            ));
        };
        let mut nonblocking: u32 = 1;
        unsafe {
            ioctlsocket(s, -2147195266i32, &mut nonblocking); // FIONBIO = 0x8004667e
        }
        let raddr = SockAddrIn {
            sin_family: 2,
            sin_port: remote_v4.port().to_be(),
            sin_addr: remote_v4.ip().octets(),
            sin_zero: [0; 8],
        };
        let _ = unsafe { connect(s, &raddr, std::mem::size_of::<SockAddrIn>() as i32) };
        let mut writefds = FdSet {
            fd_count: 1,
            fd_array: [s],
        };
        let mut exceptfds = FdSet {
            fd_count: 1,
            fd_array: [s],
        };
        let tv = TimeVal {
            tv_sec: timeout.as_secs() as i32,
            tv_usec: timeout.subsec_micros() as i32,
        };
        let ready = unsafe {
            select(
                0,
                std::ptr::null_mut(),
                &mut writefds,
                &mut exceptfds,
                &tv,
            )
        };
        if ready <= 0 || writefds.fd_count == 0 || exceptfds.fd_count != 0 {
            unsafe {
                closesocket(s);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "connection timed out",
            ));
        }
        nonblocking = 0;
        unsafe {
            ioctlsocket(s, -2147195266i32, &mut nonblocking);
            Ok(TcpStream::from_raw_socket(s as RawSocket))
        }
    }
    #[cfg(not(windows))]
    {
        let _ = local;
        TcpStream::connect_timeout(&remote, timeout)
    }
}

fn host_ipv4_candidates() -> Vec<Ipv4Addr> {
    let mut list = Vec::new();
    #[cfg(windows)]
    {
        unsafe extern "system" {
            fn gethostname(name: *mut u8, namelen: i32) -> i32;
        }
        let _ = UdpSocket::bind("127.0.0.1:0");
        let mut buf = [0u8; 256];
        if unsafe { gethostname(buf.as_mut_ptr(), buf.len() as i32) } == 0
            && let Some(end) = buf.iter().position(|&b| b == 0)
            && let Ok(hostname) = std::str::from_utf8(&buf[..end])
            && let Ok(addrs) = (hostname, 0).to_socket_addrs()
        {
            for addr in addrs {
                if let IpAddr::V4(v4) = addr.ip()
                    && !is_tun_or_virtual(v4)
                    && !list.contains(&v4)
                {
                    list.push(v4);
                }
            }
        }
    }
    list.sort_by_key(|ip| score_candidate(*ip));
    list
}

fn detect_local_bind_ip() -> Option<IpAddr> {
    if let Ok(val) = std::env::var("GDIRECT_BIND_IP") {
        let trimmed = val.trim();
        if trimmed.eq_ignore_ascii_case("none") || trimmed.is_empty() {
            return None;
        }
        if let Ok(ip) = trimmed.parse::<IpAddr>() {
            return Some(ip);
        }
    }

    // Check default outbound IP chosen by OS routing table
    if let Ok(udp) = UdpSocket::bind("0.0.0.0:0")
        && udp.connect("183.56.143.147:443").is_ok()
        && let Ok(local) = udp.local_addr()
        && let IpAddr::V4(v4) = local.ip()
        && !is_tun_or_virtual(v4)
    {
        // System default route is already a real physical interface, no TUN hijack active
        return Some(IpAddr::V4(v4));
    }

    // TUN hijack is active (e.g. Clash 198.18.0.1 or cfw-tap 10.0.0.1).
    // Probe non-TUN host IPv4 addresses to find the real physical NIC.
    for candidate in host_ipv4_candidates() {
        if can_route_external(candidate) {
            eprintln!("Bypassing Clash/TUN via local interface {candidate}");
            return Some(IpAddr::V4(candidate));
        }
    }
    None
}
