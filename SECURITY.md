# Security

Report a vulnerability privately through
[GitHub security advisories](https://github.com/bitscrafts/pi-improved/security/advisories/new),
not as a public issue.

In scope: the `pii` binary and the crates in this repository. A model provider's service and any
model you point pii at are separate projects; report those upstream.

What the code does with credentials, so you know what to test:

- Model providers read their key from an environment variable (for example `ANTHROPIC_API_KEY`); the
  key is not a config-file field.
- A child process that pii starts does not inherit `*_API_KEY` or other secret variables unless a
  graph's `env_pass` list names them.
- `pii project init` refuses a target directory that already holds a credential and writes nothing.
- A channel such as Telegram can read its token from a file you name (`token_file`); keep that file
  out of the repository.
- Trajectory logs are written with a redaction policy only when you set `PII_TRAJECTORY_REDACT`.
  Without it, assume a log can contain whatever a run saw.
