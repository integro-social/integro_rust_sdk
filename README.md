# Integro SDK — Rust

Generated Rust client for the [Integro](https://integro.social) API.

**Do not edit this repository.** Every file is machine-generated from the
Integro API definition and force-synced on every release; each sync commit
names the source revision. Pull requests cannot be accepted — every file is
replaced on the next sync — but issues are welcome here.

## Install

```toml
[dependencies]
integro_sdk = { git = "https://github.com/integro-social/integro_rust_sdk.git" }
```

## Quickstart

The default API host is `https://api.integro.social`.

```rust
use integro_sdk::routes;
use integro_sdk::runtime::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let client = Client::builder("https://api.integro.social")
    .bearer("integro_...") // API key from the dashboard
    .build();

  // Requires `ViewUsers` — every route function's doc comment carries its
  // permission contract.
  let users = routes::user::count(&client).await?;
  println!("{users} users");
  Ok(())
}
```

## Auth

Every request sends `Authorization: Bearer <token>`, where the token is an
Integro API key (`integro_...`, issued in the dashboard) or a user session
token. Set it at build time with `.bearer(...)` or later with
`client.set_bearer(Some(...))`.

Inbound event webhooks are signed with `X-Integro-Signature`
(`sha256=<hex>`, HMAC-SHA256 over the raw body with the secret shown when the
webhook is configured).

## Layout

- `routes/` — one module per domain (`message`, `post`, `group`, ...); each
  function documents the endpoint and the exact permissions it requires.
- `types/` — request/response types mirroring the server's validated newtypes;
  a request that constructs is a request the server can accept.
- `runtime` — the HTTP/SSE/WebSocket engine behind every route
  (`Client`, `ApiResult`, `WsConnection`).

## Versioning

The crate version mirrors the Integro API version at the source
revision named by the latest sync commit.
