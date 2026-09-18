# gha-indie-worker-web-server.rs

Rust web server (Axum/Maud/HTMX). Four API avenues live in `src/transport`.

`server::startup_plan` is the pure effect boundary. It validates immutable
configuration and derives non-sensitive backend capabilities before `run`
produces output; database connection strings never enter the printable plan.
