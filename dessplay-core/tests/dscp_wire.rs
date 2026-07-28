//! Wire-level regression test: the DSCP tag set on a socket must survive
//! quinn's per-packet ECN control message.
//!
//! quinn-udp attaches an `IP_TOS` / `IPV6_TCLASS` cmsg to every datagram
//! whose value is derived solely from the transmit's ECN codepoint, and a
//! per-packet cmsg overrides the socket-level `setsockopt` — so stock
//! quinn-udp 0.5.14 rewrites the TOS byte to `ecn` (DSCP 0) on the wire.
//! The vendored patch (vendor/quinn-udp) captures the socket's TOS byte at
//! `UdpSocketState` creation and ORs it into the cmsg value.
//!
//! The test sends one datagram over loopback through `quinn_udp` on a
//! DSCP-tagged socket, receives it with a raw `recvmsg` that requests the
//! TOS / traffic-class byte, and asserts the DSCP bits arrived intact.

#![cfg(unix)]

use std::mem;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::os::fd::AsRawFd;
use std::time::Duration;

use quinn_udp::{EcnCodepoint, Transmit, UdpSocketState};

/// The DSCP under test: AF41, the control-plane tag (quic.rs).
const DSCP: u8 = 34;
/// The full TOS/traffic-class byte the socket is tagged with.
const TOS: u8 = DSCP << 2;
/// ECN codepoint quinn sets per-packet; ECT(0) = 0b10.
const ECN: EcnCodepoint = EcnCodepoint::Ect0;

/// Enable TOS/TCLASS reception on `sock` via setsockopt.
fn enable_recv_tos(sock: &UdpSocket, ipv4: bool) {
    let on: libc::c_int = 1;
    let (level, opt) = if ipv4 {
        (libc::IPPROTO_IP, libc::IP_RECVTOS)
    } else {
        (libc::IPPROTO_IPV6, libc::IPV6_RECVTCLASS)
    };
    let rc = unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            level,
            opt,
            &on as *const _ as *const libc::c_void,
            mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    assert_eq!(rc, 0, "setsockopt RECVTOS/RECVTCLASS failed");
}

/// Receive one datagram and return its TOS / traffic-class byte.
fn recv_tos(sock: &UdpSocket) -> u8 {
    let mut buf = [0u8; 64];
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr() as *mut libc::c_void,
        iov_len: buf.len(),
    };
    // Aligned control buffer.
    let mut ctrl = [0u64; 16];
    let mut hdr: libc::msghdr = unsafe { mem::zeroed() };
    hdr.msg_iov = &mut iov;
    hdr.msg_iovlen = 1;
    hdr.msg_control = ctrl.as_mut_ptr() as *mut libc::c_void;
    hdr.msg_controllen = mem::size_of_val(&ctrl) as _;

    let n = unsafe { libc::recvmsg(sock.as_raw_fd(), &mut hdr, 0) };
    assert!(
        n >= 0,
        "recvmsg failed: {}",
        std::io::Error::last_os_error()
    );

    let mut cmsg = unsafe { libc::CMSG_FIRSTHDR(&hdr) };
    while !cmsg.is_null() {
        let c = unsafe { &*cmsg };
        let is_tos = (c.cmsg_level == libc::IPPROTO_IP
            && (c.cmsg_type == libc::IP_TOS || c.cmsg_type == libc::IP_RECVTOS))
            || (c.cmsg_level == libc::IPPROTO_IPV6 && c.cmsg_type == libc::IPV6_TCLASS);
        if is_tos {
            // Delivered as a single byte on BSD/macOS, an int on Linux;
            // either way the low byte is the value on little-endian, and
            // reading one byte is correct for both encodings in practice
            // (Linux writes a full host-order int whose low byte is the
            // TOS; macOS writes exactly one byte).
            let data = unsafe { libc::CMSG_DATA(cmsg) };
            return unsafe { *data };
        }
        cmsg = unsafe { libc::CMSG_NXTHDR(&hdr, cmsg) };
    }
    panic!("no TOS/TCLASS cmsg on received datagram");
}

/// Send one quinn-udp datagram from a DSCP-tagged socket to `dst`.
fn send_tagged(sender: &UdpSocket, dst: SocketAddr) {
    let state = UdpSocketState::new((&*sender).into()).expect("UdpSocketState");
    let transmit = Transmit {
        destination: dst,
        ecn: Some(ECN),
        contents: b"dscp",
        segment_size: None,
        src_ip: None,
    };
    // The state sets the socket nonblocking; loopback won't backpressure
    // a 4-byte datagram, but retry WouldBlock a few times to be safe.
    for _ in 0..10 {
        match state.try_send((&*sender).into(), &transmit) {
            Ok(()) => return,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("try_send failed: {e}"),
        }
    }
    panic!("try_send kept returning WouldBlock");
}

fn assert_dscp_survives(ipv4: bool) {
    let loopback: IpAddr = if ipv4 {
        Ipv4Addr::LOCALHOST.into()
    } else {
        Ipv6Addr::LOCALHOST.into()
    };

    let receiver = UdpSocket::bind((loopback, 0)).expect("bind receiver");
    receiver
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set_read_timeout");
    enable_recv_tos(&receiver, ipv4);
    let dst = receiver.local_addr().expect("receiver addr");

    // Tag the sender exactly the way quic.rs::bind_socket does.
    let sender = {
        let domain = if ipv4 {
            socket2::Domain::IPV4
        } else {
            socket2::Domain::IPV6
        };
        let socket =
            socket2::Socket::new(domain, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))
                .expect("create sender");
        let tagged = if ipv4 {
            socket.set_tos_v4(u32::from(TOS))
        } else {
            socket.set_tclass_v6(u32::from(TOS))
        };
        tagged.expect("DSCP setsockopt");
        socket
            .bind(&SocketAddr::from((loopback, 0)).into())
            .expect("bind sender");
        UdpSocket::from(socket)
    };

    send_tagged(&sender, dst);

    let tos = recv_tos(&receiver);
    assert_eq!(
        tos >> 2,
        DSCP,
        "DSCP clobbered: wire TOS byte was {tos:#04x} (DSCP {}, ECN {:#04b}), \
         expected DSCP {DSCP}",
        tos >> 2,
        tos & 0b11,
    );
    assert_eq!(tos & 0b11, ECN as u8, "ECN bits should still be set");
}

#[test]
fn dscp_survives_ecn_cmsg_ipv6() {
    assert_dscp_survives(false);
}

#[test]
fn dscp_survives_ecn_cmsg_ipv4() {
    assert_dscp_survives(true);
}
