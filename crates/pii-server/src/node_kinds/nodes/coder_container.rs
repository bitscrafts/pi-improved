//! Container runtime for `coder` nodes (spec 209): the agent runs inside a prebuilt image through podman, so the
//! host carries no agent binary and pii installs nothing.
//!
//! Pure helpers only: the argv a run execs, the mount guard, and the 0600 env file that carries `env_pass`
//! values without putting them on an argv. The spawn and the wait stay in [`super::coder`].

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// The directory inside the container the workspace is mounted at, and the agent's working directory.
pub const CONTAINER_WORKDIR: &str = "/work";

/// Where podman lives in production; the factory's default, overridable for tests.
pub const PODMAN_BIN: &str = "/usr/bin/podman";

/// One container run: the image and the mounts and the network it is given, all taken from the graph.
pub struct ContainerRun<'a> {
    /// The podman binary the run execs.
    pub podman: &'a str,
    /// The prebuilt image, by the tag or digest the graph names.
    pub image: &'a str,
    /// The host workspace mounted at [`CONTAINER_WORKDIR`]; the only writable path.
    pub workdir: &'a str,
    /// The env file holding the `env_pass` values; read by podman at start, removed by the caller after.
    pub env_file: &'a str,
    /// `none` unless the graph declares a network grant.
    pub network: &'a str,
    /// The run's id, labelled on the container so a killed run's container can be found.
    pub run_id: &'a str,
}

/// The argv a coder run execs: `podman run --rm` with the workspace mounted, the env file, the network and labels.
///
/// The image and the agent's own argv come last; the agent binary is named by the image, never by the host.
#[must_use]
pub fn run_argv(run: &ContainerRun<'_>, agent_argv: &[String]) -> Vec<String> {
    let mut argv = vec![
        run.podman.to_owned(),
        "run".to_owned(),
        "--rm".to_owned(),
        "--workdir".to_owned(),
        CONTAINER_WORKDIR.to_owned(),
        "-v".to_owned(),
        format!("{}:{CONTAINER_WORKDIR}", run.workdir),
        "--env-file".to_owned(),
        run.env_file.to_owned(),
        "--network".to_owned(),
        run.network.to_owned(),
        "--label".to_owned(),
        "io.pii=1".to_owned(),
        "--label".to_owned(),
        format!("pii.run={}", run.run_id),
        run.image.to_owned(),
    ];
    argv.extend(agent_argv.iter().cloned());
    argv
}

/// True when `workdir` may not be mounted into a container.
///
/// Refused: a relative path, a `..` traversal, the host root, the root account's home, or a runtime directory
/// (which holds the podman and docker sockets). Fails closed.
#[must_use]
pub fn forbidden_mount(workdir: &str) -> bool {
    const FORBIDDEN: [&str; 4] = ["/", "/root", "/var/run", "/run"];
    if !workdir.starts_with('/') || workdir.split('/').any(|c| c == "..") {
        return true;
    }
    let trimmed = workdir.trim_end_matches('/');
    let path = if trimmed.is_empty() { "/" } else { trimmed };
    FORBIDDEN
        .iter()
        .any(|f| path == *f || (*f != "/" && path.starts_with(&format!("{f}/"))))
}

/// Writes `NAME=value` lines to a new file at `path` with mode 0600. The file must not already exist, and a value
/// with a newline is refused, since it would split into a second variable.
///
/// # Errors
/// [`std::io::Error`] when the file exists, cannot be created, or a value contains a newline.
pub fn write_env_file(path: &Path, pairs: &[(String, String)]) -> std::io::Result<()> {
    if pairs.iter().any(|(_, v)| v.contains('\n')) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "env value contains a newline",
        ));
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    // The file is ours from here on: a failed write must not leave the secret behind.
    let written = pairs
        .iter()
        .try_for_each(|(name, value)| writeln!(file, "{name}={value}"));
    if let Err(e) = written {
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
    Ok(())
}
