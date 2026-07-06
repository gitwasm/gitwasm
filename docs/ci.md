# CI verification

Use CI to verify the committed `.gitwasm/` state independently of developer
machines.

```yaml
name: gitwasm

on:
  pull_request:
  push:
    branches: [main]

jobs:
  verify:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-wasip1, wasm32-unknown-unknown
      - uses: Swatinem/rust-cache@v2
      - run: cargo build --release -p gitwasm
      - run: ./target/release/gitwasm verify
      - run: ./target/release/gitwasm list
      - run: ./demo/run-demo.sh
      - run: cd demo/playground && ../../target/release/gitwasm audit
```

`gitwasm verify` checks signatures when `.gitwasm/signatures.toml` exists.
`gitwasm audit` re-derives local verdicts in the demo playground.
Replay is a local cache optimization; audit is the proof operation.
