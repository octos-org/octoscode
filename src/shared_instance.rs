//! Default local launch: discover/start one WebSocket server per runtime dir.
//! Startup and database ownership are separate locks. A client owns neither
//! the server's lifetime nor the right to replace an unresponsive live owner.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use eyre::{Result, WrapErr, eyre};
use fs2::FileExt;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

const WAIT: Duration = Duration::from_secs(60);
const RECORD: &str = "shared-instance.json";

#[derive(Clone, Debug)]
pub(crate) struct Launch {
    pub command: String,
    pub cwd: PathBuf,
}

// Deliberately no Debug: this record includes a bearer credential.
#[derive(Deserialize)]
pub(crate) struct Instance {
    pub endpoint: String,
    pub auth_token: String,
    version: u32,
    instance_id: String,
    data_dir: PathBuf,
    protocol: String,
}

fn read_instance(dir: &Path) -> Result<Option<Instance>> {
    let path = dir.join(RECORD);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || metadata.len() > 16 * 1024 {
        return Err(eyre!("invalid shared server record"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(eyre!("shared server record must be owner-readable only"));
        }
    }
    let record: Instance = serde_json::from_slice(&std::fs::read(path)?)?;
    validate_record(&record, dir)?;
    Ok(Some(record))
}

fn validate_record(record: &Instance, dir: &Path) -> Result<()> {
    let endpoint = reqwest::Url::parse(&record.endpoint)?;
    if record.version != 1
        || record.protocol != "peer.workspace_team.v1"
        || record.data_dir != dir
        || record.instance_id.is_empty()
        || record.auth_token.is_empty()
        || endpoint.scheme() != "ws"
        || endpoint.host_str() != Some("127.0.0.1")
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.path() != "/api/ui-protocol/ws"
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || endpoint.port().is_none_or(|p| p == 0)
    {
        return Err(eyre!(
            "shared server record does not match this runtime or protocol"
        ));
    }
    Ok(())
}

async fn verify(record: &Instance) -> Result<()> {
    let mut request = record.endpoint.as_str().into_client_request()?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", record.auth_token).parse()?,
    );
    let (mut ws, _) = connect_async(request).await?;
    ws.send(Message::Text(json!({"jsonrpc":"2.0", "id":"shared-instance-probe", "method":"server/instance.get", "params":{}}).to_string().into())).await?;
    while let Some(frame) = ws.next().await {
        let frame = frame?;
        if let Message::Ping(data) = frame {
            ws.send(Message::Pong(data)).await?;
            continue;
        }
        let Message::Text(text) = frame else {
            continue;
        };
        let value: Value = serde_json::from_str(&text)?;
        if value["id"] != "shared-instance-probe" {
            continue;
        }
        let identity = &value["result"];
        if identity["instance_id"] != record.instance_id
            || identity["version"] != 1
            || identity["data_dir"] != json!(record.data_dir)
            || identity["protocol"] != record.protocol
        {
            return Err(eyre!("shared server identity verification failed"));
        }
        let _ = ws.close(None).await;
        return Ok(());
    }
    Err(eyre!("shared server closed before identity verification"))
}

fn open_lock(path: &Path) -> Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    Ok(options.open(path)?)
}

fn spawn(launch: &Launch, dir: &Path) -> Result<std::process::Child> {
    let words =
        shlex::split(&launch.command).ok_or_else(|| eyre!("invalid shared launch command"))?;
    let program = words
        .first()
        .ok_or_else(|| eyre!("empty shared launch command"))?;
    let mut child = Command::new(program);
    child
        .args(&words[1..])
        .args(["--host", "127.0.0.1", "--port", "0", "--cwd"])
        .arg(&launch.cwd)
        .arg("--instance-data-dir")
        .arg(dir)
        .current_dir(&launch.cwd);
    // Unix provisioning already rewrites an off-PATH backend to its absolute
    // path. Preserve PATH order here so an older installer copy cannot shadow
    // the backend whose shared-server support we just checked.
    #[cfg(windows)]
    if let Some(bin) = crate::backend_ensure::install_bin_dir() {
        let mut paths = vec![bin];
        if let Some(path) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&path));
        }
        child.env("PATH", std::env::join_paths(paths)?);
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::{fs::OpenOptionsExt, process::CommandExt};
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        child.process_group(0);
    }
    let log = options.open(dir.join("shared-server.log"))?;
    child
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    child.spawn().wrap_err("start shared Octos server")
}

pub(crate) async fn discover(launch: &Launch) -> Result<Instance> {
    let cwd = launch
        .cwd
        .canonicalize()
        .wrap_err("resolve launch workspace")?;
    let dir = std::env::var_os("OCTOS_INSTANCE_DATA_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::profiles::instance_data_dir_for_launch(Some(&launch.command), &cwd))
        .or_else(|| crate::profiles::solo_data_dir(Some(&launch.command)))
        .ok_or_else(|| eyre!("cannot resolve local runtime directory"))?;
    std::fs::create_dir_all(&dir)?;
    let dir = dir.canonicalize()?;
    let lock = open_lock(&dir.join(".octos-shared-start.lock"))?;
    let deadline = Instant::now() + WAIT;
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(100)).await
            }
            Err(e) => return Err(eyre!("waiting for shared server startup: {e}")),
        }
    }
    // Dropping `lock` releases startup ownership on every return path.
    if let Some(record) = read_instance(&dir)? {
        if tokio::time::timeout(Duration::from_secs(3), verify(&record))
            .await
            .is_ok_and(|r| r.is_ok())
        {
            return Ok(record);
        }
    }
    let owner = open_lock(&dir.join(".octos-serve.lock"))?;
    let mut child = match owner.try_lock_exclusive() {
        Ok(()) => {
            FileExt::unlock(&owner)?;
            Some(spawn(
                &Launch {
                    command: launch.command.clone(),
                    cwd,
                },
                &dir,
            )?)
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => None,
        Err(error) => return Err(error.into()),
    };
    loop {
        if let Some(child) = &mut child {
            if let Some(status) = child.try_wait()? {
                return Err(eyre!(
                    "shared Octos server exited ({status}); see {}",
                    dir.join("shared-server.log").display()
                ));
            }
        }
        if let Some(record) = read_instance(&dir)? {
            if tokio::time::timeout(Duration::from_secs(2), verify(&record))
                .await
                .is_ok_and(|r| r.is_ok())
            {
                return Ok(record);
            }
        }
        if Instant::now() >= deadline {
            return Err(eyre!(
                "Cannot attach to the Octos server owning {}. An older stdio server may still own it; stop that server before retrying. No competing server was started while its ownership lock was held. See {}",
                dir.display(),
                dir.join("shared-server.log").display()
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(dir: &Path) -> Instance {
        Instance {
            endpoint: "ws://127.0.0.1:50123/api/ui-protocol/ws".into(),
            auth_token: "secret".into(),
            version: 1,
            instance_id: "instance-a".into(),
            data_dir: dir.to_owned(),
            protocol: "peer.workspace_team.v1".into(),
        }
    }
    #[test]
    fn discovery_rejects_remote_or_wrong_runtime_before_sending_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let mut instance = record(dir.path());
        validate_record(&instance, dir.path()).unwrap();
        instance.endpoint = "ws://example.com:80/api/ui-protocol/ws".into();
        assert!(validate_record(&instance, dir.path()).is_err());
        instance = record(Path::new("/another/runtime"));
        assert!(validate_record(&instance, dir.path()).is_err());
    }
    #[tokio::test]
    async fn shared_discovery_checks_server_identity_and_concurrent_clients_reuse_owner() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().canonicalize().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut instance = record(&dir);
        instance.endpoint = format!("ws://127.0.0.1:{port}/api/ui-protocol/ws");
        let wire = json!({"endpoint":instance.endpoint,"auth_token":instance.auth_token,
            "version":1,"instance_id":instance.instance_id,"data_dir":dir,"protocol":instance.protocol});
        let path = dir.join(RECORD);
        std::fs::write(&path, serde_json::to_vec(&wire).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let owner = open_lock(&dir.join(".octos-serve.lock")).unwrap();
        owner.try_lock_exclusive().unwrap();
        let expected = json!({"instance_id":instance.instance_id,"version":1,"data_dir":dir,"protocol":instance.protocol});
        let server = tokio::spawn(async move {
            for _ in 0..3 {
                let (socket, _) = listener.accept().await.unwrap();
                let expected = expected.clone();
                tokio::spawn(async move {
                    let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
                    let Message::Text(request) = socket.next().await.unwrap().unwrap() else {
                        panic!("request")
                    };
                    let request: Value = serde_json::from_str(&request).unwrap();
                    socket
                        .send(Message::Text(
                            json!({"jsonrpc":"2.0","id":request["id"],"result":expected})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                });
            }
        });
        let launch = Launch {
            command: format!(
                "program-that-must-not-be-spawned serve --shared --solo --data-dir {}",
                shlex::try_quote(dir.to_str().unwrap()).unwrap()
            ),
            cwd: dir.clone(),
        };
        let (a, b) = tokio::join!(discover(&launch), discover(&launch));
        assert_eq!(a.unwrap().instance_id, b.unwrap().instance_id);
        let mut forged = instance;
        forged.instance_id = "wrong-instance".into();
        assert!(verify(&forged).await.is_err());
        server.await.unwrap();
    }
    /// Cross-repository check, explicitly run with the matching backend build.
    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "requires OCTOS_WORKSPACE_TEST_BINARY pointing to a matching Octos build"]
    async fn shared_discovery_two_cold_launches_start_one_real_server() {
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Ok(bytes) = std::fs::read(self.0.join(RECORD)) {
                    if let Ok(record) = serde_json::from_slice::<Value>(&bytes) {
                        if let Some(pid) = record["pid"]
                            .as_u64()
                            .filter(|pid| *pid > 1 && *pid <= i32::MAX as u64)
                        {
                            // This fresh private fixture directory is owned by this test.
                            let _ = Command::new("kill")
                                .args(["-TERM", &pid.to_string()])
                                .status();
                        }
                    }
                }
            }
        }
        let binary = std::env::var("OCTOS_WORKSPACE_TEST_BINARY").expect("matching backend path");
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let _cleanup = Cleanup(root_path.clone());
        let launch = Launch {
            command: format!(
                "{} serve --shared --solo --data-dir {}",
                shlex::try_quote(&binary).unwrap(),
                shlex::try_quote(root_path.to_str().unwrap()).unwrap()
            ),
            cwd: root_path.clone(),
        };
        let (a, b) = tokio::join!(discover(&launch), discover(&launch));
        let a = a.expect("first launch");
        let b = b.expect("second launch");
        assert_eq!(a.instance_id, b.instance_id);
        assert_eq!(a.endpoint, b.endpoint);
        let owner = open_lock(&root_path.join(".octos-serve.lock")).unwrap();
        assert!(owner.try_lock_exclusive().is_err());
        verify(&a).await.unwrap();
        verify(&b).await.unwrap();
    }
}
