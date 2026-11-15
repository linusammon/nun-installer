//! Checking for an internet connection and, when there is none, joining Wi-Fi
//! through NetworkManager.

use crate::secret::Secret;
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiNetwork {
    pub ssid: String,
    /// 0-100.
    pub signal: u8,
    pub secured: bool,
    /// Currently connected.
    pub in_use: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("NetworkManager (nmcli) is not available")]
    NoNetworkManager,
    #[error("nmcli failed: {0}")]
    Nmcli(String),
}

/// Where `nmcli` lives. Found through `PATH` unless `NUN_NMCLI` says otherwise.
pub fn nmcli_from_env() -> PathBuf {
    std::env::var_os("NUN_NMCLI").map_or_else(|| PathBuf::from("nmcli"), PathBuf::from)
}

fn run_nmcli(nmcli: &Path, args: &[&str], stdin: Option<&Secret>) -> Result<String, NetError> {
    let mut child = Command::new(nmcli)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                NetError::NoNetworkManager
            } else {
                NetError::Nmcli(e.to_string())
            }
        })?;
    if let (Some(secret), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let _ = writeln!(pipe, "{}", secret.expose());
    }
    let out = child
        .wait_with_output()
        .map_err(|e| NetError::Nmcli(e.to_string()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = err.trim();
        let msg = if msg.is_empty() {
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        } else {
            msg.to_owned()
        };
        Err(NetError::Nmcli(msg))
    }
}

pub fn nmcli_available(nmcli: &Path) -> bool {
    run_nmcli(nmcli, &["--version"], None).is_ok()
}

/// Scan and list visible networks, strongest first, connected one on top.
pub fn wifi_scan(nmcli: &Path) -> Result<Vec<WifiNetwork>, NetError> {
    let out = run_nmcli(
        nmcli,
        &[
            "-t",
            "-f",
            "IN-USE,SSID,SIGNAL,SECURITY",
            "dev",
            "wifi",
            "list",
            "--rescan",
            "yes",
        ],
        None,
    )?;
    Ok(parse_wifi_list(&out))
}

/// Join a network. The password goes in through stdin (`--ask`), not the
/// command line, so other processes can't read it.
pub fn wifi_connect(nmcli: &Path, ssid: &str, password: Option<&Secret>) -> Result<(), NetError> {
    let args = ["--ask", "dev", "wifi", "connect", ssid];
    run_nmcli(nmcli, &args, password).map(|_| ())
}

/// Split one line of `nmcli -t` output on unescaped `:`.
fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    fields.last_mut().expect("never empty").push(next);
                }
            }
            ':' => fields.push(String::new()),
            c => fields.last_mut().expect("never empty").push(c),
        }
    }
    fields
}

/// Parse `nmcli -t -f IN-USE,SSID,SIGNAL,SECURITY dev wifi list`.
pub fn parse_wifi_list(terse: &str) -> Vec<WifiNetwork> {
    let mut nets: Vec<WifiNetwork> = Vec::new();
    for line in terse.lines() {
        let f = split_terse(line);
        let [in_use, ssid, signal, security] = f.as_slice() else {
            continue;
        };
        if ssid.is_empty() {
            continue; // hidden network
        }
        let net = WifiNetwork {
            ssid: ssid.clone(),
            signal: signal.parse().unwrap_or(0),
            secured: !security.is_empty() && security != "--",
            in_use: in_use == "*",
        };
        match nets.iter_mut().find(|n| n.ssid == net.ssid) {
            // Same network seen from several access points: keep the best one.
            Some(existing) => {
                if net.signal > existing.signal || net.in_use {
                    let in_use = existing.in_use || net.in_use;
                    *existing = WifiNetwork { in_use, ..net };
                }
            }
            None => nets.push(net),
        }
    }
    nets.sort_by(|a, b| {
        b.in_use
            .cmp(&a.in_use)
            .then(b.signal.cmp(&a.signal))
            .then(a.ssid.cmp(&b.ssid))
    });
    nets
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn parses_nmcli_terse_output() {
        let out = "\
 :Cafe WiFi:55:WPA2\n\
*:Home\\:Net:78:WPA2 WPA3\n\
 :Open Guest:30:--\n\
 ::40:WPA2\n\
 :Cafe WiFi:80:WPA2\n\
 :Back\\\\slash:10:WPA1\n";
        let nets = parse_wifi_list(out);
        let names: Vec<_> = nets.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(
            names,
            ["Home:Net", "Cafe WiFi", "Open Guest", "Back\\slash"]
        );
        assert!(nets[0].in_use && nets[0].secured);
        assert_eq!(nets[1].signal, 80, "keeps the strongest duplicate");
        assert!(!nets[2].secured);
    }

    #[test]
    fn ignores_malformed_lines() {
        assert!(parse_wifi_list("garbage\n\n:::\n").is_empty());
    }

    #[test]
    fn splits_escaped_colons() {
        assert_eq!(split_terse("a\\:b:c"), ["a:b", "c"]);
        assert_eq!(split_terse(""), [""]);
    }

    #[test]
    fn connects_to_a_listening_port_and_not_a_closed_one() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let open = listener.local_addr().unwrap().port();
        assert!(can_connect("127.0.0.1", open, Duration::from_secs(2)));
        drop(listener);
        assert!(!can_connect("127.0.0.1", open, Duration::from_secs(2)));
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
        assert!(Reachability {
            github: true,
            cache: true
        }
        .online());
        assert!(!Reachability {
            github: true,
            cache: false
        }
        .online());
        assert!(!Reachability {
            github: false,
            cache: true
        }
        .online());
    }

    fn fake_nmcli(tag: &str, body: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("nun-nm-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("nmcli");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, p)
    }

    /// See the ETXTBSY note in the keymap tests.
    fn retry<T>(mut f: impl FnMut() -> Result<T, NetError>) -> Result<T, NetError> {
        for _ in 0..50 {
            match f() {
                Err(NetError::Nmcli(m)) if m.contains("Text file busy") => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                other => return other,
            }
        }
        f()
    }

    #[test]
    fn scan_runs_nmcli_and_parses() {
        let (dir, nmcli) = fake_nmcli("scan", "echo \"*:Home:70:WPA2\"");
        let nets = retry(|| wifi_scan(&nmcli)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(nets.len(), 1);
        assert_eq!(nets[0].ssid, "Home");
    }

    #[test]
    fn connect_sends_password_on_stdin_never_in_args() {
        let (dir, nmcli) = fake_nmcli(
            "connect",
            &format!(
                "echo \"$@\" > {d}/args\nread pw\necho \"$pw\" > {d}/stdin",
                d = "$(dirname \"$0\")"
            ),
        );
        let secret = Secret::new("s3cret pass");
        retry(|| wifi_connect(&nmcli, "Home Net", Some(&secret))).unwrap();
        let args = std::fs::read_to_string(dir.join("args")).unwrap();
        let stdin = std::fs::read_to_string(dir.join("stdin")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(args.trim(), "--ask dev wifi connect Home Net");
        assert!(!args.contains("s3cret"));
        assert_eq!(stdin.trim(), "s3cret pass");
    }

    #[test]
    fn nmcli_errors_carry_their_message() {
        let (dir, nmcli) = fake_nmcli("err", "echo 'Error: Secrets were required' >&2; exit 4");
        let err = retry(|| wifi_connect(&nmcli, "x", None)).unwrap_err();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(err.to_string().contains("Secrets were required"), "{err}");
    }

    #[test]
    fn missing_nmcli_is_reported_as_such() {
        let missing = Path::new("/definitely/not/here/nmcli");
        assert!(!nmcli_available(missing));
        assert!(matches!(
            wifi_scan(missing),
            Err(NetError::NoNetworkManager)
        ));
    }
}
