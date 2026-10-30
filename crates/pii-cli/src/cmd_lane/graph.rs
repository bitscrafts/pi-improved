//! The agent lines and the coder-lane graph the shell writes (spec 317 R5). Pure.

use super::env::LaneEnv;
use std::fmt::Write as _;

/// The agent the node runs: its name, model, provider text and the TOML lines naming them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Agent {
    pub(crate) name: String,
    pub(crate) model: String,
    pub(crate) provider: String,
    pub(crate) lines: String,
}

impl Agent {
    /// `PII_AGENT`, empty meaning claude. None for any other value.
    pub(crate) fn from_env(env: &LaneEnv) -> Option<Self> {
        match env.agent.as_deref().unwrap_or("claude") {
            "claude" => {
                let model = env
                    .claude_model
                    .clone()
                    .unwrap_or_else(|| "claude-haiku-5-5".to_owned());
                let budget = env.max_budget.as_deref().unwrap_or("3.00");
                let lines = format!(
                    "agent = \"claude\"\nmodel = \"{}\"\nmax_budget_usd = {budget}",
                    toml_str(&model)
                );
                Some(Self {
                    name: "claude".to_owned(),
                    model,
                    provider: "anthropic (the claude login file inside the devbox)".to_owned(),
                    lines,
                })
            }
            "pi" => {
                let model = env
                    .pi_model
                    .clone()
                    .unwrap_or_else(|| "deepseek-v4.1-flash:cloud".to_owned());
                let provider = env
                    .pi_provider
                    .clone()
                    .unwrap_or_else(|| "ollama-cloud".to_owned());
                let lines = format!(
                    "agent = \"pi\"\nmodel = \"{}\"\nprovider = \"{}\"",
                    toml_str(&model),
                    toml_str(&provider)
                );
                Some(Self {
                    name: "pi".to_owned(),
                    model,
                    provider,
                    lines,
                })
            }
            _ => None,
        }
    }
}

/// A TOML basic string body (the shell does not escape; spec 317 divergence 4).
pub(crate) fn toml_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if c.is_control() => {
                let _ = write!(o, "\\u{:04x}", u32::from(c));
            }
            c => o.push(c),
        }
    }
    o
}

/// The one-node coder graph (`_tmp/devbox-test/graphs/q<N>-<ts>.toml`).
pub(crate) struct Coder<'a> {
    pub(crate) name: &'a str,
    pub(crate) wall_ms: u64,
    pub(crate) agent: &'a Agent,
    pub(crate) sid: Option<&'a str>,
    pub(crate) workdir: &'a str,
    pub(crate) spec: &'a str,
    /// The brief with its trailing newlines already stripped.
    pub(crate) brief: &'a [u8],
    /// Raw text of `PII_NODE_TIMEOUT_MS` (validated decimal).
    pub(crate) timeout_ms: &'a str,
}

impl Coder<'_> {
    /// The graph's bytes, as the heredoc writes them.
    pub(crate) fn render(&self) -> Vec<u8> {
        let sid_line = self
            .sid
            .map_or_else(String::new, |s| format!("sid = \"{}\"", toml_str(s)));
        let head = format!(
            "name = \"{}\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[limits]\nmax_wall_ms = {}\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\n{}\n{sid_line}\nworkdir = \"{}\"\nspec = \"{}\"\nbrief = '''\n",
            self.name, self.wall_ms, self.agent.lines, self.workdir, self.spec
        );
        let tail = format!(
            "\n'''\nenv_pass = [\"PATH\", \"HOME\", \"IS_SANDBOX\", \"OLLAMA_API_KEY\", \"PI_CODING_AGENT_DIR\", \"CLAUDE_CODE_TMPDIR\", \"CLAUDE_CONFIG_DIR\"]\ntimeout_ms = {}\n\n[[node]]\nid = \"done\"\nkind = \"transform\"\nvalue = \"done\"\n\n[[node]]\nid = \"refused\"\nkind = \"refuse\"\nhook = \"coder\"\nreason = \"the agent failed\"\n\n[[edge]]\nfrom = \"code\"\nport = \"ok\"\nto = \"done\"\nkind = \"data\"\n\n[[edge]]\nfrom = \"code\"\nport = \"err\"\nto = \"refused\"\nkind = \"data\"\n",
            self.timeout_ms
        );
        let mut out = head.into_bytes();
        out.extend_from_slice(self.brief);
        out.extend_from_slice(tail.as_bytes());
        out
    }
}
