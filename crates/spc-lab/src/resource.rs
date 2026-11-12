//! Port and storage allocation.
//!
//! Every listener gets a recorded endpoint before a dependent process starts.
//! Every pool gets its own Redis instance and Postgres database; the plan
//! rejects a collision instead of letting two runs share state.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ids::{PoolId, RunId};
use crate::scenario::Scenario;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

impl Endpoint {
    pub fn loopback(port: u16) -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port,
        }
    }

    pub fn socket_addr(&self) -> std::io::Result<SocketAddr> {
        let ip: IpAddr = self.host.parse().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad endpoint host")
        })?;
        Ok(SocketAddr::new(ip, self.port))
    }

    pub fn parse(value: &str) -> std::io::Result<Self> {
        let (host, port) = value.rsplit_once(':').ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "endpoint has no port")
        })?;
        let port = port.parse::<u16>().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "endpoint has a bad port")
        })?;
        if port == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "an allocated endpoint cannot retain port 0",
            ));
        }
        Ok(Self {
            host: host.to_string(),
            port,
        })
    }

    pub fn display(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedisRuntime {
    pub id: String,
    pub owner_pool_id: PoolId,
    pub port: u16,
    pub dir: PathBuf,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostgresRuntime {
    pub id: String,
    pub owner_pool_id: PoolId,
    pub database: String,
    pub role: String,
    pub port: u16,
    pub dir: PathBuf,
    pub url: String,
}

#[derive(Debug, Default)]
pub struct PortAllocator {
    reserved: BTreeSet<u16>,
    locks: Vec<File>,
}

impl PortAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the OS for a free loopback port, then keep a cross-process advisory
    /// lock on it for the run lifetime. The file lock is kernel-backed, so a
    /// crashed owner releases it automatically; the manifest check still sees
    /// the requested/actual endpoints.
    pub fn allocate(&mut self, requested: u16) -> std::io::Result<u16> {
        if requested != 0 {
            if self.reserved.contains(&requested) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!("port {requested} was already reserved"),
                ));
            }
            let probe =
                TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), requested))?;
            drop(probe);
            let lock = reserve_port(requested)?;
            self.reserved.insert(requested);
            self.locks.push(lock);
            return Ok(requested);
        }
        for _ in 0..128 {
            let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))?;
            let port = listener.local_addr()?.port();
            drop(listener);
            if self.reserved.contains(&port) {
                continue;
            }
            match reserve_port(port) {
                Ok(lock) => {
                    self.reserved.insert(port);
                    self.locks.push(lock);
                    return Ok(port);
                }
                Err(_) => continue,
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            "could not allocate a unique loopback port",
        ))
    }

    fn into_locks(self) -> Vec<File> {
        self.locks
    }
}

fn reserve_port(port: u16) -> std::io::Result<File> {
    let root = std::env::var("SPC_LAB_PORT_LOCK_ROOT")
        .ok()
        .or_else(|| {
            std::env::var("XDG_RUNTIME_DIR")
                .ok()
                .map(|dir| format!("{dir}/spc-lab-port-locks"))
        })
        .or_else(|| {
            std::env::var("TMPDIR")
                .ok()
                .map(|dir| format!("{dir}/spc-lab-port-locks"))
        })
        .unwrap_or_else(|| "/tmp/spc-lab-port-locks".to_string());
    let root = PathBuf::from(root);
    std::fs::create_dir_all(&root)?;
    let _ = std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700));
    let path = root.join(format!("{port}.lock"));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)?;
    match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(file),
        Err(error)
            if error == rustix::io::Errno::WOULDBLOCK || error == rustix::io::Errno::AGAIN =>
        {
            Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                format!("port {port} is reserved by another run"),
            ))
        }
        Err(error) => Err(std::io::Error::from(error)),
    }
}

#[derive(Debug)]
pub struct PlannedResources {
    pub listeners: BTreeMap<String, Endpoint>,
    pub redis_instances: Vec<RedisRuntime>,
    pub postgres_databases: Vec<PostgresRuntime>,
    pub temp_dirs: Vec<PathBuf>,
    pub port_locks: Vec<File>,
}

impl PlannedResources {
    pub fn listener(&self, key: &str) -> Option<&Endpoint> {
        self.listeners.get(key)
    }

    pub fn redis_for_pool(&self, pool: &PoolId) -> Option<&RedisRuntime> {
        self.redis_instances
            .iter()
            .find(|r| &r.owner_pool_id == pool)
    }

    pub fn postgres_for_pool(&self, pool: &PoolId) -> Option<&PostgresRuntime> {
        self.postgres_databases
            .iter()
            .find(|p| &p.owner_pool_id == pool)
    }
}

/// Allocate every requested listener and storage endpoint for one run.
///
/// `operator_ui` (the host runtime's `--operator-ui`) additionally plans one
/// loopback listener per front under the existing `front.<id>.operator` key,
/// leased by the same cross-process port lock as every other listener. The
/// key is absent unless the run opted in; nothing but the rendered pool
/// `[operator]` section and the hub inventory consumes it.
pub fn plan_resources(
    scenario: &Scenario,
    run_id: &RunId,
    run_dir: &Path,
    operator_ui: bool,
) -> std::io::Result<PlannedResources> {
    let mut allocator = PortAllocator::new();
    let mut listeners = BTreeMap::new();
    let mut keys = BTreeSet::new();

    let mut add =
        |key: String, requested: u16, allocator: &mut PortAllocator| -> std::io::Result<()> {
            if !keys.insert(key.clone()) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!("duplicate listener key {key}"),
                ));
            }
            let port = allocator.allocate(requested)?;
            listeners.insert(key, Endpoint::loopback(port));
            Ok(())
        };

    for wallet in &scenario.wallet_agents {
        add(
            format!("wallet.{}.status", wallet.identity_id),
            0,
            &mut allocator,
        )?;
    }
    for node in &scenario.nodes {
        let requested = |value: &str| -> std::io::Result<u16> {
            let (_, port) = value.rsplit_once(':').ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "listener has no port")
            })?;
            port.parse::<u16>().map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "listener has a bad port")
            })
        };
        add(
            format!("node.{}.p2p", node.id),
            requested(&node.p2p.listen)?,
            &mut allocator,
        )?;
        add(
            format!("node.{}.rpc", node.id),
            requested(&node.rpc.listen)?,
            &mut allocator,
        )?;
        add(
            format!("node.{}.tdp", node.id),
            requested(&node.tdp.listen)?,
            &mut allocator,
        )?;
    }
    for front in &scenario.fronts {
        for (name, listener) in [
            ("sv2", front.listen.sv2.as_deref()),
            ("sv1", front.listen.sv1.as_deref()),
            ("jd", front.listen.jd.as_deref()),
            ("api", front.listen.api.as_deref()),
        ] {
            if let Some(listener) = listener {
                let (_, port) = listener.rsplit_once(':').ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "front listener has no port",
                    )
                })?;
                let port = port.parse::<u16>().map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "front listener has a bad port",
                    )
                })?;
                add(format!("front.{}.{}", front.id, name), port, &mut allocator)?;
            }
        }
        if operator_ui {
            add(format!("front.{}.operator", front.id), 0, &mut allocator)?;
        }
    }
    for ark in &scenario.ark_backends {
        let (_, port) = ark.listen.rsplit_once(':').ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Ark listener has no port")
        })?;
        let port = port.parse::<u16>().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Ark listener has a bad port",
            )
        })?;
        add(format!("ark.{}", ark.id), port, &mut allocator)?;
        if ark.local.is_some() {
            add(format!("ark.{}.postgres", ark.id), 0, &mut allocator)?;
            add(format!("ark.{}.admin", ark.id), 0, &mut allocator)?;
        }
    }

    let mut redis_instances = Vec::new();
    for redis in &scenario.resources.redis_instances {
        let port = allocator.allocate(redis.port)?;
        let dir = safe_join(run_dir, &redis.dir)?;
        std::fs::create_dir_all(&dir)?;
        let url = match &redis.password {
            Some(password) => format!("redis://:{password}@127.0.0.1:{port}/0"),
            None => format!("redis://127.0.0.1:{port}/0"),
        };
        redis_instances.push(RedisRuntime {
            id: redis.id.clone(),
            owner_pool_id: redis.owner_pool_id.clone(),
            port,
            dir,
            url,
        });
    }

    let mut postgres_databases = Vec::new();
    for pg in &scenario.resources.postgres_databases {
        let port = allocator.allocate(pg.port)?;
        let dir = run_dir.join("postgres").join(&pg.id);
        std::fs::create_dir_all(&dir)?;
        postgres_databases.push(PostgresRuntime {
            id: pg.id.clone(),
            owner_pool_id: pg.owner_pool_id.clone(),
            database: pg.database.clone(),
            role: pg.role.clone(),
            port,
            dir,
            url: match &pg.password {
                Some(password) => format!(
                    "postgres://{}:{}@127.0.0.1:{port}/{}",
                    pg.role,
                    urlencoding(password),
                    pg.database
                ),
                None => format!("postgres://{}@127.0.0.1:{port}/{}", pg.role, pg.database),
            },
        });
    }

    let temp_dirs = vec![
        run_dir.join("tmp"),
        run_dir.join(format!("tmp/{}", run_id.as_str())),
    ];
    for dir in &temp_dirs {
        std::fs::create_dir_all(dir)?;
    }

    Ok(PlannedResources {
        listeners,
        redis_instances,
        postgres_databases,
        temp_dirs,
        port_locks: allocator.into_locks(),
    })
}

fn urlencoding(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn safe_join(run_dir: &Path, relative: &str) -> std::io::Result<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute() || relative.split('/').any(|part| part == "..") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("resource path {relative:?} escapes the run directory"),
        ));
    }
    Ok(run_dir.join(path))
}
