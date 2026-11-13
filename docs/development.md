# Development

```bash
cargo test
cargo run -- --config ./config.json --bind 127.0.0.1:8080 --ops-bind 127.0.0.1:8081
```

With a kubeconfig, Hugin runs against that cluster. `--no-kube` runs without a cluster and shows only static services.

The Lua sandbox is in `src/widgets/lua.rs`, the generator in `src/generator.rs`. Both reach the network only through the `Http` trait, which the tests replace: the sandbox tests cover the limits and the handling of credentials, and the generator tests drive the whole loop with a scripted model.
