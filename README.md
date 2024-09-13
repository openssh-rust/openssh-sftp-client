# openssh-sftp-client

[![Rust](https://github.com/openssh-rust/openssh-sftp-client/actions/workflows/rust.yml/badge.svg)](https://github.com/openssh-rust/openssh-sftp-client/actions/workflows/rust.yml)

[![crate.io downloads](https://img.shields.io/crates/d/openssh-sftp-client)](https://crates.io/crates/openssh-sftp-client)

[![crate.io version](https://img.shields.io/crates/v/openssh-sftp-client)](https://crates.io/crates/openssh-sftp-client)

[![docs](https://docs.rs/openssh-sftp-client/badge.svg)](https://docs.rs/openssh-sftp-client)

openssh-sftp-client, implements [sftp v3] according to
[`openssh-portable/sftp-client.c`] in rust using `tokio` and `serde`.

It exposes highlevel `async` APIs that models closely after `std::fs` that are
easy to use.

[sftp v3]: https://www.openssh.com/txt/draft-ietf-secsh-filexfer-02.txt
[`openssh-portable/sftp-client.c`]: https://github.com/openssh/openssh-portable/blob/19b3d846f06697c85957ab79a63454f57f8e22d6/sftp-client.c

## Extensions

This crate support the following extensions:
 - limits
 - expand path
 - fsync
 - hardlink
 - posix rename
 - copy-data

## How to run tests

For macOS, please install latest rsync from homebrew.

```
./run_tests.sh
```
