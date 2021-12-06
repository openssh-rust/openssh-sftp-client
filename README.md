# openssh-sftp-client

openssh sftp client, implements [sftp v3] accodring to
[`openssh-portable/sftp-client.c`].

[sftp v3]: https://www.openssh.com/txt/draft-ietf-secsh-filexfer-02.txt
[`openssh-portable/sftp-client.c`]: https://github.com/openssh/openssh-portable/blob/19b3d846f06697c85957ab79a63454f57f8e22d6/sftp-client.c

## How to run tests

On the first test, run the following commands:

```
cd /path/to/this/repo

# Clone all submodules
git submodule update --init --depth 1 --recursive

cd static-openssh-sftp-server
./compile-static.sh
```

These commands only need to run once.

Afterwards, you can run `cargo test` directly.
