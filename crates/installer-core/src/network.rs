//! Checking for an internet connection.
//!
//! Joining Wi-Fi from inside the installer was written once (nmcli scan and
//! connect, commit f4a4e34) and removed again because no screen used it.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::time::Duration;

/// Hosts an online install needs: flake inputs and the binary cache.
pub const GITHUB: (&str, u16) = ("github.com", 443);
pub const CACHE: (&str, u16) = ("cache.nixos.org", 443);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reachability {
    pub github: bool,
    pub cache: bool,
}

impl Reachability {
    pub fn online(&self) -> bool {
        self.github && self.cache
    }
}

/// Resolve and connect within `timeout`. DNS has no timeout of its own, so the
/// whole attempt runs on a thread we stop waiting for.
pub fn can_connect(host: &str, port: u16, timeout: Duration) -> bool {
    let (tx, rx) = mpsc::channel();
    let host = host.to_owned();
    std::thread::spawn(move || {
        let ok = (host.as_str(), port)
            .to_socket_addrs()
            .map(|mut addrs| addrs.any(|a| TcpStream::connect_timeout(&a, timeout).is_ok()))
            .unwrap_or(false);
        let _ = tx.send(ok);
    });
    rx.recv_timeout(timeout + Duration::from_secs(1))
        .unwrap_or(false)
}

pub fn check_online() -> Reachability {
    let timeout = Duration::from_secs(4);
    let github = std::thread::spawn(move || can_connect(GITHUB.0, GITHUB.1, timeout));
    let cache = can_connect(CACHE.0, CACHE.1, timeout);
    Reachability {
        github: github.join().unwrap_or(false),
        cache,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn connects_to_a_listening_port_and_not_a_closed_one() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let open = listener.local_addr().unwrap().port();
        assert!(can_connect("127.0.0.1", open, Duration::from_secs(2)));
        // A port that was just closed can be reused as another test's ephemeral
        // source port and even connect to itself, so use one that never is.
        assert!(!can_connect("127.0.0.1", 1, Duration::from_secs(2)));
    }

    #[test]
    fn unresolvable_host_is_not_reachable() {
        assert!(!can_connect(
            "nonexistent.invalid",
            443,
            Duration::from_secs(2)
        ));
    }

    #[test]
    fn online_needs_both_hosts() {
        let r = |github, cache| Reachability { github, cache };
        assert!(r(true, true).online());
        assert!(!r(true, false).online());
        assert!(!r(false, true).online());
    }
}
