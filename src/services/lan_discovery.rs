//! Opt-in local-link DNS-SD advertisement owned by the HTTP serving lifecycle.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
    time::Duration,
};

use argon2::password_hash::rand_core::{OsRng, RngCore};
use mdns_sd::{
    DaemonEvent, DaemonStatus, IfKind, IfPredicate, ServiceDaemon, ServiceInfo, UnregisterStatus,
};
use thiserror::Error;
use tokio::task::JoinHandle;

use crate::{
    config::LanDiscoveryConfig,
    storage::{
        file_store::{FileStore, FileStoreError},
        paths::PathSafetyError,
    },
};

const SERVICE_TYPE: &str = "_teatro-games._tcp.local.";
const DISCOVERY_ID_FILE: &str = "discovery-id";
const DISCOVERY_ID_BYTES: usize = 16;
const DISCOVERY_ID_HEX_BYTES: usize = DISCOVERY_ID_BYTES * 2;
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Error)]
pub enum LanDiscoveryError {
    #[error(transparent)]
    FileStore(#[from] FileStoreError),

    #[error(transparent)]
    Mdns(#[from] mdns_sd::Error),

    #[error("the existing discovery ID is malformed")]
    InvalidDiscoveryId,
}

pub struct LanDiscovery {
    daemon: Option<ServiceDaemon>,
    fullname: String,
    monitor: JoinHandle<()>,
}

impl LanDiscovery {
    pub async fn start(
        config: &LanDiscoveryConfig,
        data_dir: &Path,
        file_store: &FileStore,
        listener_addr: SocketAddr,
    ) -> Result<Option<Self>, LanDiscoveryError> {
        if !config.enabled {
            return Ok(None);
        }

        let IpAddr::V4(listener_ip) = listener_addr.ip() else {
            tracing::warn!(
                %listener_addr,
                "LAN discovery requires an IPv4 listener; HTTP remains available"
            );
            return Ok(None);
        };
        if listener_ip.is_loopback() {
            tracing::warn!(
                %listener_addr,
                "LAN discovery is enabled but the HTTP listener is loopback-only; HTTP remains available"
            );
            return Ok(None);
        }

        let discovery_id = load_or_create_discovery_id(file_store, data_dir).await?;
        let service = build_service(config, &discovery_id, listener_ip, listener_addr.port())?;
        let fullname = service.get_fullname().to_string();
        let host_name = service.get_hostname().to_string();
        let daemon = ServiceDaemon::new()?;
        let monitor_receiver = match daemon.monitor() {
            Ok(receiver) => receiver,
            Err(error) => {
                let _ = daemon.shutdown();
                return Err(error.into());
            }
        };
        let monitor = tokio::spawn(async move {
            while let Ok(event) = monitor_receiver.recv_async().await {
                match event {
                    DaemonEvent::Error(_) => tracing::warn!(
                        "LAN discovery daemon reported an error; HTTP remains available"
                    ),
                    DaemonEvent::NameChange(change)
                        if change.original.eq_ignore_ascii_case(&host_name) =>
                    {
                        tracing::warn!("LAN discovery changed its hostname after an mDNS conflict");
                    }
                    DaemonEvent::NameChange(_) => {
                        tracing::info!("LAN discovery renamed its instance after an mDNS conflict");
                    }
                    _ => {}
                }
            }
        });

        let setup = daemon
            .disable_interface(IfKind::IPv6)
            .and_then(|()| daemon.disable_interface(IfKind::LoopbackV4))
            .and_then(|()| daemon.register(service));
        if let Err(error) = setup {
            monitor.abort();
            let _ = daemon.shutdown();
            return Err(error.into());
        }

        tracing::info!(
            service_type = SERVICE_TYPE,
            port = listener_addr.port(),
            "LAN discovery advertisement started"
        );
        Ok(Some(Self {
            daemon: Some(daemon),
            fullname,
            monitor,
        }))
    }

    pub async fn shutdown(mut self) {
        let Some(daemon) = self.daemon.take() else {
            return;
        };

        match daemon.unregister(&self.fullname) {
            Ok(receiver) => {
                match tokio::time::timeout(CLEANUP_TIMEOUT, receiver.recv_async()).await {
                    Ok(Ok(UnregisterStatus::OK)) => {
                        tracing::info!("LAN discovery advertisement unregistered");
                    }
                    Ok(Ok(UnregisterStatus::NotFound)) => {
                        tracing::warn!("LAN discovery advertisement was already absent");
                    }
                    Ok(Err(_)) => {
                        tracing::warn!("LAN discovery unregister response channel closed")
                    }
                    Err(_) => tracing::warn!("LAN discovery unregister timed out"),
                }
            }
            Err(_) => tracing::warn!("LAN discovery unregister request failed"),
        }

        match daemon.shutdown() {
            Ok(receiver) => {
                match tokio::time::timeout(CLEANUP_TIMEOUT, receiver.recv_async()).await {
                    Ok(Ok(DaemonStatus::Shutdown)) => {
                        tracing::info!("LAN discovery daemon stopped");
                    }
                    Ok(Ok(_)) => tracing::warn!("LAN discovery daemon returned unexpected status"),
                    Ok(Err(_)) => tracing::warn!("LAN discovery shutdown response channel closed"),
                    Err(_) => tracing::warn!("LAN discovery shutdown timed out"),
                }
            }
            Err(_) => tracing::warn!("LAN discovery shutdown request failed"),
        }
        self.monitor.abort();
    }
}

impl Drop for LanDiscovery {
    fn drop(&mut self) {
        if let Some(daemon) = self.daemon.take() {
            let _ = daemon.unregister(&self.fullname);
            let _ = daemon.shutdown();
        }
        self.monitor.abort();
    }
}

fn build_service(
    config: &LanDiscoveryConfig,
    discovery_id: &str,
    listener_ip: Ipv4Addr,
    port: u16,
) -> Result<ServiceInfo, mdns_sd::Error> {
    let short_id = &discovery_id[..10];
    let instance_name = format!("{} {short_id}", config.name);
    let host_name = format!("teatro-{short_id}.local.");
    let txt_id = format!("ttr-{discovery_id}");
    let properties = [("api", "2"), ("id", txt_id.as_str()), ("scheme", "http")];

    if listener_ip.is_unspecified() {
        let mut service = ServiceInfo::new(
            SERVICE_TYPE,
            &instance_name,
            &host_name,
            "",
            port,
            &properties[..],
        )?;
        service.set_interfaces(vec![IfKind::Predicate(IfPredicate::new(|interface| {
            interface.ip().is_ipv4() && !interface.is_loopback()
        }))]);
        Ok(service.enable_addr_auto())
    } else {
        let mut service = ServiceInfo::new(
            SERVICE_TYPE,
            &instance_name,
            &host_name,
            IpAddr::V4(listener_ip),
            port,
            &properties[..],
        )?;
        service.set_interfaces(vec![IfKind::Addr(IpAddr::V4(listener_ip))]);
        Ok(service)
    }
}

async fn load_or_create_discovery_id(
    file_store: &FileStore,
    data_dir: &Path,
) -> Result<String, LanDiscoveryError> {
    let _lock = file_store.lock_root(data_dir).await?;
    match file_store
        .read_existing(
            data_dir,
            DISCOVERY_ID_FILE,
            (DISCOVERY_ID_HEX_BYTES + 1) as u64,
        )
        .await
    {
        Ok(bytes) => parse_discovery_id(&bytes),
        Err(error) if is_not_found(&error) => {
            let id = generate_discovery_id();
            file_store
                .write_new_atomic(data_dir, DISCOVERY_ID_FILE, format!("{id}\n").as_bytes())
                .await?;
            Ok(id)
        }
        Err(error) => Err(error.into()),
    }
}

fn parse_discovery_id(bytes: &[u8]) -> Result<String, LanDiscoveryError> {
    let text = std::str::from_utf8(bytes).map_err(|_| LanDiscoveryError::InvalidDiscoveryId)?;
    let id = text.strip_suffix('\n').unwrap_or(text);
    if id.len() == DISCOVERY_ID_HEX_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(id.to_string())
    } else {
        Err(LanDiscoveryError::InvalidDiscoveryId)
    }
}

fn generate_discovery_id() -> String {
    let mut bytes = [0_u8; DISCOVERY_ID_BYTES];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_not_found(error: &FileStoreError) -> bool {
    matches!(
        error,
        FileStoreError::Path(PathSafetyError::Io(error))
            if error.kind() == std::io::ErrorKind::NotFound
    )
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use tempfile::TempDir;

    use super::{
        DISCOVERY_ID_FILE, LanDiscovery, build_service, load_or_create_discovery_id,
        parse_discovery_id,
    };
    use crate::{config::LanDiscoveryConfig, storage::file_store::FileStore};

    #[tokio::test]
    async fn discovery_id_is_created_once_and_malformed_values_fail_closed() {
        let temp = TempDir::new().unwrap();
        let store = FileStore::new();
        let first = load_or_create_discovery_id(&store, temp.path())
            .await
            .unwrap();
        let second = load_or_create_discovery_id(&store, temp.path())
            .await
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 32);

        std::fs::write(temp.path().join(DISCOVERY_ID_FILE), b"INVALID\n").unwrap();
        assert!(
            load_or_create_discovery_id(&store, temp.path())
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(temp.path().join(DISCOVERY_ID_FILE)).unwrap(),
            b"INVALID\n"
        );
    }

    #[test]
    fn discovery_id_parser_accepts_only_lowercase_hex_with_one_optional_newline() {
        let id = b"0123456789abcdef0123456789abcdef";
        assert_eq!(parse_discovery_id(id).unwrap().as_bytes(), id);
        assert!(parse_discovery_id(b"0123456789abcdef0123456789abcdef\n").is_ok());
        assert!(parse_discovery_id(b"0123456789ABCDEF0123456789ABCDEF").is_err());
        assert!(parse_discovery_id(b"0123456789abcdef0123456789abcdef\n\n").is_err());
    }

    #[test]
    fn service_record_is_minimal_ipv4_and_uses_the_actual_port() {
        let config = LanDiscoveryConfig {
            enabled: true,
            name: "Living Room".to_string(),
        };
        let address = Ipv4Addr::new(192, 0, 2, 5);
        let service =
            build_service(&config, "0123456789abcdef0123456789abcdef", address, 43210).unwrap();

        assert_eq!(service.get_type(), "_teatro-games._tcp.local.");
        assert_eq!(service.get_hostname(), "teatro-0123456789.local.");
        assert_eq!(service.get_port(), 43210);
        assert_eq!(service.get_property_val_str("api"), Some("2"));
        assert_eq!(
            service.get_property_val_str("id"),
            Some("ttr-0123456789abcdef0123456789abcdef")
        );
        assert_eq!(service.get_property_val_str("scheme"), Some("http"));
        assert_eq!(service.get_properties().iter().count(), 3);
        assert_eq!(service.get_addresses().len(), 1);
        assert!(service.get_addresses().contains(&IpAddr::V4(address)));
    }

    #[tokio::test]
    async fn disabled_or_loopback_discovery_does_not_create_an_identity() {
        let temp = TempDir::new().unwrap();
        let store = FileStore::new();
        let disabled = LanDiscoveryConfig::default();
        assert!(
            LanDiscovery::start(
                &disabled,
                temp.path(),
                &store,
                SocketAddr::from(([0, 0, 0, 0], 4440)),
            )
            .await
            .unwrap()
            .is_none()
        );

        let enabled = LanDiscoveryConfig {
            enabled: true,
            name: "Teatro".to_string(),
        };
        assert!(
            LanDiscovery::start(
                &enabled,
                temp.path(),
                &store,
                SocketAddr::from(([127, 0, 0, 1], 4440)),
            )
            .await
            .unwrap()
            .is_none()
        );
        assert!(!temp.path().join(DISCOVERY_ID_FILE).exists());
    }
}
