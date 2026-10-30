//! The real `Devbox` (spec 319 R1): the two probes of spec 317, sent through the transport. Each string is the one
//! the shell sends (`scripts/run-batchC-pii.sh` lines 68, 103, 211-216).
use super::Devbox;
use super::transport::Transport;

pub(crate) struct SshDevbox<'a> {
    pub(crate) t: &'a dyn Transport,
    pub(crate) container: &'a str,
    /// The claude config dir inside the container (`$CFG`).
    pub(crate) cfg: &'a str,
}

impl Devbox for SshDevbox<'_> {
    /// `test -n "$(find $CFG -name "<sid>.jsonl" -type f -print -quit)"`: found is exit 0.
    fn resume_exists(&self, sid: &str) -> bool {
        let remote = format!(
            "podman exec {} sh -c 'test -n \"$(find {} -name \"{sid}.jsonl\" -type f -print -quit 2>/dev/null)\"'",
            self.container, self.cfg
        );
        self.t.exec(&remote).code == 0
    }

    /// `command -v uv >/dev/null` inside the container. The error is the probe's output, stderr merged.
    fn has_uv(&self) -> Result<(), String> {
        let remote = format!(
            "podman exec {} sh -c 'command -v uv >/dev/null'",
            self.container
        );
        let out = self.t.exec(&remote);
        if out.code == 0 {
            return Ok(());
        }
        let text = [out.stdout, out.stderr].concat();
        Err(String::from_utf8_lossy(&text)
            .trim_end_matches('\n')
            .to_owned())
    }
}
