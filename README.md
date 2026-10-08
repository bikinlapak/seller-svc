# seller-svc

Backend for the bikinlapak seller dashboard ([seller-app](../seller-app)). Products and the store's web builder are managed in worksh.io. This service is a thin layer that calls the works-server GraphQL API on behalf of the logged-in seller.

## Running

```sh
cp .env.example .env
cargo run                 # http://127.0.0.1:8080/healthz
cargo test
cargo clippy --all-targets
```

## Endpoints

| Method | Path                | Status                      |
| ------ | ------------------- | --------------------------- |
| GET    | `/healthz`          | liveness check              |
| GET    | `/v1/products`      | 501, not implemented yet    |
| GET    | `/v1/builder/pages` | 501, not implemented yet    |
