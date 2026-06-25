# ferrvm-guest

A freestanding x86_64 kernel that boots directly under ferrvm and prints a message over serrial.

Build with:

```sh
cargo build
```

Run with:

```sh
cargo run -- --kernel ./guest/target/x86_64-guest/debug/guest
```

You will see `** Hello World from the guest! **` on the console. 
Pressing `Ctrl-A x` will stop everything.

## References

1. https://os.phil-opp.com/freestanding-rust-binary/
2. https://os.phil-opp.com/minimal-rust-kernel/
