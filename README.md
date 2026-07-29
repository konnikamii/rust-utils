## Initialize New Crate:

```bash
# create new folder in `/crates` and run
cargo init
```

Add new memeber to main `Cargo.toml`:

```txt
[workspace]
resolver = "2"
members = [
    "crates/ls",
    "crates/tr",
]

```

## Run Crate Locally:

```bash
# run crate from root dir
cargo run -p tr
cargo watch -w crates/tr -x "run -p tr"

# or cd in the crate
cargo run -- ./app -sSn
cargo watch -x run -- ./app -sSn
```

## Build Crates:

```bash
cargo build
cargo build --release   # more optimized for release
```

## Add dependencies

```bash
cargo add colored -F feature1,feature2
```

Then updated crate that requies it

```txt
# ~/crates/tr/Cargo.toml

[dependencies]
colored.workspace = true
```
