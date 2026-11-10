# pii-channels

`pii-channels`: inbound polling + outbound reply, no sockets of its own (the webhook channel's listener is bound by the daemon through `NetService`; this crate only holds its request handler).

Public modules: `cli`, `discord`, `matrix`, `mqtt`, `observe`, `poll`, `post`, `secret`, `telegram`, `webhook`.

Depends on: `pii-abi`, `pii-core`, `pii-observe`.
Used by: `pii-server`, `pii-tools`.

15 source files; 6 integration test files under `tests/`. Test with `cargo test -p pii-channels`.
