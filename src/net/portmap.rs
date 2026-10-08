use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use igd_next::{PortMappingProtocol, SearchOptions, search_gateway};

use super::Event;

const LEASE: Duration = Duration::from_secs(60 * 60);
const RENEW_EVERY: Duration = Duration::from_secs(30 * 60);
const SEARCH_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PortMap {
    #[default]
    Off,
    Searching,
    Mapped {
        port: u16,
        external: Option<IpAddr>,
    },
    NoRouter,
    Failed(String),
}

/// Keeps a UPnP mapping for the listening port alive on its own thread, and removes it when dropped.
pub struct PortMapper {
    stop: Arc<AtomicBool>,
}

impl PortMapper {
    pub fn start(port: u16, events: Sender<Event>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let _ = thread::Builder::new()
            .name("portmap".into())
            .spawn(move || run(port, &events, &flag));
        Self { stop }
    }
}

impl Drop for PortMapper {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn run(port: u16, events: &Sender<Event>, stop: &AtomicBool) {
    let send = |state: PortMap| {
        let _ = events.send(Event::PortMap(state));
    };
    send(PortMap::Searching);
    let mut options = SearchOptions::default();
    options.timeout = Some(SEARCH_TIMEOUT);
    let Ok(gateway) = search_gateway(options) else {
        send(PortMap::NoRouter);
        return;
    };
    let Some(local) = local_ip_towards(gateway.addr) else {
        send(PortMap::Failed(
            "could not find this computer's address".into(),
        ));
        return;
    };

    let mut renewed: Option<Instant> = None;
    while !stop.load(Ordering::Relaxed) {
        if renewed.is_none_or(|at| at.elapsed() >= RENEW_EVERY) {
            renewed = Some(Instant::now());
            let result = gateway.add_port(
                PortMappingProtocol::TCP,
                port,
                SocketAddr::new(local, port),
                LEASE.as_secs() as u32,
                "bawkseek",
            );
            send(match result {
                Ok(()) => PortMap::Mapped {
                    port,
                    external: gateway.get_external_ip().ok(),
                },
                Err(err) => PortMap::Failed(err.to_string().to_lowercase()),
            });
        }
        thread::sleep(Duration::from_secs(1));
    }
    let _ = gateway.remove_port(PortMappingProtocol::TCP, port);
}

/// The local address the OS would use to reach the router, which is the one the mapping must point at.
fn local_ip_towards(router: SocketAddr) -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect(router).ok()?;
    Some(socket.local_addr().ok()?.ip())
}
